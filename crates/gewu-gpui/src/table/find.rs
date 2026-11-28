//! The search field and the shell around the table's [`FindSession`]: it feeds the session
//! the input's text, the displayed view, the visible columns and the viewport, runs the jobs
//! it returns in the background and hands their results back.

use std::{sync::Arc, time::Duration};

use gewu_app::find::{FindSession, FindStatus, FindWork};
use gewu_core::{CellMatch, ColumnId, OrderedView, ViewRow};
use gpui_kit::{
    base::{
        self,
        input::{Input, InputEvent, InputState},
    },
    prelude::FluentBuilder,
    *,
};

pub(crate) const TYPING_DELAY: Duration = Duration::from_millis(150);
/// The field keeps this width with and without a query, so typing never moves it. It leaves
/// room for text beside a count in the millions and the clear button.
const FIELD_WIDTH: Pixels = px(280.);

pub(crate) struct FindController {
    input: Entity<InputState>,
    _subscription: Subscription,
    session: FindSession,
    /// Display names of the visible columns, set together with the session's scope.
    names: Vec<(ColumnId, SharedString)>,
    navigation: Option<Task<()>>,
    count: Option<Task<()>>,
    reading: Option<Task<()>>,
}

impl EventEmitter<CellMatch> for FindController {}

impl FindController {
    pub(crate) fn new(
        view: Arc<OrderedView>,
        columns: Vec<(ColumnId, SharedString)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in table"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |find, input, event: &InputEvent, _, cx| match event {
                InputEvent::Focus => {
                    find.session.open();
                    cx.notify();
                }
                InputEvent::Change => {
                    let text = input.read(cx).value();
                    let work = find.session.set_text(&text);
                    find.restart(work, cx);
                }
                InputEvent::PressEnter { shift, .. } => find.find_match(*shift, cx),
                _ => {}
            },
        );
        Self {
            input,
            _subscription: subscription,
            session: FindSession::new(view, columns.iter().map(|(id, _)| *id).collect()),
            names: columns,
            navigation: None,
            count: None,
            reading: None,
        }
    }

    pub(crate) fn text(&self, cx: &App) -> SharedString {
        self.input.read(cx).value()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.session.is_open()
    }

    #[cfg(test)]
    pub(crate) fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(crate) fn can_navigate(&self) -> bool {
        self.session.can_navigate()
    }

    /// None means the background search has not classified this cell yet.
    #[cfg(test)]
    pub(crate) fn cached_match(&self, column: ColumnId, row: u64) -> Option<bool> {
        self.session.cached_match(column, row)
    }

    pub(crate) fn current_match(&self) -> Option<CellMatch> {
        self.session.current()
    }

    pub(crate) fn set_view(&mut self, view: Option<Arc<OrderedView>>, cx: &mut Context<Self>) {
        let work = self.session.set_view(view);
        self.restart(work, cx);
    }

    pub(crate) fn set_columns(
        &mut self,
        columns: Vec<(ColumnId, SharedString)>,
        retain_match: bool,
        cx: &mut Context<Self>,
    ) {
        let work = self
            .session
            .set_columns(columns.iter().map(|(id, _)| *id).collect(), retain_match);
        self.names = columns;
        self.restart(work, cx);
    }

    pub(crate) fn set_viewport(
        &mut self,
        rows: std::ops::Range<u64>,
        columns: Vec<ColumnId>,
        cx: &mut Context<Self>,
    ) {
        let work = self.session.set_viewport(rows, columns);
        self.run(work, cx);
    }

    pub(crate) fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.session.open();
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session.close();
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.cancel();
        cx.notify();
    }

    pub(crate) fn find_match(&mut self, previous: bool, cx: &mut Context<Self>) {
        let work = self.session.step(if previous { -1 } else { 1 });
        self.run(work, cx);
    }

    fn cancel(&mut self) {
        self.navigation = None;
        self.count = None;
        self.reading = None;
    }

    /// The scope changed: the work of the previous search no longer applies.
    fn restart(&mut self, work: FindWork, cx: &mut Context<Self>) {
        self.cancel();
        self.run(work, cx);
    }

    fn run(&mut self, work: FindWork, cx: &mut Context<Self>) {
        if let Some(hit) = work.reveal {
            cx.emit(hit);
        }
        if let Some(job) = work.step {
            self.navigation = Some(cx.spawn(async move |this, cx| {
                if job.debounce {
                    cx.background_executor().timer(TYPING_DELAY).await;
                }
                let result = cx.background_spawn(async move { job.run() }).await;
                _ = this.update(cx, |find, cx| {
                    find.navigation = None;
                    let work = find.session.step_finished(result);
                    find.run(work, cx);
                });
            }));
        }
        if let Some(job) = work.count {
            let worker = cx
                .background_executor()
                .spawn_with_priority(Priority::Low, async move { job.run() });
            self.count = Some(cx.spawn(async move |this, cx| {
                let result = worker.await;
                _ = this.update(cx, |find, cx| {
                    find.session.count_finished(result);
                    cx.notify();
                });
            }));
        }
        if let Some(job) = work.highlight {
            self.reading = Some(cx.spawn(async move |this, cx| {
                let result = cx.background_spawn(async move { job.run() }).await;
                _ = this.update(cx, |find, cx| {
                    let work = find.session.highlight_finished(result);
                    find.run(work, cx);
                });
            }));
        }
        cx.notify();
    }

    /// The status announced to assistive technology and shown as the search's location.
    pub(crate) fn message(&self, cx: &App) -> String {
        match self.session.status() {
            FindStatus::Idle => String::new(),
            FindStatus::Searching { .. } => "Searching…".into(),
            FindStatus::NoMatches => crate::ui::messages::NO_MATCHES.into(),
            FindStatus::Failed { error, .. } => crate::ui::errors::message(error),
            FindStatus::At { hit, wrapped } => {
                let source = self
                    .session
                    .view()
                    .and_then(|view| view.source_row_at(hit.display_row));
                let name = self.names.iter().find(|(id, _)| *id == hit.column);
                let (Some(source), Some((_, name))) = (source, name) else {
                    return String::new();
                };
                let prefix = if *wrapped { "Wrapped · " } else { "" };
                let row = source.0 + crate::preferences::Preferences::get(cx).row_number_base;
                format!("{prefix}Row {row} · {name}")
            }
        }
    }

    /// The match highlights to paint in one displayed cell. See [`FindSession::cell_spans`].
    pub(crate) fn cell_spans<'a>(
        &self,
        column: ColumnId,
        row: ViewRow,
        text_matchable: bool,
        source: impl FnOnce() -> Option<(&'a str, &'a str)>,
        shown_len: usize,
        spans: &mut Vec<std::ops::Range<usize>>,
    ) {
        self.session
            .cell_spans(column, row, text_matchable, source, shown_len, spans);
    }
}

