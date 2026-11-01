//! Nested Find delegates to the value/element/field's logical type using
//! borrowed parent and child selections, without decoding Arrow value payload.

use std::{ops::Range, sync::OnceLock};

use arrow::{
    array::{downcast_dictionary_array, Array, ArrayRef, AsArray, OffsetSizeTrait},
    datatypes::{ArrowNativeType, DataType},
};

use super::Column;
use crate::{
    predicate::{Predicate, CHUNK},
    Result,
};

pub(super) fn search<'a>(
    array: &'a ArrayRef,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    match array.data_type() {
        DataType::Dictionary(..) => dictionary_find(array, |values| {
            Column::new(values).prepare_find(query, matcher)
        }),
        DataType::List(_) => {
            let list = array.as_list::<i32>();
            list_search(array, list.values(), list.value_offsets(), query, matcher)
        }
        DataType::LargeList(_) => {
            let list = array.as_list::<i64>();
            list_search(array, list.values(), list.value_offsets(), query, matcher)
        }
        DataType::FixedSizeList(..) => {
            let list = array.as_fixed_size_list();
            let Some(child) = Column::new(list.values()).prepare_find(query, matcher)? else {
                return Ok(None);
            };
            Ok(Some(elements(array, child, move |row| {
                let start = list.value_offset_at(row);
                start..start + list.value_length() as usize
            })))
        }
        DataType::ListView(_) => {
            let list = array.as_list_view::<i32>();
            let Some(child) = Column::new(list.values()).prepare_find(query, matcher)? else {
                return Ok(None);
            };
            Ok(Some(elements(array, child, move |row| {
                let start = list.value_offsets()[row].as_usize();
                start..start + list.value_sizes()[row].as_usize()
            })))
        }
        DataType::LargeListView(_) => {
            let list = array.as_list_view::<i64>();
            let Some(child) = Column::new(list.values()).prepare_find(query, matcher)? else {
                return Ok(None);
            };
            Ok(Some(elements(array, child, move |row| {
                let start = list.value_offsets()[row].as_usize();
                start..start + list.value_sizes()[row].as_usize()
            })))
        }
        DataType::Struct(_) => {
            let children = array
                .as_struct()
                .columns()
                .iter()
                .map(|child| Column::new(child).prepare_find(query, matcher))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if children.is_empty() {
                return Ok(None);
            }
            Ok(Some(Predicate::new(Box::new(
                move |rows, incoming, hits, cancel| {
                    hits.fill(false);
                    let mut scratch = vec![false; rows.len()];
                    for child in &children {
                        child.evaluate(rows.clone(), &mut scratch, cancel)?;
                        for (hit, other) in hits.iter_mut().zip(&scratch) {
                            *hit |= *other;
                        }
                    }
                    for (index, row) in rows.iter().enumerate() {
                        hits[index] &= array.is_valid(row);
                    }
                    if let Some(mask) = incoming {
                        for (hit, &m) in hits.iter_mut().zip(mask) {
                            *hit &= m;
                        }
                    }
                    Ok(())
                },
            ))))
        }
        _ => unreachable!("nested type dispatched by Column"),
    }
}

/// Find follows only dictionary keys referenced by the selected parent rows.
/// This applies recursively, including dictionaries nested inside lists. No
/// dictionary-sized mask or decoded values column is built.
pub(super) fn dictionary_find<'a>(
    array: &'a ArrayRef,
    prepare: impl FnOnce(&'a ArrayRef) -> Result<Option<Predicate<'a>>>,
) -> Result<Option<Predicate<'a>>> {
    downcast_dictionary_array!(array => {
        let values = array.values();
        let Some(child) = prepare(values)? else { return Ok(None) };
        Ok(Some(Predicate::new(Box::new(move |rows, incoming, hits, cancel| {
            hits.fill(false);
            let mut keys = Vec::with_capacity(rows.len());
            let mut output_rows = Vec::with_capacity(rows.len());
            for (index, row) in rows.iter().enumerate() {
                if let Some(mask) = incoming {
                    if !mask[index] {
                        continue;
                    }
                }
                if array.is_valid(row) {
                    keys.push(array.keys().value(row).as_usize());
                    output_rows.push(index);
                }
            }
            let mut matched = vec![false; keys.len()];
            child.evaluate(&keys, &mut matched, cancel)?;
            for (key, output) in output_rows.into_iter().enumerate() {
                hits[output] = matched[key];
            }
            Ok(())
        }))))
    }, _ => unreachable!("dictionary type checked by caller"))
}

