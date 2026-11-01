//! Distinct values of a column as a compact `u8` codebook.
//!
//! A [`ValueDictionary`] gives every source row a code naming its distinct
//! value: codes `0..=254` name the non-null values in value order, and
//! [`NULL_CODE`] names null. Everything about how a physical Arrow array
//! becomes that codebook lives here: native dictionaries reuse their keys and
//! values, primitive arrays scan natively, and only text, binary and nested
//! arrays fall back to Arrow's row format. Other modules consume the result
//! through the methods below; they never see codes, values or encoding
//! strategy.
//!
//! The codebook is a pure function of the column data and is cached by
//! [`crate::Column`]. Columns with more than [`MAX_DISTINCT_VALUES`] non-null
//! values are not encoded; callers fall back to streaming counts, which this
//! module also provides through [`counted`].

use std::{
    cmp::Reverse,
    collections::{hash_map::DefaultHasher, BinaryHeap, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray, UInt32Array},
    compute::take,
    datatypes::{
        ArrowPrimitiveType, DataType, Date32Type, Date64Type, Decimal128Type, Decimal256Type,
        Decimal32Type, Decimal64Type, DurationMicrosecondType, DurationMillisecondType,
        DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type,
        Int16Type, Int32Type, Int64Type, Int8Type, Time32MillisecondType, Time32SecondType,
        Time64MicrosecondType, Time64NanosecondType, TimeUnit, TimestampMicrosecondType,
        TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt16Type,
        UInt32Type, UInt64Type, UInt8Type,
    },
    row::{RowConverter, SortField},
};

