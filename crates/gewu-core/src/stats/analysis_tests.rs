use std::sync::Arc;

use arrow::{
    array::{new_null_array, ArrayRef, DictionaryArray, Float64Array, Int8Array, StringArray},
    datatypes::{Field, Int8Type},
    record_batch::RecordBatch,
};

use super::*;
use crate::{AxisScale, ColumnId, Dataset, Error, FilteredView};

fn dataset(array: &ArrayRef) -> Arc<Dataset> {
    let batch = RecordBatch::try_from_iter([("value", array.clone())]).unwrap();
    Arc::new(Dataset {
        source_path: Default::default(),
        schema: batch.schema(),
        columns: vec![crate::column::Column::new("value", array.clone()).unwrap()],
        n_rows: array.len(),
        file_size: 0,
        format: crate::FileType::Csv,
        metadata: crate::FileMetadata::Delimited,
    })
}

fn view(array: &ArrayRef) -> Arc<FilteredView> {
    dataset(array).unfiltered()
}

#[test]
fn capabilities_describe_supported_operations() {
    use arrow::datatypes::Fields;
    let item = |data_type| Arc::new(Field::new("item", data_type, true));
    let dictionary = |value| DataType::Dictionary(Box::new(DataType::Int32), Box::new(value));
    for (data_type, expected) in [
        (DataType::Int32, AnalysisCapabilities {
            statistics: true,
            distribution: true,
            values: true,
            ..Default::default()
        }),
        (DataType::Utf8, AnalysisCapabilities {
            values: true,
            lengths: true,
            ..Default::default()
        }),
        (DataType::Binary, AnalysisCapabilities {
            lengths: true,
            ..Default::default()
        }),
        (
            DataType::List(item(DataType::Int32)),
            AnalysisCapabilities {
                lengths: true,
                structure: true,
                ..Default::default()
            },
        ),
        (
            DataType::Struct(Fields::from(vec![Field::new("a", DataType::Int32, true)])),
            AnalysisCapabilities {
                structure: true,
                ..Default::default()
            },
        ),
        (dictionary(DataType::Int64), AnalysisCapabilities {
            values: true,
            structure: true,
            ..Default::default()
        }),
        (DataType::Null, AnalysisCapabilities::default()),
    ] {
        assert_eq!(capabilities(&data_type), expected, "{data_type}");
    }
}

#[test]
fn direct_analysis_requests_reject_unsupported_types_without_capability_discovery() {
    let array: ArrayRef = Arc::new(StringArray::from(vec!["text"]));
    let text = view(&array);
    let number = view(&(Arc::new(arrow::array::Int32Array::from(vec![1])) as ArrayRef));
    let binary = view(&(Arc::new(arrow::array::BinaryArray::from_vec(vec![b"x"])) as ArrayRef));
    let c0 = ColumnId::new(0);
    assert!(matches!(text.statistics(c0), Err(Error::Analysis(_))));
    assert!(matches!(number.lengths(c0, 10), Err(Error::Analysis(_))));
    assert!(matches!(binary.values(c0), Err(Error::Analysis(_))));
    assert!(matches!(
        dataset(&array).column_structure(c0),
        Err(Error::Analysis(_))
    ));
    assert_eq!(text.count_nulls(c0).unwrap(), 0);
    assert_eq!(number.statistics(c0).unwrap().mean, Some(1.0));
}

#[test]
fn requested_bins_apply_to_numeric_and_length_distributions() {
    let numbers: ArrayRef = Arc::new(Float64Array::from_iter_values((0..100).map(f64::from)));
    let text: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..100).map(|n| "x".repeat(n)),
    ));
    let numbers = view(&numbers);
    let text = view(&text);
    let c0 = ColumnId::new(0);
    assert!(matches!(
        numbers.distribution(c0, 0, AxisScale::Linear),
        Err(Error::Analysis(_))
    ));
    assert!(matches!(text.lengths(c0, 0), Err(Error::Analysis(_))));
    for bins in [10, 20] {
        for histogram in [
            numbers.distribution(c0, bins, AxisScale::Linear).unwrap(),
            text.lengths(c0, bins).unwrap().histogram,
        ] {
            assert_eq!(histogram.counts.len(), bins as usize);
            assert_eq!(histogram.counts.iter().sum::<u64>(), 100);
        }
    }
}

#[test]
fn distribution_rejects_unsupported_types_and_logarithmic_temporal_axes() {
    let text: ArrayRef = Arc::new(StringArray::from(vec!["a"]));
    let dates: ArrayRef = Arc::new(arrow::array::Date32Array::from(vec![1, 2]));
    let c0 = ColumnId::new(0);
    assert!(matches!(
        view(&text).distribution(c0, 2, AxisScale::Linear),
        Err(Error::Analysis(_))
    ));
    assert!(matches!(
        view(&dates).distribution(c0, 2, AxisScale::Log10),
        Err(Error::Analysis(_))
    ));
    assert_eq!(
        view(&dates)
            .distribution(c0, 2, AxisScale::Linear)
            .unwrap()
            .counts
            .iter()
            .sum::<u64>(),
        2
    );
}

