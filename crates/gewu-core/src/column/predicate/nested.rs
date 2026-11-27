//! Nested Find compiles child predicates using
//! borrowed parent and child selections, without decoding Arrow value payload.

use std::ops::Range;

use arrow::{
    array::{downcast_dictionary_array, Array, ArrayRef, AsArray, OffsetSizeTrait},
    datatypes::{ArrowNativeType, DataType},
};

use super::{compile_find, Result};
use crate::execution::predicate::{Predicate, CHUNK};

pub(super) fn search<'a>(
    array: &'a ArrayRef,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    match array.data_type() {
        DataType::Dictionary(..) => {
            dictionary_find(array, |values| compile_find(values, query, matcher))
        }
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
            let Some(child) = compile_find(list.values(), query, matcher)? else {
                return Ok(None);
            };
            Ok(Some(elements(array, child, move |row| {
                let start = list.value_offset_at(row);
                start..start + list.value_length() as usize
            })))
        }
        DataType::ListView(_) => {
            let list = array.as_list_view::<i32>();
            let Some(child) = compile_find(list.values(), query, matcher)? else {
                return Ok(None);
            };
            Ok(Some(elements(array, child, move |row| {
                let start = list.value_offsets()[row].as_usize();
                start..start + list.value_sizes()[row].as_usize()
            })))
        }
        DataType::LargeListView(_) => {
            let list = array.as_list_view::<i64>();
            let Some(child) = compile_find(list.values(), query, matcher)? else {
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
                .map(|child| compile_find(child, query, matcher))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if children.is_empty() {
                return Ok(None);
            }
            Ok(Some(Predicate::new(Box::new(move |rows, hits| {
                hits.fill(false);
                let mut scratch = vec![false; rows.len()];
                for child in &children {
                    child.evaluate(rows.clone(), &mut scratch);
                    for (hit, other) in hits.iter_mut().zip(&scratch) {
                        *hit |= *other;
                    }
                }
                for (index, row) in rows.iter().enumerate() {
                    hits[index] &= array.is_valid(row);
                }
            }))))
        }
        _ => unreachable!("nested type dispatched by Column"),
    }
}

/// Find follows only dictionary keys referenced by the selected parent rows.
/// This applies recursively, including dictionaries nested inside lists. No
/// dictionary-sized mask or decoded values column is built.
pub(super) fn dictionary_find<'a>(
    array: &'a ArrayRef,
    compile: impl FnOnce(&'a ArrayRef) -> Result<Option<Predicate<'a>>>,
) -> Result<Option<Predicate<'a>>> {
    downcast_dictionary_array!(array => {
        let values = array.values();
        let Some(child) = compile(values)? else { return Ok(None) };
        Ok(Some(Predicate::new(Box::new(move |rows, hits| {
            hits.fill(false);
            let mut keys = Vec::with_capacity(rows.len());
            let mut output_rows = Vec::with_capacity(rows.len());
            for (index, row) in rows.iter().enumerate() {
                if array.is_valid(row) {
                    keys.push(array.keys().value(row).as_usize());
                    output_rows.push(index);
                }
            }
            let mut matched = vec![false; keys.len()];
            child.evaluate(&keys, &mut matched);
            for (key, output) in output_rows.into_iter().enumerate() {
                hits[output] = matched[key];
            }
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
    let Some(child) = compile_find(values, query, matcher)? else {
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
    Predicate::new(Box::new(move |rows, hits| {
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
                     matched: &mut Vec<bool>| {
            if children.is_empty() {
                return;
            }
            matched.resize(children.len(), false);
            child.evaluate(children.as_slice(), matched);
            for &(output, start, end) in owners.iter() {
                if !hits[output] && matched[start..end].contains(&true) {
                    hits[output] = true;
                }
            }
            children.clear();
            owners.clear();
        };
        for (output, row) in rows.iter().enumerate() {
            if hits[output] || array.is_null(row) {
                continue;
            }
            let span = offsets(row);
            let mut next = span.start;
            while next < span.end && !hits[output] {
                if children.len() == CHUNK {
                    flush(&mut children, &mut owners, hits, &mut matched);
                }
                let end = (next + (CHUNK - children.len())).min(span.end);
                let start = children.len();
                children.extend(next..end);
                owners.push((output, start, children.len()));
                next = end;
                if children.len() == CHUNK {
                    flush(&mut children, &mut owners, hits, &mut matched);
                }
            }
        }
        flush(&mut children, &mut owners, hits, &mut matched);
    }))
}

#[cfg(test)]
#[path = "nested_tests.rs"]
mod tests;