use crate::{
    analysis::AnalysisContext,
    column::{format, value},
    predicate::Predicate,
    scope::Selection,
    selection::RowSelection,
    Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Stable for the lifetime of the document; pass it to [`crate::Filter::InValues`].
    /// Codes `0..=254` name non-null values and [`NULL_CODE`] names null.
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
    /// Whether the keys name values for [`crate::Filter::InValues`]. False for
    /// columns with too many distinct values to encode; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`; [`NULL_CODE`] is reserved for null, so one
/// `u8` code space covers 255 values plus null. Columns with more values are
/// counted without an encoding and use streaming counts/searches.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

/// Code reserved for null. Non-null values use codes `0..=254`.
pub(crate) const NULL_CODE: u8 = 255;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

pub(crate) struct ValueDictionary {
    /// Code of each source row; [`NULL_CODE`] for null.
    codes: Vec<u8>,
    /// Non-null values in code order; index `code` formats that value.
    values: ArrayRef,
    /// Whether the source column contains at least one null.
    has_null: bool,
}

impl ValueDictionary {
    /// Encode `array`, or `None` for unsupported types and columns with more
    /// than [`MAX_DISTINCT_VALUES`] non-null values.
    pub(crate) fn build(array: &ArrayRef) -> Option<Self> {
        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::build_native(array),
            Boolean => build_boolean(array),
            Null => build_null(array),
            Int8 => build_keyed(array.as_primitive::<Int8Type>()),
            Int16 => build_keyed(array.as_primitive::<Int16Type>()),
            Int32 => build_keyed(array.as_primitive::<Int32Type>()),
            Int64 => build_keyed(array.as_primitive::<Int64Type>()),
            UInt8 => build_keyed(array.as_primitive::<UInt8Type>()),
            UInt16 => build_keyed(array.as_primitive::<UInt16Type>()),
            UInt32 => build_keyed(array.as_primitive::<UInt32Type>()),
            UInt64 => build_keyed(array.as_primitive::<UInt64Type>()),
            Float16 => build_keyed(array.as_primitive::<Float16Type>()),
            Float32 => build_keyed(array.as_primitive::<Float32Type>()),
            Float64 => build_keyed(array.as_primitive::<Float64Type>()),
            Date32 => build_keyed(array.as_primitive::<Date32Type>()),
            Date64 => build_keyed(array.as_primitive::<Date64Type>()),
            Time32(TimeUnit::Second) => build_keyed(array.as_primitive::<Time32SecondType>()),
            Time32(TimeUnit::Millisecond) => {
                build_keyed(array.as_primitive::<Time32MillisecondType>())
            }
            Time64(TimeUnit::Microsecond) => {
                build_keyed(array.as_primitive::<Time64MicrosecondType>())
            }
            Time64(TimeUnit::Nanosecond) => {
                build_keyed(array.as_primitive::<Time64NanosecondType>())
            }
            Timestamp(TimeUnit::Second, _) => {
                build_keyed(array.as_primitive::<TimestampSecondType>())
            }
            Timestamp(TimeUnit::Millisecond, _) => {
                build_keyed(array.as_primitive::<TimestampMillisecondType>())
            }
            Timestamp(TimeUnit::Microsecond, _) => {
                build_keyed(array.as_primitive::<TimestampMicrosecondType>())
            }
            Timestamp(TimeUnit::Nanosecond, _) => {
                build_keyed(array.as_primitive::<TimestampNanosecondType>())
            }
            Duration(TimeUnit::Second) => build_keyed(array.as_primitive::<DurationSecondType>()),
            Duration(TimeUnit::Millisecond) => {
                build_keyed(array.as_primitive::<DurationMillisecondType>())
            }
            Duration(TimeUnit::Microsecond) => {
                build_keyed(array.as_primitive::<DurationMicrosecondType>())
            }
            Duration(TimeUnit::Nanosecond) => {
                build_keyed(array.as_primitive::<DurationNanosecondType>())
            }
            Decimal32(..) => build_keyed(array.as_primitive::<Decimal32Type>()),
            Decimal64(..) => build_keyed(array.as_primitive::<Decimal64Type>()),
            Decimal128(..) => build_keyed(array.as_primitive::<Decimal128Type>()),
            Decimal256(..) => build_keyed(array.as_primitive::<Decimal256Type>()),
            _ => Self::build_encoded(array),
        }
    }

    /// Generic encoder: discover distinct values by hashing rows, then write
    /// one `u8` code per row. Row-format bytes give both equality and value
    /// order.
    fn build_encoded(array: &ArrayRef) -> Option<Self> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())]).ok()?;

        // Temporary codes in order of first appearance, and each value's first
        // source row. Row-format bytes sort like the values themselves.
        let mut seen: HashMap<Box<[u8]>, u8> = HashMap::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(array.len());
        let mut has_null = false;
        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter
                .convert_columns(std::slice::from_ref(&array.slice(start, len)))
                .ok()?;
            for (i, row) in rows.iter().enumerate() {
                let source_row = start + i;
                if array.is_null(source_row) {
                    has_null = true;
                    codes.push(NULL_CODE);
                    continue;
                }
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&code) => code,
                    None => {
                        if first_row.len() == MAX_DISTINCT_VALUES {
                            return None;
                        }
                        let code = first_row.len() as u8;
                        seen.insert(bytes.into(), code);
                        first_row.push(source_row);
                        code
                    }
                };
                codes.push(code);
            }
        }

        // Renumber codes into value order; null keeps its reserved code.
        let mut key_of: Vec<&[u8]> = vec![&[]; first_row.len()];
        for (bytes, &code) in &seen {
            key_of[code as usize] = bytes;
        }
        let mut order: Vec<u8> = (0..first_row.len() as u8).collect();
        order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(key_of[b as usize]));
        let mut remap = [0u8; MAX_DISTINCT_VALUES];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u8;
        }
        for code in codes.iter_mut().filter(|code| **code != NULL_CODE) {
            *code = remap[*code as usize];
        }

        Some(Self {
            codes,
            values: value_rows(array.as_ref(), &first_row, &order)?,
            has_null,
        })
    }

    /// Encode a native Dictionary. Arrow already encodes the values into
    /// `keys`/`values`, so no row hashing is needed: collect the referenced
    /// dictionary values, reuse [`Self::build`] on that small array, then
    /// walk the keys once to write the shared per-row code array.
    fn build_native(array: &ArrayRef) -> Option<Self> {
        let dictionary = array.as_any_dictionary();
        let keys = dictionary.keys();
        let dictionary_values = dictionary.values().clone();
        // A dictionary with more entries than the code space cannot be compact.
        if dictionary_values.len() > MAX_DISTINCT_VALUES + 1 {
            return None;
        }

        // Which dictionary entries are referenced, and are there logical nulls
        // (a null key, or a key pointing at a null value)?
        let value_nulls = dictionary_values.logical_nulls();
        let mut has_null = false;
        let mut referenced: Vec<u32> = Vec::new();
        let mut entry_slot = vec![u32::MAX; dictionary_values.len()];
        if !scan_keys(keys, |key| {
            let Some(entry) = key else {
                has_null = true;
                return;
            };
            if value_nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(entry))
            {
                has_null = true;
                return;
            }
            let Some(slot) = entry_slot.get_mut(entry) else {
                return;
            };
            if *slot == u32::MAX {
                *slot = referenced.len() as u32;
                referenced.push(entry as u32);
            }
        }) {
            return None;
        }

        // Encode the referenced values with the same path as a plain column.
        let referenced_values = take(
            dictionary_values.as_ref(),
            &UInt32Array::from(referenced),
            None,
        )
        .ok()?;
        let encoded = Self::build(&referenced_values)?;

        let mut key_to_code = vec![NULL_CODE; dictionary_values.len()];
        for (entry, &slot) in entry_slot.iter().enumerate() {
            if slot != u32::MAX {
                key_to_code[entry] = encoded.codes[slot as usize];
            }
        }

        // One key walk writes the shared per-row code array.
        let mut codes = Vec::with_capacity(array.len());
        if !scan_keys(keys, |key| {
            codes.push(match key {
                None => NULL_CODE,
                Some(entry) => key_to_code.get(entry).copied().unwrap_or(NULL_CODE),
            });
        }) {
            return None;
        }

        Some(Self {
            codes,
            values: encoded.values,
            has_null,
        })
    }

    #[cfg(test)]
    fn n_values(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    fn has_null(&self) -> bool {
        self.has_null
    }

    #[cfg(test)]
    fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Each distinct value with its count over `rows`, in value order with
    /// null first, leaving out values with no rows.
    pub(crate) fn counts(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        self.counts_with_zero(rows, false)
    }

    /// Every source-column value in value order with null first, including
    /// those absent from `rows`. Stable options remain selectable as other
    /// filters change.
    pub(crate) fn counts_all(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        self.counts_with_zero(rows, true)
    }

    fn counts_with_zero(&self, rows: &Selection, include_zero: bool) -> Result<Vec<DistinctValue>> {
        let mut counts = [0u64; 256];
        match rows {
            Selection::All(_) => self
                .codes
                .iter()
                .for_each(|&code| counts[code as usize] += 1),
            Selection::Rows(rows) => rows
                .iter()
                .for_each(|&row| counts[self.codes[row as usize] as usize] += 1),
        }

        let formatter = format::formatter(self.values.as_ref())?;
        let mut values = Vec::with_capacity(self.values.len() + 1);
        if self.has_null && (include_zero || counts[NULL_CODE as usize] > 0) {
            values.push(DistinctValue {
                key: u64::from(NULL_CODE),
                label: None,
                count: counts[NULL_CODE as usize],
            });
        }
        for (code, &count) in counts.iter().enumerate().take(self.values.len()) {
            if include_zero || count > 0 {
                values.push(DistinctValue {
                    key: code as u64,
                    label: format::cell(self.values.as_ref(), &formatter, code),
                    count,
                });
            }
        }
        Ok(values)
    }

    /// Resolve matching keys for a list of displayed labels (`None` = null).
    pub(crate) fn keys_for_labels(&self, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let target: HashSet<&Option<String>> = labels.iter().collect();
        let mut keys = Vec::new();
        if self.has_null && labels.iter().any(Option::is_none) {
            keys.push(u64::from(NULL_CODE));
        }
        let formatter = format::formatter(self.values.as_ref())?;
        for code in 0..self.values.len() {
            let label = format::cell(self.values.as_ref(), &formatter, code);
            if target.contains(&label) {
                keys.push(code as u64);
            }
        }
        Ok(keys)
    }

    fn fill_matches(
        &self,
        rows: RowSelection<'_>,
        incoming: Option<&[bool]>,
        output: &mut [bool],
        allowed: &[bool; 256],
    ) {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize];
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize] & mask[i];
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize];
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize] & mask[i];
                }
            }
        }
    }

    fn predicate_from_allowed(self: &Arc<Self>, allowed: [bool; 256]) -> Predicate<'static> {
        let dictionary = Arc::clone(self);
        Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
            dictionary.fill_matches(rows, incoming, output, &allowed);
            Ok(())
        }))
    }

    /// Prepare a row-based predicate for filtering by key codes.
    pub(crate) fn prepare_predicate(self: &Arc<Self>, keys: &[u64]) -> Predicate<'static> {
        let mut allowed = [false; 256];
        for &key in keys {
            if let Ok(key) = usize::try_from(key) {
                if let Some(slot) = allowed.get_mut(key) {
                    *slot = true;
                }
            }
        }
        self.predicate_from_allowed(allowed)
    }

    /// Prepare a row-based predicate for filtering by text values (`None` for null).
    pub(crate) fn prepare_text_predicate(
        self: &Arc<Self>,
        values: &[Option<String>],
        exclude: bool,
    ) -> Result<Predicate<'static>> {
        let keys = self.keys_for_labels(values)?;
        let mut allowed = [exclude; 256];
        for key in keys {
            if let Some(slot) = allowed.get_mut(key as usize) {
                *slot = !exclude;
            }
        }
        Ok(self.predicate_from_allowed(allowed))
    }
}