#[test]
fn statistics_reports_non_finite_values_without_misleading_moments() {
    let c0 = ColumnId::new(0);
    for values in [vec![Some(f64::INFINITY)], vec![
        Some(1.0),
        Some(f64::INFINITY),
        Some(f64::NAN),
        None,
    ]] {
        let array: ArrayRef = Arc::new(Float64Array::from(values));
        let statistics = view(&array).statistics(c0).unwrap();
        assert_eq!(statistics.infinite_count, 1);
        assert_eq!(statistics.nan_count, u64::from(array.len() > 1));
        assert_eq!((statistics.mean, statistics.std_dev), (None, None));
        assert!(statistics.max_label.is_some());
    }
}

#[test]
fn dictionary_null_values_and_keys_share_one_null_category() {
    let values: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), None, Some("b")]));
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), Some(1), None, Some(2), Some(0)]),
            values,
        )
        .unwrap(),
    );
    let c0 = ColumnId::new(0);
    let null_count = view(&array).count_nulls(c0).unwrap();
    assert_eq!(null_count, 2);
    let summary = view(&array).values(c0).unwrap();
    assert_eq!(summary.distinct_count, 2);
    assert_eq!(
        summary
            .values
            .iter()
            .map(|v| (v.label.as_str(), v.count))
            .collect::<Vec<_>>(),
        [("a", 2), ("b", 1)]
    );
}

#[test]
fn nested_field_labels_preserve_parameters_and_dictionary_storage_details() {
    use arrow::datatypes::TimeUnit;

    let types = [
        (DataType::Decimal256(50, 12), "decimal(50,12)"),
        (
            DataType::Timestamp(TimeUnit::Nanosecond, Some("Europe/London".into())),
            "timestamp(Europe/London)",
        ),
        (
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::UInt64)),
            "uint64",
        ),
    ];
    let c0 = ColumnId::new(0);
    for (data_type, label) in types {
        let array = new_null_array(
            &DataType::Struct(vec![Field::new("child", data_type.clone(), false)].into()),
            0,
        );
        let fields = dataset(&array).column_structure(c0).unwrap();
        assert_eq!(fields, vec![AnalysisField {
            name: "child".into(),
            display_type: label.into(),
            technical_type: data_type.to_string(),
            nullable: false,
        }]);
    }
}

#[test]
fn structural_capabilities_use_type_metadata() {
    let field = Arc::new(Field::new("child", DataType::Int32, true));
    let c0 = ColumnId::new(0);
    for data_type in [
        DataType::List(field.clone()),
        DataType::Struct(vec![field.clone()].into()),
    ] {
        let support = capabilities(&data_type);
        assert!(support.structure);
        assert!(!support.statistics);
        assert!(!support.values);
        let array = new_null_array(&data_type, 0);
        let fields = dataset(&array).column_structure(c0).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(
            (
                &*fields[0].name,
                &*fields[0].display_type,
                fields[0].nullable
            ),
            ("child", "int32", true)
        );
        assert_eq!(fields[0].technical_type, "Int32");
    }
}

/// A filtered view scans its bitmap; every analysis must match the same
/// rows scanned as explicit row IDs.
#[test]
fn bitmap_selections_match_row_selections_for_every_analysis() {
    use crate::col_view::fixture::Fixture;
    let n = 1000_usize;
    let value = |i: usize| (i * 7919 % 613) as f64 - 100.0;
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from_iter(
            (0..n).map(|i| (i % 11 != 0).then(|| value(i))),
        )),
        Arc::new(arrow::array::Int64Array::from_iter(
            (0..n).map(|i| (i % 13 != 0).then(|| value(i) as i64)),
        )),
        Arc::new(StringArray::from_iter(
            (0..n).map(|i| (i % 17 != 0).then(|| format!("s{}é", i % 300))),
        )),
    ];
    let rows: Vec<u32> = (0..n as u32).filter(|row| row % 3 != 1).collect();
    for array in arrays {
        let by_rows = Fixture::new(&array, rows.clone());
        let by_mask = Fixture::masked(&array, rows.clone());
        let (r, m) = (by_rows.view(), by_mask.view());
        assert!(matches!(m.selection(), crate::col_view::Selection::Mask(_)));
        let debug = |view| {
            format!(
                "{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
                super::count_nulls(view),
                crate::stats::analyze_statistics(view)
                    .map(|s| format!("{s:?}"))
                    .ok(),
                crate::stats::counts_scaled(view, 7, AxisScale::Linear).ok(),
                crate::stats::counts_scaled(view, 7, AxisScale::Log10).ok(),
                crate::stats::build_scaled(view, 7, AxisScale::Linear).ok(),
                crate::stats::analyze_values(view).ok(),
                crate::stats::analyze_lengths(view, 7).ok(),
            )
        };
        assert_eq!(debug(r), debug(m), "{}", array.data_type());
        let scatter = |view: crate::col_view::ColView<'_>| {
            crate::stats::scatter::analyze(
                None,
                (ColumnId::new(0), view.array()),
                view.rows(),
                64,
                AxisScale::Linear,
                AxisScale::Linear,
            )
            .map(|s| format!("{s:?}"))
            .ok()
        };
        assert_eq!(scatter(r), scatter(m));
    }
}
