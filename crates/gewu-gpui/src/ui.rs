//! Shared low-level UI primitives styled with `GewuPalette`.
use gpui_kit::{
    assets::IconName,
    base::{
        self,
        input::{Input, InputState},
    },
    component::{Icon, tooltip::Tooltip},
    prelude::FluentBuilder,
    *,
};

#[derive(Clone, Copy)]
pub(crate) enum ChevronDirection {
    Down,
    Right,
    Left,
}

#[derive(Clone, Copy)]
enum ButtonTone {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

enum ButtonSize {
    Regular,
    Compact,
    Status,
}

fn button_chrome(
    id: impl Into<ElementId>,
    tone: ButtonTone,
    active: bool,
    disabled: bool,
    cx: &App,
) -> base::Button {
    let colors = crate::theme::GewuPalette::get(cx);
    let (fill, ink, hover, pressed) = match tone {
        ButtonTone::Primary => (
            Some(colors.accent),
            colors.accent_foreground,
            colors.accent_hover,
            colors.accent_pressed(),
        ),
        ButtonTone::Secondary => (
            Some(if active {
                colors.button_secondary_hover
            } else {
                colors.button_secondary
            }),
            colors.foreground,
            colors.button_secondary_hover,
            colors.secondary_pressed(),
        ),
        ButtonTone::Ghost => (
            active.then_some(colors.button_secondary),
            colors.foreground,
            colors.button_secondary_hover,
            colors.secondary_pressed(),
        ),
        ButtonTone::Danger => (
            Some(colors.danger_fill),
            colors.danger_fill_foreground,
            colors.danger_hover(),
            colors.danger_pressed(),
        ),
    };
    base::Button::new(id)
        .styles(|styles| {
            styles.disabled(|style| {
                style
                    .opacity(crate::theme::DISABLED_OPACITY)
                    .cursor_default()
            })
        })
        .disabled(disabled)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .when_some(fill, |button, bg| button.bg(bg))
        .text_color(ink)
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(pressed))
                .focus_visible(move |s| s.shadow(colors.focus_shadows()))
        })
}

fn labeled_button(
    id: impl Into<ElementId>,
    label: SharedString,
    tone: ButtonTone,
    size: ButtonSize,
    active: bool,
    disabled: bool,
    cx: &App,
) -> base::Button {
    let control = button_chrome(id, tone, active, disabled, cx)
        .accessibility_label(label.clone())
        .whitespace_nowrap();
    let control = match size {
        ButtonSize::Regular => control.h(px(28.)).px_2().rounded(px(6.)).text_size(px(13.)),
        ButtonSize::Compact => control
            .h(px(22.))
            .px(px(8.))
            .rounded(px(5.))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM),
        ButtonSize::Status => control
            .h(px(19.))
            .px(px(6.))
            .rounded(px(4.))
            .text_size(px(11.))
            .font_features(crate::theme::tabular_digits()),
    };
    control.child(label)
}

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    accented: bool,
    cx: &App,
) -> base::Button {
    let tone = if accented {
        ButtonTone::Primary
    } else {
        ButtonTone::Secondary
    };
    labeled_button(
        id,
        label.into(),
        tone,
        ButtonSize::Regular,
        false,
        false,
        cx,
    )
}

pub(crate) fn danger_button(
    id: impl Into<ElementId>,
    label: &'static str,
    cx: &App,
) -> base::Button {
    labeled_button(
        id,
        label.into(),
        ButtonTone::Danger,
        ButtonSize::Regular,
        false,
        false,
        cx,
    )
}

pub(crate) fn compact_button(
    id: impl Into<ElementId>,
    label: &'static str,
    disabled: bool,
    cx: &App,
) -> base::Button {
    labeled_button(
        id,
        label.into(),
        ButtonTone::Secondary,
        ButtonSize::Compact,
        false,
        disabled,
        cx,
    )
}

pub(crate) fn status_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> base::Button {
    labeled_button(
        id,
        label.into(),
        ButtonTone::Secondary,
        ButtonSize::Status,
        active,
        false,
        cx,
    )
}

pub(crate) fn ghost_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    disabled: bool,
    cx: &App,
) -> base::Button {
    button_chrome(id, ButtonTone::Ghost, active, disabled, cx).accessibility_label(label.into())
}

pub(crate) fn options_trigger(
    id: &'static str,
    active: bool,
    expanded: bool,
    cx: &App,
) -> base::Button {
    let colors = crate::theme::GewuPalette::get(cx);
    ghost_button(id, "Display options", expanded, false, cx)
        .aria_expanded(expanded)
        .size(px(22.))
        .rounded(px(5.))
        .child(sliders_icon(if active || expanded {
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
    let direction = if expanded {
        ChevronDirection::Down
    } else {
        ChevronDirection::Right
    };
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
        .child(chevron(direction, colors.muted_foreground))
        .child(label)
}

pub(crate) fn chevron(direction: ChevronDirection, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut path = PathBuilder::stroke(px(1.3));
            let vertices = match direction {
                ChevronDirection::Down => [(-3., -1.5), (0., 1.5), (3., -1.5)],
                ChevronDirection::Right => [(-1.5, -3.), (1.5, 0.), (-1.5, 3.)],
                ChevronDirection::Left => [(1.5, -3.), (-1.5, 0.), (1.5, 3.)],
            };
            let [a, b, c] = vertices.map(|(x, y)| center + point(px(x), px(y)));
            path.move_to(a);
            path.line_to(b);
            path.line_to(c);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(12.))
    .flex_none()
}

pub(crate) fn sliders_icon(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            for (y, knob_x) in [(3.5, 2.75), (8.375, 6.75)] {
                window.paint_quad(
                    fill(
                        Bounds::new(o + point(px(1.5), px(y)), size(px(10.), px(1.25))),
                        color,
                    )
                    .corner_radii(px(0.625)),
                );
                window.paint_quad(
                    fill(
                        Bounds::new(o + point(px(knob_x), px(y - 1.125)), size(px(3.5), px(3.5))),
                        color,
                    )
                    .corner_radii(px(1.75)),
                );
            }
        },
    )
    .size(px(13.))
    .flex_none()
}

pub(crate) fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
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

pub(crate) fn search_input(
    id: impl Into<ElementId>,
    input: &Entity<InputState>,
    window: &Window,
    cx: &App,
) -> base::ObservedElement<Stateful<Div>> {
    use base::TestSupportExt;

    let colors = crate::theme::GewuPalette::get(cx);
    input_frame(div().id(id), true, &input.focus_handle(cx), window, cx)
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .child(
            Icon::new(IconName::Search)
                .size(px(14.))
                .text_color(colors.muted_foreground),
        )
        .child(div().min_w_0().flex_1().child(Input::new(input)))
        .test_support()
}

pub(crate) fn list_action_bar(cx: &App) -> Div {
    let colors = crate::theme::GewuPalette::get(cx);
    div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap(px(6.))
        .px_2()
        .py(px(5.))
        .bg(colors.surface)
        .border_b_1()
        .border_color(colors.border)
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