/// How a primitive value becomes a codebook key.
///
/// Contract: equal keys must mean "the same displayed value", and key order
/// must equal value order, because codes are assigned in key order.
trait PrimitiveKey: ArrowPrimitiveType {
    type Key: Copy + Eq + Ord + Hash;

    fn key(value: Self::Native) -> Self::Key;

    /// Whether this value should replace the recorded representative of its
    /// key. Floats prefer positive zero so the option reads `0.0`.
    fn prefers_representative(_: Self::Native) -> bool {
        false
    }
}

macro_rules! identity_keys {
    ($($array:ty => $native:ty),* $(,)?) => {$(
        impl PrimitiveKey for $array {
            type Key = $native;

            fn key(value: $native) -> $native {
                value
            }
        }
    )*};
}

identity_keys!(
    Int8Type => i8,
    Int16Type => i16,
    Int32Type => i32,
    Int64Type => i64,
    UInt8Type => u8,
    UInt16Type => u16,
    UInt32Type => u32,
    UInt64Type => u64,
    Date32Type => i32,
    Date64Type => i64,
    Time32SecondType => i32,
    Time32MillisecondType => i32,
    Time64MicrosecondType => i64,
    Time64NanosecondType => i64,
    TimestampSecondType => i64,
    TimestampMillisecondType => i64,
    TimestampMicrosecondType => i64,
    TimestampNanosecondType => i64,
    DurationSecondType => i64,
    DurationMillisecondType => i64,
    DurationMicrosecondType => i64,
    DurationNanosecondType => i64,
    Decimal32Type => i32,
    Decimal64Type => i64,
    Decimal128Type => i128,
    Decimal256Type => arrow::datatypes::i256,
);

impl PrimitiveKey for Float16Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        value::order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float32Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        value::order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float64Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        value::order_key(value)
    }

    fn prefers_representative(value: Self::Native) -> bool {
        value == 0.0 && value.is_sign_positive()
    }
}

/// Encode a primitive array by scanning native values.
fn build_keyed<T: PrimitiveKey>(array: &PrimitiveArray<T>) -> Option<ValueDictionary> {
    build_distinct(
        array.len(),
        |row| array.is_null(row),
        |row| T::key(array.value(row)),
        |row| T::prefers_representative(array.value(row)),
        array,
    )
}

/// Encode a boolean array. `false` sorts before `true`.
fn build_boolean(array: &ArrayRef) -> Option<ValueDictionary> {
    let source = array.as_boolean();
    build_distinct(
        source.len(),
        |row| source.is_null(row),
        |row| u8::from(source.value(row)),
        |_| false,
        source,
    )
}

/// Encode the Arrow null type: every row is null.
fn build_null(array: &ArrayRef) -> Option<ValueDictionary> {
    Some(ValueDictionary {
        codes: vec![NULL_CODE; array.len()],
        values: value_rows(array.as_ref(), &[], &[])?,
        has_null: !array.is_empty(),
    })
}

