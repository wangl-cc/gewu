//! Inspector and filter controls over the table's applied snapshot.
mod charts;
mod draft;
mod filter;
mod inspector;

pub(crate) use filter::{FilterEditor, FilterEvent};
use gpui_kit::{base, *};
pub(crate) use inspector::{Inspector, Page as InspectorPage};

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    accented: bool,
    cx: &App,
) -> base::Button {
    let label = label.into();
    let control = base::Button::new(id).accessibility_label(label.clone());
    style_control(control, label, accented, cx)
}

fn tab(id: &'static str, selected: bool, cx: &App) -> base::Tab {
    style_control(
        base::Tab::new(id)
            .selected(selected)
            .accessibility_label(id),
        id.into(),
        selected,
        cx,
    )
}

pub(crate) fn radio_group(id: &'static str, label: &'static str) -> base::RadioGroup {
    base::RadioGroup::new(id)
        .axis(Axis::Horizontal)
        .aria_label(label)
        .flex()
        .items_center()
        .flex_wrap()
        .gap_1()
}

pub(crate) fn radio(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    cx: &App,
    activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> base::Radio {
    let label = label.into();
    let control = base::Radio::new(id)
        .checked(checked)
        .accessibility_label(label.clone())
        .on_change(move |_, ev, w, cx| activate(ev, w, cx))
        .flex()
        .items_center()
        .justify_center()
        .line_height(relative(1.));
    style_control(control, label, checked, cx)
}

pub(crate) fn toggle(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    pressed: bool,
    cx: &App,
    activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> base::Toggle {
    let label = label.into();
    let control = base::Toggle::new(id)
        .pressed(pressed)
        .accessibility_label(label.clone())
        .on_change(move |_, ev, w, cx| activate(ev, w, cx));
    style_control(control, label, pressed, cx)
}

fn style_control<T: Styled + StatefulInteractiveElement + ParentElement>(
    control: T,
    label: SharedString,
    accented: bool,
    cx: &App,
) -> T {
    let colors = base::Theme::global(cx).tokens.colors;
    control
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .bg(if accented {
            colors.primary
        } else {
            colors.secondary
        })
        .text_color(if accented {
            colors.primary_foreground
        } else {
            colors.secondary_foreground
        })
        .hover(move |style| {
            style.bg(if accented {
                colors.primary.opacity(0.86)
            } else {
                colors.accent
            })
        })
        .child(label)
}

fn value_preview(value: &str) -> SharedString {
    if value.is_empty() {
        "(empty text)".into()
    } else {
        crate::data::preview_with_limit(value, 96)
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
        FilterCondition::Exact { values, exclude } => {
            let preview = values
                .iter()
                .take(3)
                .map(|s| value_preview(s).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            if values.is_empty() {
                (if *exclude { "All values" } else { "No values" }).to_owned()
            } else {
                format!(
                    "{} {preview}{}",
                    if *exclude { "Exclude" } else { "Include" },
                    if values.len() > 3 {
                        format!(" and {} more", values.len() - 3)
                    } else {
                        String::new()
                    }
                )
            }
        }
    };
    format!(
        "{summary} · {} missing",
        if missing { "include" } else { "exclude" }
    )
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
