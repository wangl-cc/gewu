#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, RecordBatch, Time32MillisecondArray, TimestampMicrosecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use gewu_core::{Cancellation, Error, Filter, NumericStatistics, QuerySpec, ValueSummary};
use parquet::arrow::ArrowWriter;

fn statistics(doc: &std::sync::Arc<gewu_core::View>) -> NumericStatistics {
    doc.analyze_statistics(0, &gewu_core::Cancellation::default())
        .unwrap()
}

fn values(doc: &std::sync::Arc<gewu_core::View>) -> ValueSummary {
    doc.analyze_values(0, &gewu_core::Cancellation::default())
        .unwrap()
}

fn document(array: ArrayRef) -> std::sync::Arc<gewu_core::View> {
    let file = tempfile::NamedTempFile::new().unwrap();
    let batch = RecordBatch::try_from_iter(vec![("value", array)]).unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    gewu_core::Dataset::open(file.path())
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap()
}

fn check_range(
    doc: &std::sync::Arc<gewu_core::View>,
    min: Option<String>,
    max: Option<String>,
    expected: &[u64],
) -> Arc<gewu_core::View> {
    let spec = QuerySpec::single(0, Filter::range(min, max, false));
    let view = doc
        .query(&spec, &gewu_core::Cancellation::default())
        .unwrap();
    assert_eq!(
        view.rows(0, 100, &[0]).unwrap().source_rows,
        expected,
        "type {:?}, spec {spec:?}",
        doc.dataset().schema().field(0).data_type()
    );
    view
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
                let min = min.map(|text| (text).to_owned());
                let max = max.map(|text| (text).to_owned());
                check_range(&doc, min, max, expected);
            }
            let max = (<$native>::MAX as i64).to_string();
            let min = (<$native>::MIN as i64).to_string();
            check_range(&doc, Some(max), None, &[6]);
            check_range(&doc, None, Some(min), &[0]);
            let outside = (u64::MAX).to_string();
            check_range(&doc, Some(outside), None, &[]);
            for input in ["inf", "-inf", "NaN", "invalid"] {
                assert!(matches!(
                    doc.query(
                        &QuerySpec::single(0, Filter::range(Some((input).to_owned()), None, false)),
                        &Cancellation::default()
                    ),
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
                let min = min.map(|text| (text).to_owned());
                let max = max.map(|text| (text).to_owned());
                check_range(&doc, min, max, expected);
            }
            let max = (<$native>::MAX as u64).to_string();
            check_range(&doc, Some(max), None, &[3]);
            for input in ["inf", "-inf", "NaN", "invalid"] {
                assert!(matches!(
                    doc.query(
                        &QuerySpec::single(0, Filter::range(Some((input).to_owned()), None, false)),
                        &Cancellation::default()
                    ),
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
        Some((large + 1).to_string()),
        Some((large + 1).to_string()),
        &[2],
    );
    check_range(
        &signed,
        Some((large).to_string()),
        Some((large).to_string()),
        &[1],
    );
    check_range(&signed, Some(("9223372036854775808").to_owned()), None, &[]);
    check_range(&signed, None, Some(("-9223372036854775808").to_owned()), &[
        0,
    ]);

    let unsigned = document(Arc::new(UInt64Array::from(vec![
        0,
        large as u64,
        large as u64 + 1,
        u64::MAX,
    ])));
    check_range(
        &unsigned,
        Some((large as u64 + 1).to_string()),
        Some((large as u64 + 1).to_string()),
        &[2],
    );
    check_range(
        &unsigned,
        Some(("18446744073709551616").to_owned()),
        None,
        &[],
    );
    check_range(
        &unsigned,
        None,
        Some(("18446744073709551616").to_owned()),
        &[0, 1, 2, 3],
    );

    signed
        .query(&QuerySpec::default(), &gewu_core::Cancellation::default())
        .unwrap();
    unsigned
        .query(&QuerySpec::default(), &gewu_core::Cancellation::default())
        .unwrap();
    assert_eq!(values(&signed).distinct_count, 4);
    assert_eq!(values(&unsigned).distinct_count, 4);
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
    check_range(&doc, None, Some(("0.1").to_owned()), &[3]);
    check_range(&doc, Some(("0.1").to_owned()), Some(("1").to_owned()), &[
        0, 1,
    ]);
    check_range(&doc, None, None, &[0, 1, 3, 4]);
    check_range(&doc, Some(("inf").to_owned()), None, &[4]);
    check_range(&doc, None, Some(("-inf").to_owned()), &[3]);
    assert!(matches!(
        doc.query(
            &QuerySpec::single(0, Filter::range(Some(("NaN").to_owned()), None, false)),
            &Cancellation::default()
        ),
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

    check_range(&integers, Some(("1.2").to_owned()), None, &[2, 3]);

    check_range(&integers, Some(("128").to_owned()), None, &[]);
    assert!(matches!(
        integers.query(
            &QuerySpec::single(
                0,
                Filter::range(Some(("not a number").to_owned()), None, false)
            ),
            &Cancellation::default()
        ),
        Err(Error::InvalidArgument(_))
    ));
    assert!(integers
        .query(
            &QuerySpec::single(
                0,
                Filter::range(Some(("not a number").to_owned()), None, false)
            ),
            &Cancellation::default()
        )
        .is_err());

    let floats = document(Arc::new(Float32Array::from(vec![0.0, 0.1, 1.0])));

    check_range(&floats, None, Some(("0.1").to_owned()), &[0]);

    let unsigned = document(Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX])));
    // Both values have the same approximate chart coordinate, but the raw
    // filter must still select only the exact maximum.

    check_range(&unsigned, Some((u64::MAX).to_string()), None, &[1]);
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

        check_range(&doc, Some((input).to_owned()), Some((input).to_owned()), &[
            1,
        ]);
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
    check_range(&doc, Some((large + 1).to_string()), None, &[3, 4]);
    check_range(&doc, None, Some((large + 1).to_string()), &[0, 1, 2]);
    check_range(&doc, Some((-large - 1).to_string()), None, &[1, 2, 3, 4]);
    check_range(&doc, None, Some((-large - 1).to_string()), &[0]);
    check_range(&doc, Some((u64::MAX).to_string()), None, &[4]);
    check_range(&doc, None, Some((u64::MAX).to_string()), &[0, 1, 2, 3]);
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
    let summary = statistics(&doc);
    assert_eq!(summary.nan_count, 1);
    assert_eq!(
        (
            summary.min_label.as_deref(),
            summary.max_label.as_deref(),
            summary.mean,
            summary.std_dev
        ),
        (Some("1.0"), Some("3.0"), Some(2.0), Some(1.0))
    );
    let histogram = doc
        .filter_distribution(0, 2, gewu_core::AxisScale::Linear, &Cancellation::default())
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!(histogram.counts, [1, 2]);
    assert_eq!(
        (histogram.null_count, histogram.nan_count, histogram.total),
        (1, 1, 5)
    );

    let doc = check_range(&doc, Some(("2").to_owned()), None, &[1, 2]);
    let summary = statistics(&doc);
    assert_eq!(
        (doc.row_count(), values(&doc).distinct_count, summary.mean),
        (2, 2, Some(2.5))
    );
    let histogram = doc
        .analyze_distribution(
            0,
            2,
            gewu_core::AxisScale::Linear,
            &gewu_core::Cancellation::default(),
        )
        .unwrap();
    assert_eq!((histogram.lo, histogram.hi), (Some(2.0), Some(3.0)));
    assert_eq!(histogram.counts, [1, 1]);
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
        let summary = statistics(&doc);
        assert_eq!((values(&doc).distinct_count, summary.mean), (3, Some(1.0)));
        let histogram = doc
            .filter_distribution(
                0,
                10,
                gewu_core::AxisScale::Linear,
                &Cancellation::default(),
            )
            .map(|result| result.expect("histogram domain").histogram)
            .unwrap();
        assert!(histogram.integer);
        assert_eq!((histogram.lo, histogram.hi), (Some(0.0), Some(3.0)));
        assert_eq!(histogram.counts, [1, 1, 1]);
        let min = doc.dataset().format_value(0, 1).unwrap();
        let max = doc.dataset().format_value(0, 2).unwrap();
        check_range(&doc, Some(min), Some(max), &[1, 2]);
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
        assert_eq!(values(&doc).distinct_count, 2);
        // Chart coordinates may round, but filter endpoints remain exact.

        check_range(&doc, Some((base + 1).to_string()), None, &[1]);
        check_range(
            &doc,
            Some(format!("{base}.1")),
            Some(format!("{}.9", base + 1)),
            &[1],
        );
    }
}
