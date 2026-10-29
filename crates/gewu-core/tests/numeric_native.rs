use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, RecordBatch, Time32MillisecondArray, TimestampMicrosecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use gewu_core::{Bound, Document, Filter, ViewState};
use parquet::arrow::ArrowWriter;

fn document(array: ArrayRef) -> Document {
    let file = tempfile::NamedTempFile::new().unwrap();
    let batch = RecordBatch::try_from_iter(vec![("value", array)]).unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Document::open(file.path()).unwrap()
}

fn check_range(doc: &Document, min: Option<Bound>, max: Option<Bound>, expected: &[u64]) {
    let view = doc
        .apply_view(&ViewState {
            filters: vec![Filter::Range {
                column: 0,
                min,
                max,
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        doc.rows(view.version, 0, 100, &[0]).unwrap().source_rows,
        expected,
        "type {:?}, min {min:?}, max {max:?}",
        doc.schema().field(0).data_type()
    );
}

#[test]
fn signed_native_ranges_handle_fractional_and_out_of_domain_bounds() {
    macro_rules! check {
        ($array:ty, $native:ty) => {{
            let doc = document(Arc::new(<$array>::from(vec![
                Some(<$native>::MIN),
                Some(-2),
                Some(-1),
                Some(0),
                Some(1),
                Some(2),
                Some(<$native>::MAX),
                None,
            ])));
            check_range(&doc, None, None, &[0, 1, 2, 3, 4, 5, 6]);
            check_range(
                &doc,
                Some(Bound::Float(-1.2)),
                Some(Bound::Float(1.2)),
                &[2, 3, 4],
            );
            check_range(&doc, Some(Bound::Float(1.2)), Some(Bound::Float(1.8)), &[]);
            check_range(&doc, Some(Bound::Int(<$native>::MAX as i64)), None, &[6]);
            check_range(&doc, None, Some(Bound::Int(<$native>::MIN as i64)), &[0]);
            check_range(&doc, Some(Bound::Float(1e30)), None, &[]);
            check_range(&doc, None, Some(Bound::Float(-1e30)), &[]);
            check_range(&doc, Some(Bound::UInt(u64::MAX)), None, &[]);
            check_range(&doc, Some(Bound::Float(f64::INFINITY)), None, &[]);
            check_range(&doc, None, Some(Bound::Float(f64::NEG_INFINITY)), &[]);
            check_range(
                &doc,
                Some(Bound::Float(f64::NEG_INFINITY)),
                Some(Bound::Float(f64::INFINITY)),
                &[0, 1, 2, 3, 4, 5, 6],
            );
            check_range(&doc, Some(Bound::Float(f64::NAN)), None, &[]);
        }};
    }
    check!(Int8Array, i8);
    check!(Int16Array, i16);
    check!(Int32Array, i32);
    check!(Int64Array, i64);
}

#[test]
fn unsigned_native_ranges_do_not_saturate_impossible_bounds() {
    macro_rules! check {
        ($array:ty, $native:ty) => {{
            let doc = document(Arc::new(<$array>::from(vec![
                Some(0),
                Some(1),
                Some(2),
                Some(<$native>::MAX),
                None,
            ])));
            check_range(&doc, None, None, &[0, 1, 2, 3]);
            check_range(
                &doc,
                Some(Bound::Float(0.2)),
                Some(Bound::Float(2.8)),
                &[1, 2],
            );
            check_range(&doc, Some(Bound::UInt(<$native>::MAX as u64)), None, &[3]);
            check_range(&doc, None, Some(Bound::Int(-1)), &[]);
            check_range(&doc, None, Some(Bound::Float(-0.1)), &[]);
            check_range(&doc, Some(Bound::Float(1e30)), None, &[]);
            check_range(&doc, Some(Bound::Float(f64::INFINITY)), None, &[]);
            check_range(&doc, None, Some(Bound::Float(f64::NEG_INFINITY)), &[]);
            check_range(
                &doc,
                Some(Bound::Int(-1)),
                Some(Bound::Float(f64::INFINITY)),
                &[0, 1, 2, 3],
            );
            check_range(&doc, None, Some(Bound::Float(f64::NAN)), &[]);
        }};
    }
    check!(UInt8Array, u8);
    check!(UInt16Array, u16);
    check!(UInt32Array, u32);
    check!(UInt64Array, u64);
}

#[test]
fn large_integer_ranges_keep_exact_values_and_float_domain_edges() {
    let large = 1_i64 << 53;
    let signed = document(Arc::new(Int64Array::from(vec![
        i64::MIN,
        large,
        large + 1,
        i64::MAX,
    ])));
    check_range(
        &signed,
        Some(Bound::Int(large + 1)),
        Some(Bound::Int(large + 1)),
        &[2],
    );
    check_range(
        &signed,
        Some(Bound::Float(large as f64)),
        Some(Bound::Float(large as f64)),
        &[1],
    );
    check_range(&signed, Some(Bound::Float(2_f64.powi(63))), None, &[]);
    check_range(&signed, None, Some(Bound::Float(-2_f64.powi(63))), &[0]);

    let unsigned = document(Arc::new(UInt64Array::from(vec![
        0,
        large as u64,
        large as u64 + 1,
        u64::MAX,
    ])));
    check_range(
        &unsigned,
        Some(Bound::UInt(large as u64 + 1)),
        Some(Bound::UInt(large as u64 + 1)),
        &[2],
    );
    check_range(&unsigned, Some(Bound::Float(2_f64.powi(64))), None, &[]);
    check_range(
        &unsigned,
        None,
        Some(Bound::Float(2_f64.powi(64))),
        &[0, 1, 2, 3],
    );

    signed.apply_view(&ViewState::default()).unwrap();
    unsigned.apply_view(&ViewState::default()).unwrap();
    assert_eq!(signed.column_summary(0, 0).unwrap().distinct_count, 4);
    assert_eq!(unsigned.column_summary(0, 0).unwrap().distinct_count, 4);
}

#[test]
fn float32_range_compares_without_rounding_the_query_to_float32() {
    let doc = document(Arc::new(Float32Array::from(vec![
        Some(0.1),
        Some(1.0),
        Some(f32::NAN),
        Some(f32::NEG_INFINITY),
        Some(f32::INFINITY),
        None,
    ])));
    // The stored f32 value is slightly greater than the f64 value 0.1.
    check_range(&doc, None, Some(Bound::Float(0.1)), &[3]);
    check_range(&doc, Some(Bound::Float(0.1)), Some(Bound::Int(1)), &[0, 1]);
    check_range(&doc, None, None, &[0, 1, 3, 4]);
    check_range(&doc, Some(Bound::Float(f64::INFINITY)), None, &[4]);
    check_range(&doc, None, Some(Bound::Float(f64::NEG_INFINITY)), &[3]);
    check_range(&doc, Some(Bound::Float(f64::NAN)), None, &[]);
}

#[test]
fn parsed_numeric_bounds_keep_query_precision_and_domain() {
    let integers = document(Arc::new(Int8Array::from(vec![
        Some(0),
        Some(1),
        Some(2),
        Some(i8::MAX),
        None,
    ])));
    let fractional = integers.parse_value(0, "1.2").unwrap();
    assert_eq!(fractional, Bound::Float(1.2));
    check_range(&integers, Some(fractional), None, &[2, 3]);
    let outside = integers.parse_value(0, "128").unwrap();
    assert_eq!(outside, Bound::Int(128));
    check_range(&integers, Some(outside), None, &[]);
    assert!(integers.parse_value(0, "not a number").is_err());

    let floats = document(Arc::new(Float32Array::from(vec![0.0, 0.1, 1.0])));
    let upper = floats.parse_value(0, "0.1").unwrap();
    assert_eq!(upper, Bound::Float(0.1));
    check_range(&floats, None, Some(upper), &[0]);

    let unsigned = document(Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX])));
    let max = unsigned.parse_value(0, &u64::MAX.to_string()).unwrap();
    assert_eq!(max, Bound::UInt(u64::MAX));
    check_range(&unsigned, Some(max), None, &[1]);
}

