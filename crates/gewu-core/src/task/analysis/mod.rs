//! On-demand column analysis over an immutable view snapshot. Jobs own their
//! cancellation state; cancelling an inspector never cancels a table query.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
};

use crate::{
    column::{format, stats},
    scope::Selection,
    types::Type,
    DistinctValue, Error, Histogram, LogicalType, Result,
};

mod lengths;
mod median;
pub(crate) mod numeric;
mod values;

use numeric::Numeric;

/// Facts about one column over the rows of the current view (so explicit
/// filters apply), for aggregate reporting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnSummary {
    /// Rows in the current view.
    pub rows: u64,
    pub null_count: u64,
    pub nan_count: u64,
    /// Distinct non-null values.
    pub distinct_count: u64,
    /// Numbers, dates and times only (dates and times as their physical
    /// value; see `Session::format_value`). These f64 results can approximate
    /// large integers and decimals; distinct counts use the native values.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Smallest and largest non-null values of any comparable column, as
    /// the table displays them.
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    /// Other columns: the most common values, most common first.
    pub top_values: Vec<DistinctValue>,
}

pub use crate::types::{AnalysisSection, LengthUnit};

#[derive(Debug, Clone)]
pub struct NumericStatistics {
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    /// Exact native median text. Nulls and NaNs are excluded. Opposite infinities
    /// as the two central values have no median. Even temporal medians retain
    /// fractional native units rather than rounding back to the column's unit.
    pub median_label: Option<String>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub nan_count: u64,
    pub infinite_count: u64,
}

#[derive(Debug, Clone)]
pub struct ValueSummary {
    /// Non-null distinct values, estimated with a fixed-capacity sketch.
    pub distinct_count: u64,
    /// Applies to distinct cardinality and candidate selection. Counts recount
    /// candidates over all rows; 64-bit hash collisions may merge values.
    pub estimated: bool,
    pub values: Vec<DistinctValue>,
}

#[derive(Debug, Clone)]
pub struct LengthSummary {
    pub unit: LengthUnit,
    pub empty_count: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub mean: Option<f64>,
    pub histogram: Histogram,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisField {
    pub name: String,
    pub logical_type: LogicalType,
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

#[derive(Default)]
pub(crate) struct AnalysisContext {
    cancelled: AtomicBool,
}

impl AnalysisContext {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn check_at(&self, index: usize) -> Result<()> {
        if index.is_multiple_of(4096) {
            self.check()?;
        }
        Ok(())
    }
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
    context: AnalysisContext,
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
            context: AnalysisContext::default(),
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
                || !Type::from_arrow(self.array.data_type()).is_some_and(|ty| ty.is_numeric()))
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
        let data = match self.section {
            AnalysisSection::Overview => AnalysisData::Overview {
                rows: self.rows.len() as u64,
                null_count: self.null_count()?,
            },
            AnalysisSection::Statistics => {
                let mut summary = ColumnSummary {
                    rows: self.rows.len() as u64,
                    ..Default::default()
                };
                let details = Numeric::new(&self.array)
                    .expect("validated statistics capability")
                    .summarize(self.rows.iter(), &mut summary, false, &self.context)?;
                let formatter = format::formatter(self.array.as_ref())?;
                let label = |row: Option<usize>| {
                    row.and_then(|row| format::cell(self.array.as_ref(), &formatter, row))
                };
                AnalysisData::Statistics {
                    statistics: NumericStatistics {
                        min_label: label(details.min_row),
                        max_label: label(details.max_row),
                        median_label: median::label(&self.array, &self.rows, &self.context)?,
                        mean: summary.mean.filter(|v| v.is_finite()),
                        std_dev: summary
                            .std_dev
                            .filter(|v| v.is_finite() && details.infinite_count == 0),
                        nan_count: summary.nan_count,
                        infinite_count: details.infinite_count,
                    },
                }
            }
            AnalysisSection::Distribution => AnalysisData::Distribution {
                histogram: stats::counts_scaled_with_context(
                    &self.array,
                    &self.rows,
                    self.histogram_bins,
                    self.histogram_scale,
                    &self.context,
                )?,
            },
            AnalysisSection::Values => AnalysisData::Values {
                summary: values::analyze(&self.array, &self.rows, &self.context)?,
            },
            AnalysisSection::Lengths => AnalysisData::Lengths {
                summary: lengths::analyze(
                    &self.array,
                    &self.rows,
                    self.histogram_bins,
                    &self.context,
                )?,
            },
            AnalysisSection::Structure => AnalysisData::Structure {
                fields: structure(self.array.data_type()),
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
pub(crate) fn capabilities(data_type: &DataType) -> Vec<AnalysisSection> {
    Type::from_arrow(data_type).map_or_else(
        || vec![AnalysisSection::Overview],
        |ty| ty.analysis_sections(),
    )
}

fn structure(data_type: &DataType) -> Vec<AnalysisField> {
    let field = |f: &arrow::datatypes::Field| {
        let logical = LogicalType::from_arrow(f.data_type());
        AnalysisField {
            name: f.name().clone(),
            display_type: logical.display_name(),
            technical_type: f.data_type().to_string(),
            logical_type: logical,
            nullable: f.is_nullable(),
        }
    };
    match data_type {
        DataType::Struct(fields) => fields.iter().map(|f| field(f)).collect(),
        DataType::List(f)
        | DataType::LargeList(f)
        | DataType::ListView(f)
        | DataType::LargeListView(f)
        | DataType::FixedSizeList(f, _)
        | DataType::Map(f, _) => vec![field(f)],
        DataType::Dictionary(key, value) => {
            let key_logical = LogicalType::from_arrow(key);
            let value_logical = LogicalType::from_arrow(value);
            vec![
                AnalysisField {
                    name: "Key".into(),
                    display_type: key_logical.display_name(),
                    technical_type: key.to_string(),
                    logical_type: key_logical,
                    nullable: false,
                },
                AnalysisField {
                    name: "Value".into(),
                    display_type: value_logical.display_name(),
                    technical_type: value.to_string(),
                    logical_type: value_logical,
                    nullable: true,
                },
            ]
        }
        _ => Vec::new(),
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
                .map(|v| (v.label.as_deref(), v.count))
                .collect::<Vec<_>>(),
            [(Some("a"), 2), (Some("b"), 1)]
        );
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
            assert_eq!(fields[0].logical_type, LogicalType::Integer {
                bits: 32,
                signed: true
            });
        }
    }
}