/// Explicit filters retain the established eager dictionary-value cache. This
/// avoids changing their low-cardinality execution policy as part of Find.
pub(super) fn dictionary<'a>(
    array: &'a ArrayRef,
    prepare: impl FnOnce(&'a ArrayRef) -> Result<Option<Predicate<'a>>>,
) -> Result<Option<Predicate<'a>>> {
    downcast_dictionary_array!(array => {
        let values = array.values();
        let Some(child) = prepare(values)? else { return Ok(None) };
        let cached = OnceLock::<Vec<bool>>::new();
        Ok(Some(Predicate::new(Box::new(move |rows, incoming, hits, cancel| {
            let matched = match cached.get() {
                Some(matched) => matched,
                None => {
                    let mut matched = vec![false; values.len()];
                    child.evaluate(0..values.len(), &mut matched, cancel)?;
                    cached.get_or_init(|| matched)
                }
            };
            match incoming {
                Some(mask) => {
                    for (index, row) in rows.iter().enumerate() {
                        hits[index] = mask[index]
                            && array.is_valid(row)
                            && matched[array.keys().value(row).as_usize()];
                    }
                }
                None => {
                    for (index, row) in rows.iter().enumerate() {
                        hits[index] = array.is_valid(row) && matched[array.keys().value(row).as_usize()];
                    }
                }
            }
            Ok(())
        }))))
    }, _ => unreachable!("dictionary type checked by caller"))
}

fn list_search<'a, O: OffsetSizeTrait>(
    array: &'a ArrayRef,
    values: &'a ArrayRef,
    offsets: &'a [O],
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    let Some(child) = Column::new(values).prepare_find(query, matcher)? else {
        return Ok(None);
    };
    Ok(Some(elements(array, child, move |row| {
        offsets[row].as_usize()..offsets[row + 1].as_usize()
    })))
}