/// Shared codebook algorithm: one native pass, one code per distinct key, then
/// codes renumbered into key order.
fn build_distinct<K: Copy + Eq + Ord + Hash>(
    len: usize,
    is_null: impl Fn(usize) -> bool,
    key_at: impl Fn(usize) -> K,
    prefer_at: impl Fn(usize) -> bool,
    array: &dyn Array,
) -> Option<ValueDictionary> {
    let mut seen: HashMap<K, u8> = HashMap::new();
    let mut key_of: Vec<K> = Vec::new();
    let mut first_row: Vec<usize> = Vec::new();
    let mut codes: Vec<u8> = Vec::with_capacity(len);
    let mut has_null = false;
    for row in 0..len {
        if is_null(row) {
            has_null = true;
            codes.push(NULL_CODE);
            continue;
        }
        let key = key_at(row);
        let code = match seen.get(&key) {
            Some(&code) => code,
            None => {
                if key_of.len() == MAX_DISTINCT_VALUES {
                    return None;
                }
                let code = key_of.len() as u8;
                seen.insert(key, code);
                key_of.push(key);
                first_row.push(row);
                code
            }
        };
        if prefer_at(row) {
            first_row[code as usize] = row;
        }
        codes.push(code);
    }
    let mut order: Vec<u8> = (0..key_of.len() as u8).collect();
    order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(&key_of[b as usize]));
    let mut remap = [0u8; MAX_DISTINCT_VALUES];
    for (new, &old) in order.iter().enumerate() {
        remap[old as usize] = new as u8;
    }
    for code in codes.iter_mut().filter(|code| **code != NULL_CODE) {
        *code = remap[*code as usize];
    }
    Some(ValueDictionary {
        codes,
        values: value_rows(array, &first_row, &order)?,
        has_null,
    })
}

/// Build the `values` array in code order from the first row of each value.
fn value_rows(array: &dyn Array, first_row: &[usize], order: &[u8]) -> Option<ArrayRef> {
    let rows = UInt32Array::from(
        order
            .iter()
            .map(|&old| first_row[old as usize] as u32)
            .collect::<Vec<_>>(),
    );
    take(array, &rows, None).ok()
}

/// Visit every key of a native Dictionary array; `None` for a null key.
/// Returns false if the key type is not an Arrow dictionary index type.
fn scan_keys(keys: &dyn Array, mut f: impl FnMut(Option<usize>)) -> bool {
    macro_rules! visit {
        ($array:expr) => {{
            for row in 0..$array.len() {
                f(if $array.is_null(row) {
                    None
                } else {
                    Some($array.value(row) as usize)
                });
            }
        }};
    }
    match keys.data_type() {
        DataType::Int8 => visit!(keys.as_primitive::<Int8Type>()),
        DataType::Int16 => visit!(keys.as_primitive::<Int16Type>()),
        DataType::Int32 => visit!(keys.as_primitive::<Int32Type>()),
        DataType::Int64 => visit!(keys.as_primitive::<Int64Type>()),
        DataType::UInt8 => visit!(keys.as_primitive::<UInt8Type>()),
        DataType::UInt16 => visit!(keys.as_primitive::<UInt16Type>()),
        DataType::UInt32 => visit!(keys.as_primitive::<UInt32Type>()),
        DataType::UInt64 => visit!(keys.as_primitive::<UInt64Type>()),
        _ => return false,
    }
    true
}

/// Distinct values counted once, without keeping an index.
pub(crate) struct ValueCounts {
    /// Distinct values among the rows, null included.
    pub distinct: u64,
    /// The `keep` most common values (ties: earliest first), in value
    /// order. Their keys only number them (0, 1, ...): there is no index to
    /// name them, so they cannot be used in a filter.
    pub top: Vec<DistinctValue>,
}

/// Count the distinct values of `array` over `rows`, keeping the `keep`
/// most common. Values are told apart by a 64-bit hash, so memory is a few
/// words per distinct value; a collision (unlikely below billions of
/// values) would merge two values. Floats use exact normalized bit keys instead:
/// signed zeros form one group, all NaNs another, and null remains separate.
pub(crate) fn count_values(array: &ArrayRef, rows: &Selection, keep: usize) -> Result<ValueCounts> {
    count_values_with_context(array, rows, keep, &AnalysisContext::default())
}