#[test]
fn parsed_temporal_bounds_use_the_columns_physical_units() {
    let cases: [(ArrayRef, &str); 3] = [
        (Arc::new(Date32Array::from(vec![0, 1, 2])), "1970-01-02"),
        (
            Arc::new(Time32MillisecondArray::from(vec![0, 1, 2])),
            "00:00:00.001",
        ),
        (
            Arc::new(TimestampMicrosecondArray::from(vec![0, 1, 2]).with_timezone("UTC")),
            "1970-01-01T00:00:00.000001Z",
        ),
    ];
    for (array, input) in cases {
        let doc = document(array);
        let bound = doc.parse_value(0, input).unwrap();
        assert_eq!(bound, Bound::Int(1));
        check_range(&doc, Some(bound), Some(bound), &[1]);
    }
}

#[test]
fn float64_ranges_compare_large_integer_bounds_exactly() {
    let large = 1_i64 << 53;
    let doc = document(Arc::new(Float64Array::from(vec![
        -(large as f64) - 2.0,
        -(large as f64),
        large as f64,
        large as f64 + 2.0,
        2_f64.powi(64),
    ])));
    // The odd integer between these adjacent floats cannot be represented by f64.
    check_range(&doc, Some(Bound::Int(large + 1)), None, &[3, 4]);
    check_range(&doc, None, Some(Bound::Int(large + 1)), &[0, 1, 2]);
    check_range(&doc, Some(Bound::Int(-large - 1)), None, &[1, 2, 3, 4]);
    check_range(&doc, None, Some(Bound::Int(-large - 1)), &[0]);
    check_range(&doc, Some(Bound::UInt(large as u64 + 1)), None, &[3, 4]);
    check_range(&doc, None, Some(Bound::UInt(large as u64 + 1)), &[0, 1, 2]);
    check_range(&doc, Some(Bound::UInt(u64::MAX)), None, &[4]);
    check_range(&doc, None, Some(Bound::UInt(u64::MAX)), &[0, 1, 2, 3]);
}

