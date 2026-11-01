//! Core distinct-value extraction and codebook assembly kernels.
//!
//! Provides [`build_distinct`] for single-pass value deduplication and
//! [`finish_dictionary`] for remapping temporary discovery codes into canonical
//! sorted value order.

use std::{collections::HashMap, hash::Hash};

use arrow::{
    array::{Array, ArrayRef, UInt32Array},
    compute::take,
};

use super::{ValueDictionary, MAX_DISTINCT_VALUES, NULL_CODE};

/// Shared single-pass distinct extraction: one temporary code per unique key,
/// then codes renumbered into canonical key order.
pub(super) fn build_distinct<K: Copy + Eq + Ord + Hash>(
    len: usize,
    is_null: impl Fn(usize) -> bool,
    key_at: impl Fn(usize) -> K,
    prefer_at: impl Fn(usize) -> bool,
    array: &dyn Array,
) -> Option<ValueDictionary> {
    let mut seen: HashMap<K, u8> = HashMap::new();
    let mut key_of: Vec<K> = Vec::new();
    let mut representative_row: Vec<usize> = Vec::new();
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
                representative_row.push(row);
                code
            }
        };
        if prefer_at(row) {
            representative_row[code as usize] = row;
        }
        codes.push(code);
    }

    let mut order: Vec<u8> = (0..key_of.len() as u8).collect();
    order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(&key_of[b as usize]));
    finish_dictionary(codes, &representative_row, &order, has_null, array)
}

/// Renumber temporary codes into value order and assemble the finished dictionary.
pub(super) fn finish_dictionary(
    mut codes: Vec<u8>,
    representative_row: &[usize],
    order: &[u8],
    has_null: bool,
    array: &dyn Array,
) -> Option<ValueDictionary> {
    let mut remap = [0u8; MAX_DISTINCT_VALUES];
    for (new, &old) in order.iter().enumerate() {
        remap[old as usize] = new as u8;
    }
    for code in codes.iter_mut().filter(|code| **code != NULL_CODE) {
        *code = remap[*code as usize];
    }
    Some(ValueDictionary {
        codes,
        values: value_rows(array, representative_row, order)?,
        has_null,
    })
}

/// Build the dictionary's `values` array in code order from each value's
/// representative row.
fn value_rows(array: &dyn Array, representative_row: &[usize], order: &[u8]) -> Option<ArrayRef> {
    let rows = UInt32Array::from(
        order
            .iter()
            .map(|&old| representative_row[old as usize] as u32)
            .collect::<Vec<_>>(),
    );
    take(array, &rows, None).ok()
}