/// Evaluate selected list children in bounded groups. Groups may span multiple
/// parent rows or one enormous list, but never contain the whole child column.
/// The child predicate and all scratch buffers are reused across parent cells.
fn elements<'a>(
    array: &'a ArrayRef,
    child: Predicate<'a>,
    offsets: impl Fn(usize) -> Range<usize> + Send + Sync + 'a,
) -> Predicate<'a> {
    Predicate::new(Box::new(move |rows, incoming, hits, cancel| {
        hits.fill(false);
        // Size scratch to this batch rather than to CHUNK: the closure is reused
        // for every 1,024-row batch, so preallocating a full CHUNK per call would
        // churn megabytes per scan. Large spans still grow these vectors as needed.
        let hint = rows.len().min(CHUNK);
        let mut children = Vec::with_capacity(hint);
        let mut owners: Vec<(usize, usize, usize)> = Vec::with_capacity(hint);
        let mut matched = Vec::with_capacity(hint);
        let flush = |children: &mut Vec<usize>,
                     owners: &mut Vec<(usize, usize, usize)>,
                     hits: &mut [bool],
                     matched: &mut Vec<bool>|
         -> Result<()> {
            if children.is_empty() {
                return Ok(());
            }
            matched.resize(children.len(), false);
            child.evaluate(children.as_slice(), matched, cancel)?;
            for &(output, start, end) in owners.iter() {
                if !hits[output] && matched[start..end].contains(&true) {
                    hits[output] = true;
                }
            }
            children.clear();
            owners.clear();
            Ok(())
        };
        for (output, row) in rows.iter().enumerate() {
            cancel.check()?;
            if let Some(mask) = incoming {
                if !mask[output] {
                    continue;
                }
            }
            if hits[output] || array.is_null(row) {
                continue;
            }
            let span = offsets(row);
            let mut next = span.start;
            while next < span.end && !hits[output] {
                if children.len() == CHUNK {
                    flush(&mut children, &mut owners, hits, &mut matched)?;
                }
                let end = (next + (CHUNK - children.len())).min(span.end);
                let start = children.len();
                children.extend(next..end);
                owners.push((output, start, children.len()));
                next = end;
                if children.len() == CHUNK {
                    flush(&mut children, &mut owners, hits, &mut matched)?;
                }
            }
        }
        flush(&mut children, &mut owners, hits, &mut matched)
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use arrow::{
        array::{DictionaryArray, Int32Array, ListArray, StringArray},
        buffer::OffsetBuffer,
        datatypes::{DataType, Field, Int32Type},
    };

    use super::*;
    use crate::engine::CancelToken;

    fn dictionary(keys: Int32Array) -> ArrayRef {
        let values: ArrayRef = Arc::new(StringArray::from_iter_values(
            (0..10_000).map(|row| row.to_string()),
        ));
        Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap())
    }

    #[test]
    fn find_dictionary_evaluates_only_referenced_keys() {
        let array = dictionary(Int32Array::from(vec![
            Some(9_999),
            Some(2),
            None,
            Some(9_999),
        ]));
        let selected = Mutex::new(Vec::new());
        let child = Predicate::new(Box::new(|rows, _incoming, hits, _| {
            selected.lock().unwrap().extend(rows.iter());
            for (index, row) in rows.iter().enumerate() {
                hits[index] = row == 2;
            }
            Ok(())
        }));
        let predicate = dictionary_find(&array, |_| Ok(Some(child)))
            .unwrap()
            .unwrap();
        let rows = [3, 1, 2];
        let mut hits = [false; 3];
        predicate
            .evaluate(&rows[..], &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(*selected.lock().unwrap(), [9_999, 2]);
        assert_eq!(hits, [false, true, false]);
    }

    #[test]
    fn dictionary_nested_in_list_also_evaluates_only_referenced_keys() {
        let dictionary = dictionary(Int32Array::from(vec![Some(9_999), Some(2)]));
        let list: ArrayRef = Arc::new(ListArray::new(
            Arc::new(Field::new(
                "item",
                DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
                true,
            )),
            OffsetBuffer::new(vec![0_i32, 2].into()),
            dictionary.clone(),
            None,
        ));
        let selected = Mutex::new(Vec::new());
        let child = Predicate::new(Box::new(|rows, _incoming, hits, _| {
            selected.lock().unwrap().extend(rows.iter());
            for (index, row) in rows.iter().enumerate() {
                hits[index] = row == 2;
            }
            Ok(())
        }));
        let child = dictionary_find(&dictionary, |_| Ok(Some(child)))
            .unwrap()
            .unwrap();
        let predicate = elements(&list, child, |_| 0..2);
        let mut hits = [false];
        predicate
            .evaluate(0..1, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(*selected.lock().unwrap(), [9_999, 2]);
        assert_eq!(hits, [true]);
    }

    #[test]
    fn enormous_list_uses_bounded_child_selections_without_payload_take() {
        let len = CHUNK * 2 + 17;
        let values: ArrayRef = Arc::new(StringArray::from_iter_values((0..len).map(|_| "x")));
        let list: ArrayRef = Arc::new(ListArray::new(
            Arc::new(Field::new("item", DataType::Utf8, true)),
            OffsetBuffer::new(vec![0_i32, i32::try_from(len).unwrap()].into()),
            values,
            None,
        ));
        let calls = Mutex::new(Vec::new());
        let child = Predicate::new(Box::new(|rows, _incoming, _hits, _| {
            let len = rows.len();
            let first = rows.iter().next().unwrap();
            calls.lock().unwrap().push((first, len));
            Ok(())
        }));
        let predicate = elements(&list, child, move |_| 0..len);
        predicate
            .evaluate(0..1, &mut [false], &CancelToken::never())
            .unwrap();
        assert_eq!(*calls.lock().unwrap(), [
            (0, CHUNK),
            (CHUNK, CHUNK),
            (CHUNK * 2, 17)
        ]);
    }
}
