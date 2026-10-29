//! Nested search delegates to the value/element/field's logical type.

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

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Result<Option<Predicate<'a>>> {
    match array.data_type() {
        DataType::Dictionary(..) => {
            dictionary(array, |values| Column::new(values).prepare_search(text))
        }
        DataType::List(_) => {
            let list = array.as_list::<i32>();
            list_search(array, list.values(), list.value_offsets(), text)
        }
        DataType::LargeList(_) => {
            let list = array.as_list::<i64>();
            list_search(array, list.values(), list.value_offsets(), text)
        }
        DataType::FixedSizeList(..) => {
            let list = array.as_fixed_size_list();
            let Some(child) = Column::new(list.values()).prepare_search(text)? else {
                return Ok(None);
            };
            Ok(Some(elements(
                array,
                child,
                move |row| {
                    let start = list.value_offset_at(row);
                    start..start + list.value_length() as usize
                },
                true,
            )))
        }
        DataType::ListView(_) => {
            let list = array.as_list_view::<i32>();
            let Some(child) = Column::new(list.values()).prepare_search(text)? else {
                return Ok(None);
            };
            Ok(Some(elements(
                array,
                child,
                move |row| {
                    let start = list.value_offsets()[row].as_usize();
                    start..start + list.value_sizes()[row].as_usize()
                },
                false,
            )))
        }
        DataType::LargeListView(_) => {
            let list = array.as_list_view::<i64>();
            let Some(child) = Column::new(list.values()).prepare_search(text)? else {
                return Ok(None);
            };
            Ok(Some(elements(
                array,
                child,
                move |row| {
                    let start = list.value_offsets()[row].as_usize();
                    start..start + list.value_sizes()[row].as_usize()
                },
                false,
            )))
        }
        DataType::Struct(_) => {
            let children = array
                .as_struct()
                .columns()
                .iter()
                .map(|child| Column::new(child).prepare_search(text))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if children.is_empty() {
                return Ok(None);
            }
            Ok(Some(Predicate::batch(Box::new(
                move |rows, hits, cancel| {
                    let mut scratch = vec![false; rows.len()];
                    for child in &children {
                        child.evaluate(rows.clone(), &mut scratch, cancel)?;
                        for (hit, other) in hits.iter_mut().zip(&scratch) {
                            *hit |= *other;
                        }
                    }
                    for (row, hit) in rows.zip(hits) {
                        *hit &= array.is_valid(row);
                    }
                    Ok(())
                },
            ))))
        }
        _ => unreachable!("nested type dispatched by Column"),
    }
}

/// Dictionary values are evaluated once per prepared query, on first execution.
/// No normalized-key copy or decoded column is allocated. An interrupted scan
/// never enters the cache; keys retain their source offsets across batches.
pub(super) fn dictionary<'a>(
    array: &'a ArrayRef,
    prepare: impl FnOnce(&'a ArrayRef) -> Result<Option<Predicate<'a>>>,
) -> Result<Option<Predicate<'a>>> {
    downcast_dictionary_array!(array => {
        let values = array.values();
        let Some(child) = prepare(values)? else { return Ok(None) };
        let cached = OnceLock::<Vec<bool>>::new();
        Ok(Some(Predicate::batch(Box::new(move |rows, hits, cancel| {
            let matched = match cached.get() {
                Some(matched) => matched,
                None => {
                    let mut matched = vec![false; values.len()];
                    child.evaluate(0..values.len(), &mut matched, cancel)?;
                    cached.get_or_init(|| matched)
                }
            };
            for (row, hit) in rows.zip(hits) {
                *hit = array.is_valid(row) && matched[array.keys().value(row).as_usize()];
            }
            Ok(())
        }))))
    }, _ => unreachable!("dictionary type checked by caller"))
}

fn list_search<'a, O: OffsetSizeTrait>(
    array: &'a ArrayRef,
    values: &'a ArrayRef,
    offsets: &'a [O],
    text: &str,
) -> Result<Option<Predicate<'a>>> {
    let Some(child) = Column::new(values).prepare_search(text)? else {
        return Ok(None);
    };
    Ok(Some(elements(
        array,
        child,
        move |row| offsets[row].as_usize()..offsets[row + 1].as_usize(),
        true,
    )))
}

fn elements<'a>(
    array: &'a ArrayRef,
    child: Predicate<'a>,
    offsets: impl Fn(usize) -> Range<usize> + Send + Sync + 'a,
    contiguous: bool,
) -> Predicate<'a> {
    Predicate::batch(Box::new(move |rows, hits, cancel| {
        if rows.is_empty() {
            return Ok(());
        }
        // Ordinary lists have contiguous child ranges. Batch the children too,
        // including a single enormous list, without allocating a child-sized mask.
        if contiguous {
            let children = offsets(rows.start).start..offsets(rows.end - 1).end;
            let mut scratch = vec![false; children.len().min(CHUNK)];
            let mut first = rows.start;
            for start in (children.start..children.end).step_by(CHUNK) {
                let end = (start + CHUNK).min(children.end);
                let matched = &mut scratch[..end - start];
                child.evaluate(start..end, matched, cancel)?;
                while first < rows.end && offsets(first).end <= start {
                    first += 1;
                }
                for row in first..rows.end {
                    let span = offsets(row);
                    if span.start >= end {
                        break;
                    }
                    if array.is_valid(row) && !hits[row - rows.start] {
                        hits[row - rows.start] = matched
                            [span.start.max(start) - start..span.end.min(end) - start]
                            .contains(&true);
                    }
                }
            }
        } else {
            // ListView ranges may overlap or appear out of order.
            let mut scratch = Vec::new();
            for (row, hit) in rows.zip(hits) {
                cancel.check()?;
                if array.is_null(row) {
                    continue;
                }
                let span = offsets(row);
                scratch.resize(span.len().min(CHUNK), false);
                for start in (span.start..span.end).step_by(CHUNK) {
                    let end = (start + CHUNK).min(span.end);
                    child.evaluate(start..end, &mut scratch[..end - start], cancel)?;
                    if scratch[..end - start].contains(&true) {
                        *hit = true;
                        break;
                    }
                }
            }
        }
        Ok(())
    }))
}