pub(crate) fn count_values_with_context(
    array: &ArrayRef,
    rows: &Selection,
    keep: usize,
    context: &AnalysisContext,
) -> Result<ValueCounts> {
    context.check()?;
    let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
    let counts = match array.data_type() {
        DataType::Float16 => count_floats(array.as_primitive::<Float16Type>(), rows, context)?,
        DataType::Float32 => count_floats(array.as_primitive::<Float32Type>(), rows, context)?,
        DataType::Float64 => count_floats(array.as_primitive::<Float64Type>(), rows, context)?,
        _ => {
            // hash -> (count, first row)
            let mut counts: HashMap<u64, (u64, u32)> = HashMap::new();
            let mut tally =
                |chunk: &ArrayRef, source: &mut dyn Iterator<Item = usize>| -> Result<()> {
                    context.check()?;
                    let converted = converter.convert_columns(std::slice::from_ref(chunk))?;
                    for (i, (row, r)) in converted.iter().zip(source).enumerate() {
                        context.check_at(i)?;
                        let mut h = DefaultHasher::new();
                        row.as_ref().hash(&mut h);
                        counts.entry(h.finish()).or_insert((0, r as u32)).0 += 1;
                    }
                    Ok(())
                };
            match rows {
                Selection::All(n) => {
                    for start in (0..*n).step_by(CHUNK) {
                        let len = CHUNK.min(n - start);
                        tally(&array.slice(start, len), &mut (start..start + len))?;
                    }
                }
                Selection::Rows(rows) => {
                    for chunk in rows.chunks(CHUNK) {
                        context.check()?;
                        let taken = take(array.as_ref(), &UInt32Array::from(chunk.to_vec()), None)?;
                        tally(&taken, &mut chunk.iter().map(|&r| r as usize))?;
                    }
                }
            }
            counts
        }
    };

    let distinct = counts.len() as u64;
    // Keep only the requested candidates, not a second collection of every
    // distinct value. Higher counts and earlier source rows win ties.
    let mut candidates = BinaryHeap::with_capacity(keep.min(counts.len()));
    for (i, (count, row)) in counts.into_values().enumerate() {
        context.check_at(i)?;
        let candidate = Reverse((count, Reverse(row)));
        if candidates.len() < keep {
            candidates.push(candidate);
        } else if let Some(mut smallest) = candidates.peek_mut() {
            if candidate.0 > smallest.0 {
                *smallest = candidate;
            }
        }
    }
    let top: Vec<(u64, u32)> = candidates
        .into_iter()
        .map(|Reverse((count, Reverse(row)))| (count, row))
        .collect();

    // Back into value order.
    let first_rows = UInt32Array::from(top.iter().map(|&(_, r)| r).collect::<Vec<_>>());
    let taken = take(array.as_ref(), &first_rows, None)?;
    let converted = converter.convert_columns(&[taken])?;
    let mut order: Vec<usize> = (0..top.len()).collect();
    order.sort_by(|&a, &b| converted.row(a).cmp(&converted.row(b)));

    let f = format::formatter(array.as_ref())?;
    let nulls = array.logical_nulls();
    let top = order
        .into_iter()
        .enumerate()
        .map(|(key, i)| DistinctValue {
            key: key as u64,
            label: if nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(top[i].1 as usize))
            {
                None
            } else {
                format::cell(array.as_ref(), &f, top[i].1 as usize)
            },
            count: top[i].0,
        })
        .collect();
    context.check()?;
    Ok(ValueCounts { distinct, top })
}

/// Float equality for analysis groups signed zero and NaN payloads. Native
/// scalar reads avoid a row-format allocation or widened copy of the column.
fn count_floats<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    context: &AnalysisContext,
) -> Result<HashMap<u64, (u64, u32)>>
where
    T::Native: Into<f64>,
{
    const NAN_KEY: u64 = f64::NAN.to_bits();
    // All source NaNs become NAN_KEY, leaving this payload exclusively for null.
    const NULL_KEY: u64 = NAN_KEY | 1;
    let mut counts = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        let key = if array.is_null(row) {
            NULL_KEY
        } else {
            let value: f64 = array.value(row).into();
            value::normalized_bits(value)
        };
        counts.entry(key).or_insert((0, row as u32)).0 += 1;
    }
    context.check()?;
    Ok(counts)
}

