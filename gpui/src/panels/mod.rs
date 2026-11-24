//! Inspector and filter controls over the table's applied snapshot.
mod charts;
pub(crate) mod choice;
mod draft;
mod filter;
mod inspector;

pub(crate) use filter::{FilterEditor, FilterEvent};
use gpui_kit::{base, prelude::FluentBuilder, *};
pub(crate) use inspector::{Inspector, Page as InspectorPage};

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    accented: bool,
    cx: &App,
) -> base::Button {
    let label = label.into();
    let control = button_base(id).accessibility_label(label.clone());
    style_control(control, label, accented, cx)
}

pub(crate) fn button_base(id: impl Into<ElementId>) -> base::Button {
    base::Button::new(id)
        .styles(|styles| styles.disabled(|style| style.opacity(0.45).cursor_default()))
}

fn display_options(id: &'static str, expanded: bool, cx: &App) -> base::Button {
    disclosure(id, "Chart display options", expanded, cx)
}

pub(crate) fn disclosure(
    id: &'static str,
    label: &'static str,
    expanded: bool,
    cx: &App,
) -> base::Button {
    let colors = base::Theme::global(cx).tokens.colors;
    base::Button::new(id)
        .accessibility_label(label)
        .aria_expanded(expanded)
        .flex()
        .items_center()
        .justify_start()
        .gap_1()
        .text_xs()
        .text_color(colors.muted_foreground)
        .cursor_pointer()
        .child(inspector::controls::chevron(
            expanded,
            colors.muted_foreground,
        ))
        .child(label)
}

pub(crate) fn segmented_tabs(id: &'static str, label: &'static str, cx: &App) -> base::Tabs {
    let colors = base::Theme::global(cx).tokens.colors;
    base::Tabs::new(id)
        .aria_label(label)
        .flex()
        .items_center()
        .flex_none()
        .p(px(2.))
        .rounded_md()
        .bg(colors.muted)
}

pub(crate) fn segment<T: Styled + StatefulInteractiveElement + ParentElement + FluentBuilder>(
    control: T,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> T {
    let colors = base::Theme::global(cx).tokens.colors;
    control
        .flex_1()
        .min_w_0()
        .flex()
        .justify_center()
        .items_center()
        .h(px(23.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(colors.foreground)
        .when(selected, |c| {
            c.bg(colors.background)
                .shadow_xs()
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!selected, |c| c.hover(move |s| s.bg(colors.accent)))
        .child(label.into())
}

pub(crate) fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    cx: &App,
    activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> base::Checkbox {
    let colors = base::Theme::global(cx).tokens.colors;
    let label = label.into();
    base::Checkbox::new(id)
        .accessibility_label(label.clone())
        .checked(checked)
        .on_change(move |_, ev, w, cx| activate(ev, w, cx))
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .text_size(px(12.))
        .child(
            div()
                .flex_none()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if checked {
                    colors.primary
                } else {
                    colors.muted_foreground
                })
                .bg(if checked {
                    colors.primary
                } else {
                    colors.background
                })
                .when(checked, |box_| {
                    box_.child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                let p = |x, y| bounds.origin + point(px(x), px(y));
                                let mut mark = PathBuilder::stroke(px(1.6));
                                mark.move_to(p(1., 5.));
                                mark.line_to(p(4., 8.));
                                mark.line_to(p(10., 1.5));
                                if let Ok(path) = mark.build() {
                                    window.paint_path(path, colors.primary_foreground);
                                }
                            },
                        )
                        .w(px(11.))
                        .h(px(10.)),
                    )
                }),
        )
        .child(div().min_w_0().truncate().child(label))
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
                crate::theme::hover_fill(colors.primary, colors.primary_foreground)
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
