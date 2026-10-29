//! Per-column index of distinct values.
//!
//! Every row gets a code (`u32`) naming its distinct value. Codes are ordered
//! like the values themselves (nulls first), so code order is display order.
//! The Excel-style filter keeps a set of codes; counting values is a single
//! pass over the codes.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef},
    row::{RowConverter, SortField},
};

use crate::{format, Result};

pub(crate) struct ValueIndex {
    /// Code of each source row.
    pub codes: Vec<u32>,
    /// Display label of each code, `None` for null.
    pub labels: Vec<Option<String>>,
}

impl ValueIndex {
    pub fn build(array: &ArrayRef) -> Result<Self> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
        let rows = converter.convert_columns(std::slice::from_ref(array))?;

        // First pass: assign codes in order of first appearance.
        let mut seen: HashMap<Box<[u8]>, u32> = HashMap::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut codes = Vec::with_capacity(array.len());
        for i in 0..array.len() {
            let bytes = rows.row(i);
            let bytes: &[u8] = bytes.as_ref();
            let code = match seen.get(bytes) {
                Some(&c) => c,
                None => {
                    let c = first_row.len() as u32;
                    seen.insert(bytes.into(), c);
                    first_row.push(i);
                    c
                }
            };
            codes.push(code);
        }

        // Row-format bytes sort like the values (nulls first), so sorting the
        // unique byte strings gives value order.
        let mut order: Vec<u32> = (0..first_row.len() as u32).collect();
        order.sort_unstable_by(|&a, &b| {
            rows.row(first_row[a as usize])
                .cmp(&rows.row(first_row[b as usize]))
        });
        let mut remap = vec![0u32; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u32;
        }
        for c in codes.iter_mut() {
            *c = remap[*c as usize];
        }

        let f = format::formatter(array.as_ref())?;
        let labels = order
            .iter()
            .map(|&old| format::cell(array.as_ref(), &f, first_row[old as usize]))
            .collect();

        Ok(Self { codes, labels })
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};

    use super::*;

    #[test]
    fn codes_follow_value_order() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("dog"),
            None,
            Some("cat"),
            Some("dog"),
        ]));
        let idx = ValueIndex::build(&a).unwrap();
        assert_eq!(
            idx.labels,
            vec![None, Some("cat".into()), Some("dog".into())]
        );
        assert_eq!(idx.codes, vec![2, 0, 1, 2]);
    }

    #[test]
    fn numbers_sort_numerically() {
        let a: ArrayRef = Arc::new(Int32Array::from(vec![10, -3, 2, 10]));
        let idx = ValueIndex::build(&a).unwrap();
        assert_eq!(
            idx.labels,
            vec![Some("-3".into()), Some("2".into()), Some("10".into())]
        );
    }
}