/// Distinct values with counts over `rows`: the codebook when the column has
/// one, streaming counts otherwise. `keep` bounds the streaming path's list; a
/// codebook never holds more than [`MAX_DISTINCT_VALUES`] values itself.
pub(crate) fn counted(
    dictionary: Option<&Arc<ValueDictionary>>,
    array: &ArrayRef,
    rows: &Selection,
    keep: usize,
) -> Result<ValueCounts> {
    match dictionary {
        Some(dictionary) => {
            let values = dictionary.counts(rows)?;
            Ok(ValueCounts {
                distinct: values.len() as u64,
                top: values,
            })
        }
        None => count_values(array, rows, keep),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            BooleanArray, Date32Array, Decimal128Array, Decimal256Array, DictionaryArray,
            DurationSecondArray, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
            NullArray, StringArray, Time32MillisecondArray, TimestampMicrosecondArray,
        },
        compute::cast,
        datatypes::{i256, Int16Type, Int32Type, Int8Type},
    };

    use super::*;
    use crate::{column::format, engine::CancelToken, scope::Selection};

    fn labels(dictionary: &ValueDictionary) -> Vec<Option<String>> {
        let formatter = format::formatter(dictionary.values.as_ref()).unwrap();
        (0..dictionary.values.len())
            .map(|code| format::cell(dictionary.values.as_ref(), &formatter, code))
            .collect()
    }

    #[test]
    fn float_counts_group_signed_zero_and_nan_payloads_separately_from_null() {
        let source = Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(f64::from_bits(0xfff8_2000_0000_0000)),
            None,
            None,
            Some(1.0),
        ]);
        for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
            let array = arrow::compute::cast(&source, &data_type).unwrap();
            for rows in [
                Selection::All(array.len()),
                Selection::Rows(Arc::new(vec![7, 6, 5, 4, 3, 2, 1, 0])),
            ] {
                let result = count_values(&array, &rows, 8).unwrap();
                assert_eq!(result.distinct, 4);
                assert_eq!(result.top.len(), 4);
                for value in result.top {
                    let expected = match value.label {
                        None => 2,
                        Some(label) => match label.parse::<f64>().unwrap() {
                            value if value.is_nan() => 3,
                            0.0 => 2,
                            1.0 => 1,
                            value => panic!("unexpected value {value}"),
                        },
                    };
                    assert_eq!(value.count, expected);
                }
            }
        }
    }

    #[test]
    fn codes_follow_value_order_and_reserve_the_null_code() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("dog"),
            None,
            Some("cat"),
            Some("dog"),
        ]));
        let dictionary = ValueDictionary::build(&a).unwrap();
        assert_eq!(dictionary.n_values(), 2);
        assert!(dictionary.has_null());
        assert_eq!(labels(&dictionary), vec![
            Some("cat".into()),
            Some("dog".into())
        ]);
        assert_eq!(
            (0..4)
                .map(|row| dictionary.code_at(row))
                .collect::<Vec<_>>(),
            [1, NULL_CODE, 0, 1]
        );
    }

    #[test]
    fn numbers_sort_numerically() {
        let a: ArrayRef = Arc::new(Int32Array::from(vec![10, -3, 2, 10]));
        let dictionary = ValueDictionary::build(&a).unwrap();
        assert_eq!(labels(&dictionary), vec![
            Some("-3".into()),
            Some("2".into()),
            Some("10".into())
        ]);
        assert_eq!(dictionary.n_values(), 3);
    }

    #[test]
    fn too_many_values_are_not_encoded() {
        let at_limit: ArrayRef =
            Arc::new(Int32Array::from_iter_values(0..MAX_DISTINCT_VALUES as i32));
        assert!(ValueDictionary::build(&at_limit).is_some());
        let over: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
        assert!(ValueDictionary::build(&over).is_none());
    }

    #[test]
    fn nulls_do_not_consume_value_codes() {
        let array: ArrayRef = Arc::new(Int32Array::from_iter(
            (0..MAX_DISTINCT_VALUES as i32).map(Some).chain([None]),
        ));
        let dictionary = ValueDictionary::build(&array).unwrap();
        assert_eq!(dictionary.n_values(), MAX_DISTINCT_VALUES);
        assert!(dictionary.has_null());
    }

    #[test]
    fn all_counts_keep_zero_count_options_and_stable_keys() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
        ]));
        let dictionary = ValueDictionary::build(&a).unwrap();
        let selection = Selection::Rows(Arc::new(vec![0, 3]));
        let all = dictionary.counts_all(&selection).unwrap();
        assert_eq!(all, vec![
            DistinctValue {
                key: u64::from(NULL_CODE),
                label: None,
                count: 0
            },
            DistinctValue {
                key: 0,
                label: Some("a".into()),
                count: 0
            },
            DistinctValue {
                key: 1,
                label: Some("b".into()),
                count: 2
            },
        ]);
        assert_eq!(dictionary.counts(&selection).unwrap(), vec![all[2].clone()]);
        let empty = dictionary
            .counts_all(&Selection::Rows(Arc::new(vec![])))
            .unwrap();
        assert_eq!(empty.iter().map(|value| value.key).collect::<Vec<_>>(), [
            u64::from(NULL_CODE),
            0,
            1
        ]);
        assert!(empty.iter().all(|value| value.count == 0));
        assert_eq!(
            dictionary.counts_all(&Selection::All(a.len())).unwrap(),
            dictionary.counts(&Selection::All(a.len())).unwrap()
        );
    }

    #[test]
    fn label_lookup_maps_null_to_the_reserved_key() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), Some("a"), None]));
        let dictionary = ValueDictionary::build(&a).unwrap();
        assert_eq!(
            dictionary
                .keys_for_labels(&[Some("a".into()), None])
                .unwrap(),
            vec![u64::from(NULL_CODE), 0]
        );
        assert!(dictionary
            .keys_for_labels(&[Some("missing".into())])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn empty_and_all_null_columns_still_encode() {
        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        let dictionary = ValueDictionary::build(&empty).unwrap();
        assert_eq!(dictionary.n_values(), 0);
        assert!(!dictionary.has_null());

        let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
        let dictionary = ValueDictionary::build(&nulls).unwrap();
        assert_eq!(dictionary.n_values(), 0);
        assert!(dictionary.has_null());
        assert_eq!(dictionary.code_at(0), NULL_CODE);
        assert_eq!(dictionary.counts(&Selection::All(2)).unwrap(), vec![
            DistinctValue {
                key: u64::from(NULL_CODE),
                label: None,
                count: 2
            }
        ]);
    }

    #[test]
    fn predicates_match_null_through_the_reserved_code() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), None, Some("a")]));
        let dictionary = Arc::new(ValueDictionary::build(&a).unwrap());
        let mut hits = [false; 3];
        dictionary
            .prepare_predicate(&[u64::from(NULL_CODE)])
            .evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [false, true, false]);

        let mut hits = [false; 3];
        dictionary
            .prepare_text_predicate(&[None], false)
            .unwrap()
            .evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [false, true, false]);
    }

    #[test]
    fn native_dictionary_reuses_keys_and_unifies_both_nulls() {
        let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(2), Some(2), Some(0)]);
        let values = StringArray::from(vec![Some("b"), None, Some("a")]);
        let array: ArrayRef =
            Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
        let dictionary = ValueDictionary::build(&array).unwrap();
        assert_eq!(dictionary.n_values(), 2);
        assert!(dictionary.has_null());
        assert_eq!(labels(&dictionary), vec![
            Some("a".into()),
            Some("b".into())
        ]);
        assert_eq!(
            (0..6)
                .map(|row| dictionary.code_at(row))
                .collect::<Vec<_>>(),
            [1, NULL_CODE, NULL_CODE, 0, 0, 1]
        );
        assert_eq!(dictionary.counts(&Selection::All(6)).unwrap(), vec![
            DistinctValue {
                key: u64::from(NULL_CODE),
                label: None,
                count: 2
            },
            DistinctValue {
                key: 0,
                label: Some("a".into()),
                count: 2
            },
            DistinctValue {
                key: 1,
                label: Some("b".into()),
                count: 2
            },
        ]);
    }

    #[test]
    fn native_dictionary_drops_duplicate_and_unused_entries() {
        let keys = Int8Array::from(vec![Some(0), Some(1), Some(2)]);
        let values = StringArray::from(vec![Some("b"), Some("a"), Some("a"), Some("z")]);
        let array: ArrayRef =
            Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
        let dictionary = ValueDictionary::build(&array).unwrap();
        assert_eq!(dictionary.n_values(), 2);
        assert!(!dictionary.has_null());
        assert_eq!(labels(&dictionary), vec![
            Some("a".into()),
            Some("b".into())
        ]);
        assert_eq!(
            (0..3)
                .map(|row| dictionary.code_at(row))
                .collect::<Vec<_>>(),
            [1, 0, 0]
        );
    }

    #[test]
    fn native_dictionary_over_the_limit_falls_back() {
        let values: ArrayRef =
            Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
        let keys = Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32);
        let array: ArrayRef =
            Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());
        assert!(ValueDictionary::build(&array).is_none());
    }

    #[test]
    fn native_dictionary_value_width_does_not_matter() {
        let keys = Int16Array::from(vec![Some(0), Some(1), Some(0)]);
        let values = Int16Array::from(vec![Some(1), Some(2)]);
        let array: ArrayRef =
            Arc::new(DictionaryArray::<Int16Type>::try_new(keys, Arc::new(values)).unwrap());
        let dictionary = ValueDictionary::build(&array).unwrap();
        assert_eq!(dictionary.n_values(), 2);
        assert_eq!(
            (0..3)
                .map(|row| dictionary.code_at(row))
                .collect::<Vec<_>>(),
            [0, 1, 0]
        );
    }

    #[test]
    fn predicates_cover_ranges_indices_and_incoming_masks() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            None,
            Some("a"),
            Some("b"),
        ]));
        let dictionary = Arc::new(ValueDictionary::build(&a).unwrap());
        let keys = dictionary.keys_for_labels(&[Some("b".into())]).unwrap();
        let predicate = dictionary.prepare_predicate(&keys);
        let cancel = CancelToken::never();

        let mut range_hits = [false; 4];
        predicate.evaluate(0..4, &mut range_hits, &cancel).unwrap();
        assert_eq!(range_hits, [true, false, false, true]);

        // Rows selected out of order by index.
        let rows = [3usize, 1, 2, 0];
        let mut index_hits = [false; 4];
        predicate
            .evaluate(rows.as_slice(), &mut index_hits, &cancel)
            .unwrap();
        assert_eq!(index_hits, [true, false, false, true]);

        // Incoming mask intersects existing matches.
        let mut matched = [true; 4];
        let mut scratch = [false; 4];
        predicate
            .intersect(0..4, &mut matched, &mut scratch, &cancel)
            .unwrap();
        assert_eq!(matched, [true, false, false, true]);
    }

    #[test]
    fn plain_and_native_dictionaries_agree() {
        let plain: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            None,
            Some("a"),
            Some("b"),
            Some("a"),
        ]));
        let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(0), Some(1)]);
        let values = StringArray::from(vec![Some("b"), Some("a")]);
        let native: ArrayRef =
            Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
        let plain = ValueDictionary::build(&plain).unwrap();
        let native = ValueDictionary::build(&native).unwrap();
        assert_eq!(plain.n_values(), native.n_values());
        assert_eq!(
            plain.counts_all(&Selection::All(5)).unwrap(),
            native.counts_all(&Selection::All(5)).unwrap()
        );
        for labels in [
            vec![Some("a".into())],
            vec![Some("b".into())],
            vec![None],
            vec![Some("a".into()), None],
        ] {
            assert_eq!(
                plain.keys_for_labels(&labels).unwrap(),
                native.keys_for_labels(&labels).unwrap()
            );
        }
    }

    #[test]
    fn float_values_group_signed_zero_and_nan_payloads() {
        let source = Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(f64::from_bits(0xfff8_2000_0000_0000)),
            None,
            Some(1.0),
        ]);
        for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
            let array = arrow::compute::cast(&source, &data_type).unwrap();
            let dictionary = ValueDictionary::build(&array).unwrap();
            assert_eq!(dictionary.n_values(), 3, "{data_type:?}");
            assert!(dictionary.has_null(), "{data_type:?}");

            let labels = labels(&dictionary);
            let parsed: Vec<Option<f64>> = labels
                .iter()
                .map(|label| label.as_deref().map(|text| text.parse::<f64>().unwrap()))
                .collect();
            assert_eq!(parsed[0], Some(0.0), "{data_type:?}");
            assert_eq!(parsed[1], Some(1.0), "{data_type:?}");
            assert!(parsed[2].unwrap().is_nan(), "{data_type:?}");

            let counts = dictionary.counts(&Selection::All(array.len())).unwrap();
            assert_eq!(counts.len(), 4, "{data_type:?}");
            assert_eq!(counts[0].label, None, "{data_type:?}");
            assert_eq!(counts[0].count, 1, "{data_type:?}");
            assert_eq!(counts[1].count, 2, "{data_type:?}");
            assert_eq!(counts[2].count, 1, "{data_type:?}");
            assert_eq!(counts[3].label.as_deref(), Some("NaN"), "{data_type:?}");
            assert_eq!(counts[3].count, 3, "{data_type:?}");

            // The streaming fallback must group the same way.
            let counted = count_values(&array, &Selection::All(array.len()), 10).unwrap();
            assert_eq!(counted.distinct as usize, counts.len(), "{data_type:?}");
        }
    }

    #[test]
    fn counting_without_an_index() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
            Some("c"),
            Some("b"),
            Some("a"),
        ]));
        let all = count_values(&a, &Selection::All(a.len()), 2).unwrap();
        assert_eq!(all.distinct, 4);
        let top: Vec<_> = all
            .top
            .iter()
            .map(|v| (v.label.clone().unwrap(), v.count))
            .collect();
        assert_eq!(top, [("a".to_string(), 2), ("b".to_string(), 3)]);

        let some = count_values(&a, &Selection::Rows(Arc::new(vec![2, 4])), 10).unwrap();
        assert_eq!(some.distinct, 2);
        assert_eq!(some.top[0].label, None);
    }

    fn codes_of(dictionary: &ValueDictionary) -> Vec<u8> {
        (0..dictionary.codes.len())
            .map(|row| dictionary.code_at(row))
            .collect()
    }

    #[test]
    fn integer_arrays_group_and_order_natively() {
        let signed: ArrayRef = Arc::new(Int64Array::from(vec![
            Some(-3),
            Some(7),
            Some(-3),
            None,
            Some(2),
        ]));
        for data_type in [
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
        ] {
            let array = cast(&signed, &data_type).expect("integer cast");
            let dictionary = ValueDictionary::build(&array).expect("encoded");
            assert_eq!(
                labels(&dictionary),
                [Some("-3".into()), Some("2".into()), Some("7".into())],
                "{data_type}"
            );
            assert_eq!(codes_of(&dictionary), [0, 2, 0, 255, 1], "{data_type}");
            assert!(dictionary.has_null());
        }

        let unsigned: ArrayRef = Arc::new(arrow::array::UInt64Array::from(vec![
            Some(3),
            Some(7),
            Some(3),
            None,
            Some(2),
        ]));
        for data_type in [
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
        ] {
            let array = cast(&unsigned, &data_type).expect("unsigned cast");
            let dictionary = ValueDictionary::build(&array).expect("encoded");
            assert_eq!(
                labels(&dictionary),
                [Some("2".into()), Some("3".into()), Some("7".into())],
                "{data_type}"
            );
            assert_eq!(codes_of(&dictionary), [1, 2, 1, 255, 0], "{data_type}");
        }
    }

    #[test]
    fn temporal_arrays_order_by_stored_unit() {
        let cases: [(ArrayRef, DataType); 4] = [
            (
                Arc::new(Date32Array::from(vec![
                    Some(3),
                    Some(1),
                    Some(3),
                    None,
                    Some(2),
                ])),
                DataType::Date32,
            ),
            (
                Arc::new(Time32MillisecondArray::from(vec![
                    Some(3_000),
                    Some(1_000),
                    Some(3_000),
                    None,
                    Some(2_000),
                ])),
                DataType::Time32(TimeUnit::Millisecond),
            ),
            (
                Arc::new(TimestampMicrosecondArray::from(vec![
                    Some(3_000_000),
                    Some(1_000_000),
                    Some(3_000_000),
                    None,
                    Some(2_000_000),
                ])),
                DataType::Timestamp(TimeUnit::Microsecond, None),
            ),
            (
                Arc::new(DurationSecondArray::from(vec![
                    Some(3),
                    Some(1),
                    Some(3),
                    None,
                    Some(2),
                ])),
                DataType::Duration(TimeUnit::Second),
            ),
        ];
        for (array, data_type) in cases {
            assert_eq!(array.data_type(), &data_type);
            let dictionary = ValueDictionary::build(&array).expect("encoded");
            assert_eq!(dictionary.n_values(), 3, "{data_type}");
            assert_eq!(codes_of(&dictionary), [2, 0, 2, 255, 1], "{data_type}");
        }
    }

    #[test]
    fn decimal_arrays_order_by_coefficient() {
        let d128: ArrayRef = Arc::new(
            Decimal128Array::from(vec![Some(123_i128), Some(-45), Some(123), None, Some(0)])
                .with_precision_and_scale(10, 2)
                .unwrap(),
        );
        let dictionary = ValueDictionary::build(&d128).expect("encoded");
        assert_eq!(labels(&dictionary), [
            Some("-0.45".into()),
            Some("0.00".into()),
            Some("1.23".into())
        ]);
        assert_eq!(codes_of(&dictionary), [2, 0, 2, 255, 1]);

        let d256: ArrayRef = Arc::new(
            Decimal256Array::from(vec![
                Some(i256::from_i128(5)),
                Some(i256::from_i128(-2)),
                None,
                Some(i256::from_i128(5)),
            ])
            .with_precision_and_scale(76, 0)
            .unwrap(),
        );
        let dictionary = ValueDictionary::build(&d256).expect("encoded");
        assert_eq!(dictionary.n_values(), 2);
        assert_eq!(codes_of(&dictionary), [1, 0, 255, 1]);
    }

    #[test]
    fn boolean_arrays_order_false_before_true() {
        let array: ArrayRef = Arc::new(BooleanArray::from(vec![
            Some(true),
            Some(false),
            Some(true),
            None,
        ]));
        let dictionary = ValueDictionary::build(&array).expect("encoded");
        assert_eq!(labels(&dictionary), [
            Some("false".into()),
            Some("true".into())
        ]);
        assert_eq!(codes_of(&dictionary), [1, 0, 1, 255]);
    }

    #[test]
    fn arrow_null_type_encodes_every_row_as_null() {
        let array: ArrayRef = Arc::new(NullArray::new(3));
        let dictionary = ValueDictionary::build(&array).expect("encoded");
        assert_eq!(dictionary.n_values(), 0);
        assert!(dictionary.has_null());
        assert_eq!(codes_of(&dictionary), [255, 255, 255]);
        let present = dictionary.counts(&Selection::All(3)).unwrap();
        assert_eq!(present, [DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 3
        }]);
    }
}