#[test]
fn float32_summary_and_histogram_follow_selected_rows() {
    let doc = document(Arc::new(Float32Array::from(vec![
        Some(1.0),
        Some(2.0),
        Some(3.0),
        Some(f32::NAN),
        None,
    ])));
    let summary = doc.column_summary(0, 0).unwrap();
    assert_eq!(
        (
            summary.rows,
            summary.null_count,
            summary.nan_count,
            summary.distinct_count
        ),
        (5, 1, 1, 3)
    );
    assert_eq!(
        (summary.min, summary.max, summary.mean, summary.std_dev),
        (Some(1.0), Some(3.0), Some(2.0), Some(1.0))
    );
    let histogram = doc.histogram(0, None, 2).unwrap();
    assert_eq!(histogram.counts, [1, 2]);
    assert_eq!(
        (histogram.null_count, histogram.nan_count, histogram.total),
        (1, 1, 5)
    );

    check_range(&doc, Some(Bound::Int(2)), None, &[1, 2]);
    let summary = doc.column_summary(0, 0).unwrap();
    assert_eq!(
        (summary.rows, summary.distinct_count, summary.mean),
        (2, 2, Some(2.5))
    );
    let histogram = doc
        .view_histogram(doc.current_view().version, 0, 2)
        .unwrap();
    assert_eq!((histogram.lo, histogram.hi), (Some(1.0), Some(3.0)));
    assert_eq!(histogram.counts, [0, 2]);
}