impl Render for FindController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = &self.input;
        let colors = crate::theme::GewuPalette::get(cx);
        let searching = self.session.is_open() && !self.text(cx).is_empty();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let icon = gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Search)
            .size(px(14.))
            .text_color(colors.muted_foreground);
        let error = matches!(self.session.status(), FindStatus::Failed { .. });
        let notice: Option<String> = match self.session.status() {
            FindStatus::Failed { error, .. } => Some(crate::ui::errors::message(error)),
            FindStatus::NoMatches => Some(crate::ui::messages::NO_MATCHES.into()),
            FindStatus::At { wrapped: true, .. } => Some("Wrapped".into()),
            _ => None,
        };
        let count = self.session.count().map(|position| match position {
            Some((current, total)) => {
                let current = crate::ui::messages::format_count(current);
                let total = crate::ui::messages::format_count(total);
                (
                    format!("{current}/{total}"),
                    format!("Match {current} of {total}"),
                )
            }
            None => ("…".into(), "Counting matches".into()),
        });
        let trigger = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap_1()
            .child(icon)
            .child(div().flex_1().min_w_0().child(Input::new(input)))
            .children(notice.filter(|_| searching).map(|text| {
                div()
                    .id("find-notice")
                    .role(if error { Role::Alert } else { Role::Status })
                    .aria_label(text.clone())
                    .max_w(relative(0.55))
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(if error {
                        colors.danger_text
                    } else {
                        colors.muted_foreground
                    })
                    .when(error, |notice| {
                        notice.child(crate::ui::errors::warning_icon())
                    })
                    .child(div().min_w_0().truncate().child(text))
                    .test_support()
            }))
            .children(count.map(|(text, label)| {
                div()
                    .id("find-count")
                    .role(Role::Status)
                    .aria_label(label)
                    .flex_none()
                    .font_features(crate::theme::tabular_digits())
                    .text_size(px(11.))
                    .text_color(colors.muted_foreground)
                    .child(text)
                    .test_support()
            }))
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, w, cx| {
                if event.button == MouseButton::Left
                    && (!this.session.is_open()
                        || !this.input.read(cx).focus_handle(cx).is_focused(w))
                {
                    this.session.open();
                    this.input.update(cx, |input, cx| input.focus(w, cx));
                    cx.notify();
                }
            }));
        let active_focus = focused && window.is_window_active();
        let mut field = div()
            .id("table-search")
            .role(Role::Group)
            .aria_label("Find in table")
            .aria_description(self.message(cx))
            .w(FIELD_WIDTH)
            .max_w(relative(0.62))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .h(px(28.))
            .pl_2()
            .pr_1()
            .bg(colors.canvas)
            .border_1()
            .border_color(if active_focus {
                colors.muted_foreground
            } else {
                colors.border
            })
            .when(!active_focus, |field| {
                field.hover(move |s| s.border_color(colors.input_hover()))
            })
            .rounded(px(6.))
            .child(trigger);
        if searching {
            field = field.child(
                crate::ui::ghost_button("close-find", "Clear search", false, false, cx)
                    .size(px(20.))
                    .rounded(px(4.))
                    .text_color(colors.muted_foreground)
                    .hover(move |style| {
                        style
                            .bg(colors.button_secondary_hover)
                            .text_color(colors.foreground)
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(base::input::Escape), cx)
                    })
                    .child("×"),
            );
        }
        field.test_support()
    }
}
