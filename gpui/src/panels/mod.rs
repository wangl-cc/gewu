//! Inspector and filter controls over the table's applied snapshot.
mod charts;
pub(crate) mod choice;
mod draft;
mod filter;
mod inspector;

#[cfg(test)]
pub(crate) use charts::scatter_data_bounds;
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
    style_control(control, label, ButtonTone::for_selection(accented), cx)
}

pub(crate) fn danger_button(
    id: impl Into<ElementId>,
    label: &'static str,
    cx: &App,
) -> base::Button {
    style_control(
        button_base(id).accessibility_label(label),
        label.into(),
        ButtonTone::Danger,
        cx,
    )
}

pub(crate) fn button_base(id: impl Into<ElementId>) -> base::Button {
    base::Button::new(id).styles(|styles| {
        styles.disabled(|style| {
            style
                .opacity(crate::theme::DISABLED_OPACITY)
                .cursor_default()
        })
    })
}

pub(in crate::panels) fn options_trigger(
    id: &'static str,
    active: bool,
    expanded: bool,
    cx: &App,
) -> base::Button {
    let colors = crate::theme::GewuPalette::get(cx);
    base::Button::new(id)
        .accessibility_label("Display options")
        .aria_expanded(expanded)
        .size(px(22.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .when(expanded, |b| b.bg(colors.button_secondary))
        .hover(move |s| s.bg(colors.button_secondary_hover))
        .active(move |s| s.bg(colors.secondary_pressed()))
        .focus_visible(move |s| s.shadow(colors.focus_shadows()))
        .child(inspector::controls::sliders_icon(if active || expanded {
            colors.accent
        } else {
            colors.muted_foreground
        }))
}

pub(crate) fn disclosure(
    id: &'static str,
    label: &'static str,
    expanded: bool,
    cx: &App,
) -> base::Button {
    let colors = crate::theme::GewuPalette::get(cx);
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

pub(crate) fn input_frame<T: Styled + InteractiveElement + FluentBuilder>(
    frame: T,
    enabled: bool,
    focus: &FocusHandle,
    window: &Window,
    cx: &App,
) -> T {
    let colors = crate::theme::GewuPalette::get(cx);
    let focused = enabled && window.is_window_active() && focus.is_focused(window);
    frame
        .h(px(28.))
        .min_w_0()
        .rounded(px(6.))
        .bg(colors.canvas)
        .border(px(1. / window.scale_factor()))
        .border_color(colors.border)
        .text_size(px(13.))
        .text_color(colors.foreground)
        .when(enabled, |s| {
            s.hover(move |s| s.border_color(colors.input_hover()))
        })
        .when(!enabled, |s| {
            s.opacity(crate::theme::DISABLED_OPACITY).cursor_default()
        })
        .when(focused, |s| s.shadow(colors.focus_shadows()))
}

pub(crate) fn elevated<T: Styled>(surface: T, window: &Window, cx: &App) -> T {
    let colors = crate::theme::GewuPalette::get(cx);
    surface
        .rounded(px(10.))
        .border(px(1. / window.scale_factor()))
        .border_color(colors.elevated_border)
        .bg(colors.elevated)
        .text_color(colors.foreground)
        .shadow(colors.elevated_shadows())
}

pub(crate) fn segmented_tabs(
    id: &'static str,
    label: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> base::Tabs {
    segment_track(id.into(), base::Tabs::new(id).aria_label(label), window, cx)
}

pub(crate) fn segmented_radios(
    id: impl Into<ElementId>,
    label: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> base::RadioGroup {
    let id = id.into();
    let radios = base::RadioGroup::new(id.clone())
        .axis(Axis::Horizontal)
        .aria_label(label);
    segment_track(id, radios, window, cx)
}

fn segment_track<T: Styled + InteractiveElement + FluentBuilder>(
    id: ElementId,
    track: T,
    window: &mut Window,
    cx: &mut App,
) -> T {
    let colors = crate::theme::GewuPalette::get(cx);
    // The tab or radio owns keyboard focus. The group paints a single ring around its children.
    let focus = window.use_keyed_state(id, cx, |_, cx| cx.focus_handle().tab_stop(false));
    let focus = focus.read(cx);
    let focused = window.is_window_active()
        && window.last_input_was_keyboard()
        && focus.contains_focused(window, cx);
    track
        .flex()
        .items_center()
        .flex_none()
        .h(px(28.))
        .p(px(2.))
        .rounded(px(6.))
        .bg(colors.segment_track)
        .track_focus(focus)
        .when(focused, |s| s.shadow(colors.focus_shadows()))
}

pub(crate) fn segment<T: Styled + StatefulInteractiveElement + ParentElement + FluentBuilder>(
    control: T,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> T {
    let colors = crate::theme::GewuPalette::get(cx);
    control
        .flex_1()
        .min_w_0()
        .flex()
        .justify_center()
        .items_center()
        .h_full()
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(colors.foreground)
        .when(selected, |c| {
            c.bg(colors.segment_thumb)
                .shadow_xs()
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!selected, |c| {
            c.hover(move |s| s.bg(colors.button_secondary_hover))
                .active(move |s| s.bg(colors.secondary_pressed()))
        })
        .child(label.into())
}

pub(crate) fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    cx: &App,
    activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> base::Checkbox {
    let colors = crate::theme::GewuPalette::get(cx);
    let label = label.into();
    base::Checkbox::new(id)
        .accessibility_label(label.clone())
        .checked(checked)
        .styles(|styles| {
            styles.disabled(|style| {
                style
                    .opacity(crate::theme::DISABLED_OPACITY)
                    .cursor_default()
            })
        })
        .group("checkbox")
        .cursor_pointer()
        .focus_visible(move |s| s.shadow(colors.focus_shadows()))
        .on_change(move |_, ev, w, cx| activate(ev, w, cx))
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .child(
            div()
                .id("checkbox-mark")
                .flex_none()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if checked {
                    colors.accent
                } else {
                    colors.muted_foreground
                })
                .bg(if checked {
                    colors.accent
                } else {
                    colors.canvas
                })
                .group_hover("checkbox", move |s| {
                    if checked {
                        s.bg(colors.accent_hover).border_color(colors.accent_hover)
                    } else {
                        s.border_color(colors.foreground)
                    }
                })
                .group_active("checkbox", move |s| {
                    if checked {
                        s.bg(colors.accent_pressed())
                    } else {
                        s.bg(colors.button_secondary_hover)
                    }
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
                                    window.paint_path(path, colors.accent_foreground);
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

enum ButtonTone {
    Secondary,
    Primary,
    Danger,
}

impl ButtonTone {
    fn for_selection(selected: bool) -> Self {
        if selected {
            Self::Primary
        } else {
            Self::Secondary
        }
    }
}

fn style_control<T: Styled + StatefulInteractiveElement + ParentElement>(
    control: T,
    label: SharedString,
    tone: ButtonTone,
    cx: &App,
) -> T {
    let colors = crate::theme::GewuPalette::get(cx);
    let (fill, ink, hover, pressed) = match tone {
        ButtonTone::Secondary => (
            colors.button_secondary,
            colors.foreground,
            colors.button_secondary_hover,
            colors.secondary_pressed(),
        ),
        ButtonTone::Primary => (
            colors.accent,
            colors.accent_foreground,
            colors.accent_hover,
            colors.accent_pressed(),
        ),
        ButtonTone::Danger => (
            colors.danger_fill,
            colors.danger_fill_foreground,
            colors.danger_hover(),
            colors.danger_pressed(),
        ),
    };
    control
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .h(px(28.))
        .px_2()
        .rounded(px(6.))
        .cursor_pointer()
        .text_size(px(13.))
        .whitespace_nowrap()
        .bg(fill)
        .text_color(ink)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .focus_visible(move |s| s.shadow(colors.focus_shadows()))
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
