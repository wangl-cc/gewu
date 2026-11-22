//! Column analysis metadata.

use arrow::{array::Array, datatypes::DataType};

use crate::{col_view::ColView, stats::Result, types, types::display_type};

pub(crate) fn count_nulls(view: ColView<'_>) -> u64 {
    super::scan::null_count(view.array().logical_nulls().as_ref(), &view.selection())
}

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

/// Type metadata only: capabilities never scan the column or compute cardinality.
pub(crate) fn capabilities(data_type: &DataType) -> AnalysisCapabilities {
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
            return Err(crate::AnalysisError::UnsupportedDataType {
                operation: crate::AnalysisOperation::Structure,
                data_type: data_type.clone(),
            });
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
        _ => Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Structure,
            data_type: data_type.clone(),
        }),
    }
}

#[cfg(test)]
#[path = "analysis_tests.rs"]
mod tests;
