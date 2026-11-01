#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, RecordBatch, Time32MillisecondArray, TimestampMicrosecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use gewu_core::{Bound, Error, Filter, QuerySpec, Session};
use parquet::arrow::ArrowWriter;

fn document(array: ArrayRef) -> Session {
    let file = tempfile::NamedTempFile::new().unwrap();
    let batch = RecordBatch::try_from_iter(vec![("value", array)]).unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Session::open(file.path()).unwrap()
}

fn check_range(doc: &Session, min: Option<Bound>, max: Option<Bound>, expected: &[u64]) {
    let spec = QuerySpec::single(0, Filter::Range {
        include_nulls: false,
        min,
        max,
    });
    let view = doc.apply_view(&spec).unwrap();
    assert_eq!(
        doc.rows(view.version, 0, 100, &[0]).unwrap().source_rows,
        expected,
        "type {:?}, spec {spec:?}",
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
            for (min, max, expected) in [
                (None, None, &[0, 1, 2, 3, 4, 5, 6][..]),
                (Some("-1.2"), Some("1.2"), &[2, 3, 4]),
                (Some("1.2"), Some("1.8"), &[]),
                (Some("1e30"), None, &[]),
                (None, Some("-1e30"), &[]),
            ] {
                let min = min.map(|text| doc.parse_bound(0, text).unwrap());
                let max = max.map(|text| doc.parse_bound(0, text).unwrap());
                check_range(&doc, min, max, expected);
            }
            let max = doc.make_bound_i64(0, <$native>::MAX as i64).unwrap();
            let min = doc.make_bound_i64(0, <$native>::MIN as i64).unwrap();
            check_range(&doc, Some(max), None, &[6]);
            check_range(&doc, None, Some(min), &[0]);
            let outside = doc.make_bound_u64(0, u64::MAX).unwrap();
            check_range(&doc, Some(outside), None, &[]);
            for input in ["inf", "-inf", "NaN", "invalid"] {
                assert!(matches!(
                    doc.parse_bound(0, input),
                    Err(Error::InvalidArgument(_))
                ));
            }
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
            for (min, max, expected) in [
                (None, None, &[0, 1, 2, 3][..]),
                (Some("0.2"), Some("2.8"), &[1, 2]),
                (None, Some("-1"), &[]),
                (None, Some("-0.1"), &[]),
                (Some("1e30"), None, &[]),
                (Some("-1"), None, &[0, 1, 2, 3]),
            ] {
                let min = min.map(|text| doc.parse_bound(0, text).unwrap());
                let max = max.map(|text| doc.parse_bound(0, text).unwrap());
                check_range(&doc, min, max, expected);
            }
            let max = doc.make_bound_u64(0, <$native>::MAX as u64).unwrap();
            check_range(&doc, Some(max), None, &[3]);
            for input in ["inf", "-inf", "NaN", "invalid"] {
                assert!(matches!(
                    doc.parse_bound(0, input),
                    Err(Error::InvalidArgument(_))
                ));
            }
        }};
    }
    check!(UInt8Array, u8);
    check!(UInt16Array, u16);
    check!(UInt32Array, u32);
    check!(UInt64Array, u64);
}