#[test]
fn temporal_range_summary_and_histogram_keep_physical_units() {
    let arrays: [ArrayRef; 3] = [
        Arc::new(Date32Array::from(vec![Some(0), Some(1), Some(2), None])),
        Arc::new(Time32MillisecondArray::from(vec![
            Some(0),
            Some(1),
            Some(2),
            None,
        ])),
        Arc::new(
            TimestampMicrosecondArray::from(vec![Some(0), Some(1), Some(2), None])
                .with_timezone("UTC"),
        ),
    ];
    for array in arrays {
        let doc = document(array);
        let summary = doc.column_summary(0, 0).unwrap();
        assert_eq!(
            (summary.null_count, summary.distinct_count, summary.mean),
            (1, 3, Some(1.0))
        );
        let histogram = doc.histogram(0, None, 10).unwrap();
        assert!(histogram.integer);
        assert_eq!((histogram.lo, histogram.hi), (Some(0.0), Some(3.0)));
        assert_eq!(histogram.counts, [1, 1, 1]);
        check_range(&doc, Some(Bound::Float(0.5)), Some(Bound::Int(2)), &[1, 2]);
    }
}

#[test]
fn histogram_separates_adjacent_large_integers() {
    let large = 1_i64 << 53;
    let doc = document(Arc::new(Int64Array::from(vec![large, large + 1])));
    let histogram = doc.histogram(0, None, 10).unwrap();
    assert_eq!(histogram.counts, [1, 1]);
    assert_eq!(histogram.total, 2);
}

#[test]
fn histogram_does_not_overflow_for_finite_float_extremes() {
    let doc = document(Arc::new(Float64Array::from(vec![-f64::MAX, 0.0, f64::MAX])));
    assert_eq!(doc.histogram(0, None, 3).unwrap().counts, [1, 1, 1]);
}

#[test]
fn decimal_summary_keeps_native_distinct_values_and_existing_range_semantics() {
    use arrow::{
        array::{Decimal128Array, Decimal256Array},
        datatypes::i256,
    };
    let base = 9_007_199_254_740_992_i64;
    let arrays: [ArrayRef; 2] = [
        Arc::new(
            Decimal128Array::from(vec![i128::from(base), i128::from(base + 1)])
                .with_precision_and_scale(16, 0)
                .unwrap(),
        ),
        Arc::new(
            Decimal256Array::from(vec![i256::from(base), i256::from(base + 1)])
                .with_precision_and_scale(16, 0)
                .unwrap(),
        ),
    ];
    for array in arrays {
        let doc = document(array);
        assert_eq!(doc.column_summary(0, 0).unwrap().distinct_count, 2);
        // Bound still exposes an approximate f64 representation for decimals.
        // Preserve the existing comparison until that public contract changes.
        check_range(&doc, Some(Bound::Int(base + 1)), None, &[0, 1]);
    }
}

#[test]
fn histogram_handles_empty_nan_and_infinite_values_explicitly() {
    for values in [vec![], vec![None], vec![Some(f64::NAN), None]] {
        let doc = document(Arc::new(Float64Array::from(values.clone())));
        let h = doc.histogram(0, None, 3).unwrap();
        assert!(h.counts.is_empty());
        assert_eq!((h.lo, h.hi), (None, None));
        assert_eq!(h.total, values.len() as u64);
        assert_eq!(h.null_count + h.nan_count, h.total);
    }
    let doc = document(Arc::new(Float64Array::from(vec![
        f64::NEG_INFINITY,
        0.0,
        1.0,
        2.0,
        f64::INFINITY,
    ])));
    let h = doc.histogram(0, None, 2).unwrap();
    assert_eq!((h.lo, h.hi), (Some(0.0), Some(2.0)));
    assert_eq!(h.counts, [2, 3]);
    let doc = document(Arc::new(Float64Array::from(vec![f64::INFINITY])));
    assert!(matches!(
        doc.histogram(0, None, 2),
        Err(gewu_core::Error::InvalidArgument(_))
    ));
}
