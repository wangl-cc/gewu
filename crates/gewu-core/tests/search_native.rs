use std::sync::Arc;

use arrow::{
    array::{
        ArrayRef, Float32Array, Int16Array, Int32Array, Int64Array, Int8Array, RecordBatch,
        StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    },
    compute::cast,
    datatypes::{DataType, TimeUnit},
};
use gewu_core::{Document, Filter, ViewState};
use parquet::arrow::ArrowWriter;

/// Exercise both callers of the typed matcher, including fetched-cell highlights.
fn check_queries(array: ArrayRef, queries: &[(&str, Vec<u64>)]) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let batch = RecordBatch::try_from_iter(vec![("value", array)]).unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let doc = Document::open(file.path()).unwrap();

    for (query, expected) in queries {
        let states = [
            ViewState {
                search: Some((*query).into()),
                ..Default::default()
            },
            ViewState {
                filters: vec![Filter::Contains {
                    column: 0,
                    text: (*query).into(),
                    case_sensitive: false,
                }],
                ..Default::default()
            },
        ];
        for state in states {
            let view = doc.apply_view(&state).unwrap();
            let rows = doc.rows(view.version, 0, 100, &[0]).unwrap();
            assert_eq!(
                &rows.source_rows,
                expected,
                "query {query:?}, type {:?}, state {state:?}",
                batch.schema().field(0).data_type()
            );
            if state.search.is_some() {
                assert_eq!(rows.matched, vec![true; expected.len()]);
            }
        }
    }
}

#[test]
fn integer_search_keeps_native_bounds_and_nulls() {
    macro_rules! check_integer {
        ($array:ty, $native:ty) => {{
            let min = <$native>::MIN;
            let max = <$native>::MAX;
            check_queries(
                Arc::new(<$array>::from(vec![Some(min), Some(1), Some(max), None])),
                &[
                    (&min.to_string(), vec![0]),
                    (&max.to_string(), vec![2]),
                    (&((min as i128) - 1).to_string(), vec![]),
                    (&((max as i128) + 1).to_string(), vec![]),
                    ("+1", vec![1]),
                    ("1.0", vec![]),
                    ("word", vec![]),
                ],
            );
        }};
    }
    check_integer!(Int8Array, i8);
    check_integer!(Int16Array, i16);
    check_integer!(Int32Array, i32);
    check_integer!(Int64Array, i64);
    check_integer!(UInt8Array, u8);
    check_integer!(UInt16Array, u16);
    check_integer!(UInt32Array, u32);
    check_integer!(UInt64Array, u64);
}

#[test]
fn float_search_keeps_precision_and_special_values_at_every_width() {
    let values: ArrayRef = Arc::new(Float32Array::from(vec![
        Some(-1.23),
        Some(1.19),
        Some(1.23),
        Some(1.29),
        None,
        Some(f32::NAN),
        Some(f32::INFINITY),
        Some(f32::NEG_INFINITY),
        Some(-0.0),
        Some(0.0),
    ]));
    for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
        check_queries(cast(&values, &data_type).unwrap(), &[
            ("1.2", vec![1, 2]),
            ("1.20", vec![]),
            ("-1.2", vec![0]),
            ("nan", vec![5]),
            ("inf", vec![6]),
            ("-inf", vec![7]),
            ("0.0000", vec![8, 9]),
            ("word", vec![]),
        ]);
    }
}

#[test]
fn temporal_search_keeps_native_units_and_pre_epoch_values() {
    let dates: ArrayRef = Arc::new(StringArray::from(vec![
        Some("1969-12-31"),
        Some("1970-01-01"),
        Some("2024-03-15"),
        None,
    ]));
    for data_type in [DataType::Date32, DataType::Date64] {
        check_queries(cast(&dates, &data_type).unwrap(), &[
            ("1969", vec![0]),
            ("1970-01-01", vec![1]),
            ("2024-03", vec![2]),
        ]);
    }
    let timestamps: ArrayRef = Arc::new(StringArray::from(vec![
        Some("1969-12-31T23:59:59"),
        Some("1970-01-01T00:00:00"),
        Some("2024-03-15T10:30:15"),
        None,
    ]));
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        check_queries(
            cast(&timestamps, &DataType::Timestamp(unit, Some("UTC".into()))).unwrap(),
            &[
                ("1969-12-31 23:59", vec![0]),
                ("1970", vec![1]),
                ("2024-03-15 10:30:15", vec![2]),
                ("2024-03-15 10:30:16", vec![]),
            ],
        );
    }
}
