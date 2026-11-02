//! Column analysis metadata and logical-null counts.

use arrow::{array::Array, datatypes::DataType};

use super::{ArrayView, Cancellation};
use crate::{column::display_type, scope::Selection, types, Error, Result};

/// Type-level analysis support. Presentation order and expansion state belong to the UI.
/// Overview is available for every loaded column.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalysisCapabilities {
    pub statistics: bool,
    pub distribution: bool,
    pub values: bool,
    pub lengths: bool,
    pub structure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisField {
    pub name: String,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

/// Row and logical-null counts in the requested view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnOverview {
    pub rows: u64,
    pub null_count: u64,
}

impl ArrayView<'_> {
    pub(crate) fn overview(&self, cancellation: &Cancellation) -> Result<ColumnOverview> {
        let null_count = match self.array.logical_nulls() {
            None => 0,
            Some(nulls) if matches!(self.rows, Selection::All(_)) => nulls.null_count() as u64,
            Some(nulls) => {
                let mut count = 0;
                for (i, row) in self.rows.iter().enumerate() {
                    cancellation.check_at(i)?;
                    count += u64::from(nulls.is_null(row));
                }
                count
            }
        };
        Ok(ColumnOverview {
            rows: self.len() as u64,
            null_count,
        })
    }
}

/// Type metadata only: capabilities never scan the column or compute cardinality.
pub fn capabilities(data_type: &DataType) -> AnalysisCapabilities {
    if !types::is_describable(data_type) {
        return AnalysisCapabilities::default();
    }
    AnalysisCapabilities {
        statistics: super::numeric::supports(data_type),
        distribution: super::supports_histogram(data_type),
        values: super::values::supports(data_type),
        lengths: super::lengths::unit(data_type).is_some(),
        structure: supports_structure(data_type),
    }
}

fn supports_structure(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::Map(..)
            | DataType::Dictionary(..)
    )
}

pub(crate) fn structure(data_type: &DataType) -> Result<Vec<AnalysisField>> {
    let field = |name: &str, data_type: &DataType, nullable: bool| -> Result<AnalysisField> {
        if !types::is_describable(data_type) {
            return Err(Error::Unsupported(format!(
                "unsupported nested type {data_type}"
            )));
        }
        Ok(AnalysisField {
            name: name.to_owned(),
            display_type: display_type(data_type),
            technical_type: data_type.to_string(),
            nullable,
        })
    };
    match data_type {
        DataType::Struct(fields) => fields
            .iter()
            .map(|f| field(f.name(), f.data_type(), f.is_nullable()))
            .collect(),
        DataType::List(f)
        | DataType::LargeList(f)
        | DataType::ListView(f)
        | DataType::LargeListView(f)
        | DataType::FixedSizeList(f, _)
        | DataType::Map(f, _) => Ok(vec![field(f.name(), f.data_type(), f.is_nullable())?]),
        DataType::Dictionary(key, value) => Ok(vec![
            field("Key", key, false)?,
            field("Value", value, true)?,
        ]),
        _ => Err(Error::InvalidArgument(format!(
            "{data_type} has no field structure"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{new_null_array, ArrayRef, DictionaryArray, Float64Array, Int8Array, StringArray},
        datatypes::{Field, Int8Type},
        record_batch::RecordBatch,
    };

    use super::*;
    use crate::{AxisScale, Cancellation, View};

    fn view(array: &ArrayRef) -> Arc<View> {
        let batch = RecordBatch::try_from_iter([("value", array.clone())]).unwrap();
        Arc::new(crate::Dataset {
            source_path: Default::default(),
            schema: batch.schema(),
            columns: vec![crate::Column::new("value", array.clone()).unwrap()],
            n_rows: array.len(),
            file_size: 0,
            format: crate::FileType::Csv,
            metadata: crate::FileMetadata::Delimited,
        })
        .view()
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
        let cancel = Cancellation::default();
        assert!(matches!(
            text.analyze_statistics(0, &cancel),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            number.analyze_lengths(0, 10, &cancel),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            binary.analyze_values(0, &cancel),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            text.dataset().column_structure(0),
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(text.analyze_overview(0, &cancel).unwrap().rows, 1);
        assert_eq!(
            number.analyze_statistics(0, &cancel).unwrap().mean,
            Some(1.0)
        );
    }

    #[test]
    fn requested_bins_apply_to_numeric_and_length_distributions() {
        let numbers: ArrayRef = Arc::new(Float64Array::from_iter_values((0..100).map(f64::from)));
        let text: ArrayRef = Arc::new(StringArray::from_iter_values(
            (0..100).map(|n| "x".repeat(n)),
        ));
        let numbers = view(&numbers);
        let text = view(&text);
        let cancel = Cancellation::default();
        assert!(matches!(
            numbers.analyze_distribution(0, 0, AxisScale::Linear, &cancel),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            text.analyze_lengths(0, 0, &cancel),
            Err(Error::InvalidArgument(_))
        ));
        for bins in [10, 20] {
            for histogram in [
                numbers
                    .analyze_distribution(0, bins, AxisScale::Linear, &cancel)
                    .unwrap(),
                text.analyze_lengths(0, bins, &cancel).unwrap().histogram,
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
        let cancellation = Cancellation::default();
        assert!(matches!(
            view(&text).analyze_distribution(0, 2, AxisScale::Linear, &cancellation),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            view(&dates).analyze_distribution(0, 2, AxisScale::Log10, &cancellation),
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(
            view(&dates)
                .analyze_distribution(0, 2, AxisScale::Linear, &cancellation)
                .unwrap()
                .total,
            2
        );
    }

    #[test]
    fn statistics_reports_non_finite_values_without_misleading_moments() {
        for values in [vec![Some(f64::INFINITY)], vec![
            Some(1.0),
            Some(f64::INFINITY),
            Some(f64::NAN),
            None,
        ]] {
            let array: ArrayRef = Arc::new(Float64Array::from(values));
            let statistics = view(&array)
                .analyze_statistics(0, &Cancellation::default())
                .unwrap();
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
        let ColumnOverview { rows, null_count } = view(&array)
            .analyze_overview(0, &Cancellation::default())
            .unwrap();
        assert_eq!((rows, null_count), (5, 2));
        let summary = view(&array)
            .analyze_values(0, &Cancellation::default())
            .unwrap();
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
        for (data_type, label) in types {
            let array = new_null_array(
                &DataType::Struct(vec![Field::new("child", data_type.clone(), false)].into()),
                0,
            );
            let fields = view(&array).dataset().column_structure(0).unwrap();
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
        for data_type in [
            DataType::List(field.clone()),
            DataType::Struct(vec![field.clone()].into()),
        ] {
            let support = capabilities(&data_type);
            assert!(support.structure);
            assert!(!support.statistics);
            assert!(!support.values);
            let array = new_null_array(&data_type, 0);
            let fields = view(&array).dataset().column_structure(0).unwrap();
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
}