#[test]
fn large_integer_ranges_keep_exact_values_and_domain_edges() {
    let large = 1_i64 << 53;
    let signed = document(Arc::new(Int64Array::from(vec![
        i64::MIN,
        large,
        large + 1,
        i64::MAX,
    ])));
    check_range(
        &signed,
        Some(signed.make_bound_i64(0, large + 1).unwrap()),
        Some(signed.make_bound_i64(0, large + 1).unwrap()),
        &[2],
    );
    check_range(
        &signed,
        Some(signed.make_bound_i64(0, large).unwrap()),
        Some(signed.make_bound_i64(0, large).unwrap()),
        &[1],
    );
    check_range(
        &signed,
        Some(signed.parse_bound(0, "9223372036854775808").unwrap()),
        None,
        &[],
    );
    check_range(
        &signed,
        None,
        Some(signed.parse_bound(0, "-9223372036854775808").unwrap()),
        &[0],
    );

    let unsigned = document(Arc::new(UInt64Array::from(vec![
        0,
        large as u64,
        large as u64 + 1,
        u64::MAX,
    ])));
    check_range(
        &unsigned,
        Some(unsigned.make_bound_u64(0, large as u64 + 1).unwrap()),
        Some(unsigned.make_bound_u64(0, large as u64 + 1).unwrap()),
        &[2],
    );
    check_range(
        &unsigned,
        Some(unsigned.parse_bound(0, "18446744073709551616").unwrap()),
        None,
        &[],
    );
    check_range(
        &unsigned,
        None,
        Some(unsigned.parse_bound(0, "18446744073709551616").unwrap()),
        &[0, 1, 2, 3],
    );

    signed.apply_view(&QuerySpec::default()).unwrap();
    unsigned.apply_view(&QuerySpec::default()).unwrap();
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
    check_range(&doc, None, Some(doc.parse_bound(0, "0.1").unwrap()), &[3]);
    check_range(
        &doc,
        Some(doc.parse_bound(0, "0.1").unwrap()),
        Some(doc.parse_bound(0, "1").unwrap()),
        &[0, 1],
    );
    check_range(&doc, None, None, &[0, 1, 3, 4]);
    check_range(&doc, Some(doc.parse_bound(0, "inf").unwrap()), None, &[4]);
    check_range(&doc, None, Some(doc.parse_bound(0, "-inf").unwrap()), &[3]);
    assert!(matches!(
        doc.parse_bound(0, "NaN"),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn raw_numeric_filters_preserve_precision_independently_of_axis_projection() {
    let integers = document(Arc::new(Int8Array::from(vec![
        Some(0),
        Some(1),
        Some(2),
        Some(i8::MAX),
        None,
    ])));
    assert_eq!(integers.parse_bound(0, "1.2").unwrap().axis_value(), 1.2);
    check_range(
        &integers,
        Some(integers.parse_bound(0, "1.2").unwrap()),
        None,
        &[2, 3],
    );
    assert_eq!(integers.parse_bound(0, "128").unwrap().axis_value(), 128.0);
    check_range(
        &integers,
        Some(integers.parse_bound(0, "128").unwrap()),
        None,
        &[],
    );
    assert!(matches!(
        integers.parse_bound(0, "not a number"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(integers.parse_bound(0, "not a number").is_err());

    let floats = document(Arc::new(Float32Array::from(vec![0.0, 0.1, 1.0])));
    assert_eq!(floats.parse_bound(0, "0.1").unwrap().axis_value(), 0.1);
    check_range(
        &floats,
        None,
        Some(floats.parse_bound(0, "0.1").unwrap()),
        &[0],
    );

    let unsigned = document(Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX])));
    // Both values have the same approximate chart coordinate, but the raw
    // filter must still select only the exact maximum.
    assert_eq!(
        unsigned
            .parse_bound(0, &(u64::MAX - 1).to_string())
            .unwrap()
            .axis_value(),
        unsigned.make_bound_u64(0, u64::MAX).unwrap().axis_value()
    );
    check_range(
        &unsigned,
        Some(unsigned.make_bound_u64(0, u64::MAX).unwrap()),
        None,
        &[1],
    );
}

#[test]
fn temporal_axis_projection_and_filters_use_the_columns_physical_units() {
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
        assert_eq!(doc.parse_bound(0, input).unwrap().axis_value(), 1.0);
        check_range(
            &doc,
            Some(doc.parse_bound(0, input).unwrap()),
            Some(doc.parse_bound(0, input).unwrap()),
            &[1],
        );
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
    check_range(
        &doc,
        Some(doc.make_bound_i64(0, large + 1).unwrap()),
        None,
        &[3, 4],
    );
    check_range(
        &doc,
        None,
        Some(doc.make_bound_i64(0, large + 1).unwrap()),
        &[0, 1, 2],
    );
    check_range(
        &doc,
        Some(doc.make_bound_i64(0, -large - 1).unwrap()),
        None,
        &[1, 2, 3, 4],
    );
    check_range(
        &doc,
        None,
        Some(doc.make_bound_i64(0, -large - 1).unwrap()),
        &[0],
    );
    check_range(
        &doc,
        Some(doc.make_bound_u64(0, u64::MAX).unwrap()),
        None,
        &[4],
    );
    check_range(
        &doc,
        None,
        Some(doc.make_bound_u64(0, u64::MAX).unwrap()),
        &[0, 1, 2, 3],
    );
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

    check_range(&doc, Some(doc.parse_bound(0, "2").unwrap()), None, &[1, 2]);
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
        let min = doc.format_value(0, 1).unwrap();
        let max = doc.format_value(0, 2).unwrap();
        check_range(
            &doc,
            Some(doc.parse_bound(0, &min).unwrap()),
            Some(doc.parse_bound(0, &max).unwrap()),
            &[1, 2],
        );
    }
}

#[test]
fn decimal_summary_and_ranges_distinguish_adjacent_large_coefficients() {
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
        // Chart coordinates may round, but filter endpoints remain exact.
        assert_eq!(
            doc.parse_bound(0, &base.to_string()).unwrap().axis_value(),
            doc.parse_bound(0, &(base + 1).to_string())
                .unwrap()
                .axis_value()
        );
        check_range(
            &doc,
            Some(doc.parse_bound(0, &(base + 1).to_string()).unwrap()),
            None,
            &[1],
        );
        check_range(
            &doc,
            Some(doc.parse_bound(0, &format!("{base}.1")).unwrap()),
            Some(doc.parse_bound(0, &format!("{}.9", base + 1)).unwrap()),
            &[1],
        );
    }
}
