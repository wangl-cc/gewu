//! On-demand column analysis over an immutable view snapshot. Jobs own their
//! cancellation state; cancelling an inspector never cancels a table query.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
};

pub use super::LengthUnit;
use super::{ArrayView, Cancellation, Histogram, LengthSummary, NumericStatistics, ValueSummary};
use crate::{column::display_type, scope::Selection, types, Error, Result};

/// A section of the column inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisSection {
    Overview,
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisField {
    pub name: String,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub enum AnalysisData {
    Overview { rows: u64, null_count: u64 },
    Statistics { statistics: NumericStatistics },
    Distribution { histogram: Histogram },
    Values { summary: ValueSummary },
    Lengths { summary: LengthSummary },
    Structure { fields: Vec<AnalysisField> },
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub version: u64,
    pub column: u32,
    pub data: AnalysisData,
}

/// A read-only job for exactly one section. Array buffers and source row IDs
/// are shared, not copied. Results keep the captured version even if the table
/// subsequently changes; clients must reject results for an obsolete selection.
pub struct ColumnAnalysis {
    array: ArrayRef,
    rows: Selection,
    version: u64,
    column: u32,
    section: AnalysisSection,
    histogram_bins: u32,
    histogram_scale: crate::AxisScale,
    context: Cancellation,
}

impl ColumnAnalysis {
    pub(crate) fn new(
        array: ArrayRef,
        rows: Arc<Vec<u32>>,
        version: u64,
        column: u32,
        section: AnalysisSection,
    ) -> Result<Self> {
        if !capabilities(array.data_type()).contains(&section) {
            return Err(Error::InvalidArgument(format!(
                "{} does not support {section:?} analysis",
                array.data_type()
            )));
        }
        // All analysis sections are order-independent. A full view can use
        // Arrow's cached null count and contiguous buffers even after sorting.
        let rows = if rows.len() == array.len() {
            Selection::All(array.len())
        } else {
            Selection::Rows(rows)
        };
        Ok(Self {
            array,
            rows,
            version,
            column,
            section,
            histogram_bins: 40,
            histogram_scale: crate::AxisScale::Linear,
            context: Cancellation::default(),
        })
    }

    pub fn cancel(&self) {
        self.context.cancel();
    }

    /// Set the maximum bin count for distribution and length histograms.
    /// The default is 40; discrete data may need fewer bins. Zero is rejected.
    pub fn with_histogram_bins(mut self, bins: u32) -> Result<Self> {
        if bins == 0 {
            return Err(Error::InvalidArgument("bins must be at least 1".into()));
        }
        self.histogram_bins = bins;
        Ok(self)
    }

    /// Choose numeric distribution binning; other sections remain untransformed.
    pub fn with_histogram_scale(mut self, scale: crate::AxisScale) -> Result<Self> {
        if scale == crate::AxisScale::Log10
            && (self.section != AnalysisSection::Distribution
                || !super::supports_logarithmic(self.array.data_type()))
        {
            return Err(Error::InvalidArgument(
                "logarithmic binning requires a numeric distribution".into(),
            ));
        }
        self.histogram_scale = scale;
        Ok(self)
    }

    /// Execute off the UI thread. Only the requested section is computed.
    /// Cancellation is terminal for this job and checked throughout scans.
    pub fn run(&self) -> Result<AnalysisResult> {
        self.context.check()?;
        let view = ArrayView::new(&self.array, &self.rows);
        let data = match self.section {
            AnalysisSection::Overview => AnalysisData::Overview {
                rows: view.len() as u64,
                null_count: self.null_count()?,
            },
            AnalysisSection::Statistics => AnalysisData::Statistics {
                statistics: view.numeric_statistics(&self.context)?,
            },
            AnalysisSection::Distribution => AnalysisData::Distribution {
                histogram: view.histogram(
                    self.histogram_bins,
                    self.histogram_scale,
                    &self.context,
                )?,
            },
            AnalysisSection::Values => AnalysisData::Values {
                summary: view.value_summary(&self.context)?,
            },
            AnalysisSection::Lengths => AnalysisData::Lengths {
                summary: view.length_summary(self.histogram_bins, &self.context)?,
            },
            AnalysisSection::Structure => AnalysisData::Structure {
                fields: structure(self.array.data_type())?,
            },
        };
        self.context.check()?;
        Ok(AnalysisResult {
            version: self.version,
            column: self.column,
            data,
        })
    }

    fn null_count(&self) -> Result<u64> {
        let Some(nulls) = self.array.logical_nulls() else {
            return Ok(0);
        };
        if matches!(self.rows, Selection::All(_)) {
            return Ok(nulls.null_count() as u64);
        }
        let mut count = 0;
        for (i, row) in self.rows.iter().enumerate() {
            self.context.check_at(i)?;
            count += u64::from(nulls.is_null(row));
        }
        Ok(count)
    }
}

/// Type metadata only: capabilities never scan the column or compute cardinality.
pub fn capabilities(data_type: &DataType) -> Vec<AnalysisSection> {
    use AnalysisSection::*;
    let mut sections = vec![Overview];
    if !crate::types::is_describable(data_type) {
        return sections;
    }
    if super::numeric::supports(data_type) {
        sections.push(Statistics);
    }
    if super::supports_histogram(data_type) {
        sections.push(Distribution);
    }
    if super::values::supports(data_type) {
        sections.push(Values);
    }
    if super::lengths::unit(data_type).is_some() {
        sections.push(Lengths);
    }
    if matches!(
        data_type,
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::Map(..)
            | DataType::Dictionary(..)
    ) {
        sections.push(Structure);
    }
    sections
}

fn structure(data_type: &DataType) -> Result<Vec<AnalysisField>> {
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
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use arrow::{
        array::{new_null_array, DictionaryArray, Float64Array, Int8Array, StringArray},
        datatypes::{Field, Int8Type},
    };

    use super::*;

    fn run(array: &ArrayRef, section: AnalysisSection) -> AnalysisData {
        ColumnAnalysis::new(
            array.clone(),
            Arc::new((0..array.len() as u32).collect()),
            1,
            0,
            section,
        )
        .unwrap()
        .run()
        .unwrap()
        .data
    }

    #[test]
    fn analysis_sections_preserve_presentation_order() {
        use arrow::datatypes::Fields;
        use AnalysisSection::*;
        let item = |data_type| Arc::new(Field::new("item", data_type, true));
        let dictionary = |value| DataType::Dictionary(Box::new(DataType::Int32), Box::new(value));
        for (data_type, expected) in [
            (DataType::Int32, vec![
                Overview,
                Statistics,
                Distribution,
                Values,
            ]),
            (DataType::Utf8, vec![Overview, Values, Lengths]),
            (DataType::Binary, vec![Overview, Lengths]),
            (DataType::List(item(DataType::Int32)), vec![
                Overview, Lengths, Structure,
            ]),
            (
                DataType::Struct(Fields::from(vec![Field::new("a", DataType::Int32, true)])),
                vec![Overview, Structure],
            ),
            (dictionary(DataType::Int64), vec![
                Overview, Values, Structure,
            ]),
            (DataType::Null, vec![Overview]),
        ] {
            assert_eq!(capabilities(&data_type), expected, "{data_type}");
        }
    }

    #[test]
    fn requested_bins_apply_to_numeric_and_length_distributions() {
        let numbers: ArrayRef = Arc::new(Float64Array::from_iter_values((0..100).map(f64::from)));
        let text: ArrayRef = Arc::new(StringArray::from_iter_values(
            (0..100).map(|n| "x".repeat(n)),
        ));
        for (array, section) in [
            (numbers, AnalysisSection::Distribution),
            (text, AnalysisSection::Lengths),
        ] {
            let job = || {
                ColumnAnalysis::new(array.clone(), Arc::new((0..100).collect()), 1, 0, section)
                    .unwrap()
            };
            assert!(matches!(
                job().with_histogram_bins(0),
                Err(Error::InvalidArgument(_))
            ));
            for bins in [10, 20] {
                let result = job().with_histogram_bins(bins).unwrap().run().unwrap();
                let histogram = match result.data {
                    AnalysisData::Distribution { histogram } => histogram,
                    AnalysisData::Lengths { summary } => summary.histogram,
                    _ => panic!("histogram"),
                };
                assert_eq!(histogram.counts.len(), bins as usize);
                assert_eq!(histogram.counts.iter().sum::<u64>(), 100);
            }
        }
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
            let AnalysisData::Statistics { statistics } = run(&array, AnalysisSection::Statistics)
            else {
                panic!("statistics")
            };
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
        let AnalysisData::Overview { rows, null_count } = run(&array, AnalysisSection::Overview)
        else {
            panic!("overview")
        };
        assert_eq!((rows, null_count), (5, 2));
        let AnalysisData::Values { summary } = run(&array, AnalysisSection::Values) else {
            panic!("values")
        };
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
            let AnalysisData::Structure { fields } = run(&array, AnalysisSection::Structure) else {
                panic!("structure")
            };
            assert_eq!(fields, vec![AnalysisField {
                name: "child".into(),
                display_type: label.into(),
                technical_type: data_type.to_string(),
                nullable: false,
            }]);
        }
    }

    #[test]
    fn structural_sections_use_type_metadata_and_never_offer_scalar_statistics() {
        let field = Arc::new(Field::new("child", DataType::Int32, true));
        for data_type in [
            DataType::List(field.clone()),
            DataType::Struct(vec![field.clone()].into()),
        ] {
            let sections = capabilities(&data_type);
            assert!(sections.contains(&AnalysisSection::Structure));
            assert!(!sections.contains(&AnalysisSection::Statistics));
            assert!(!sections.contains(&AnalysisSection::Values));
            let array = new_null_array(&data_type, 0);
            let AnalysisData::Structure { fields } = run(&array, AnalysisSection::Structure) else {
                panic!("structure")
            };
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
