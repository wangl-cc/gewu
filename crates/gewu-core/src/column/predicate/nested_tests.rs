use std::sync::{Arc, Mutex};

use arrow::{
    array::{DictionaryArray, Int32Array, ListArray, StringArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Int32Type},
};

use super::*;

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
    let child = Predicate::new(Box::new(|rows, hits| {
        selected.lock().unwrap().extend(rows.iter());
        for (index, row) in rows.iter().enumerate() {
            hits[index] = row == 2;
        }
    }));
    let predicate = dictionary_find(&array, |_| Ok(Some(child)))
        .unwrap()
        .unwrap();
    let rows = [3, 1, 2];
    let mut hits = [false; 3];
    predicate.evaluate(&rows[..], &mut hits);
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
    let child = Predicate::new(Box::new(|rows, hits| {
        selected.lock().unwrap().extend(rows.iter());
        for (index, row) in rows.iter().enumerate() {
            hits[index] = row == 2;
        }
    }));
    let child = dictionary_find(&dictionary, |_| Ok(Some(child)))
        .unwrap()
        .unwrap();
    let predicate = elements(&list, child, |_| 0..2);
    let mut hits = [false];
    predicate.evaluate(0..1, &mut hits);
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
    let child = Predicate::new(Box::new(|rows, _hits| {
        let len = rows.len();
        let first = rows.iter().next().unwrap();
        calls.lock().unwrap().push((first, len));
    }));
    let predicate = elements(&list, child, move |_| 0..len);
    predicate.evaluate(0..1, &mut [false]);
    assert_eq!(*calls.lock().unwrap(), [
        (0, CHUNK),
        (CHUNK, CHUNK),
        (CHUNK * 2, 17)
    ]);
}
