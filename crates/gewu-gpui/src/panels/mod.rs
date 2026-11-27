//! Inspector and filter controls over the table's applied snapshot.
mod charts;
pub(crate) mod choice;
mod draft;
mod filter;
mod inspector;

#[cfg(test)]
pub(crate) use charts::scatter_data_bounds;
pub(crate) use filter::{FilterEditor, FilterEvent};
use gpui_kit::*;
pub(crate) use inspector::{Inspector, Page as InspectorPage};

fn value_preview(value: &str) -> SharedString {
    if value.is_empty() {
        "(empty text)".into()
    } else {
        crate::table::data::preview_with_limit(value, 96)
    }
}

pub(crate) fn filter_description(filter: &gewu_core::Filter) -> String {
    use gewu_core::{Filter, FilterCondition, TextMatch};
    let (condition, missing) = match filter {
        Filter::OnlyNulls => return "Only missing values".into(),
        Filter::NotNull => return "Exclude missing values".into(),
        Filter::Condition {
            condition,
            include_nulls,
        } => (condition, *include_nulls),
    };
    let summary = match condition {
        FilterCondition::Range { min, max } => match (min, max) {
            (Some(a), Some(b)) => format!("{} to {} inclusive", value_preview(a), value_preview(b)),
            (Some(a), None) => format!("At least {}", value_preview(a)),
            (None, Some(b)) => format!("At most {}", value_preview(b)),
            (None, None) => "All values".into(),
        },
        FilterCondition::Text {
            text,
            mode,
            case_sensitive,
        } => format!(
            "{} {}{}",
            match mode {
                TextMatch::Contains => "Contains",
                TextMatch::Equals => "Equals",
            },
            value_preview(text),
            if *case_sensitive {
                " (case sensitive)"
            } else {
                ""
            }
        ),
        FilterCondition::Exact { values, exclude } => exact_description(values.iter(), *exclude),
    };
    format!(
        "{summary} · {} missing",
        if missing { "include" } else { "exclude" }
    )
}

fn exact_description<'a>(
    values: impl ExactSizeIterator<Item = &'a String>,
    exclude: bool,
) -> String {
    let count = values.len();
    if count == 0 {
        return (if exclude { "All values" } else { "No values" }).to_owned();
    }
    let preview = values
        .take(3)
        .map(|value| value_preview(value).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let action = if exclude { "Exclude" } else { "Include" };
    if count > 3 {
        format!("{action} {preview} and {} more", count - 3)
    } else {
        format!("{action} {preview}")
    }
}

fn row() -> Div {
    div().flex().items_center().flex_wrap().gap_1()
}

fn section(label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(label.into()))
}

#[cfg(test)]
mod tests;
