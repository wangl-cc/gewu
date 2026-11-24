//! Native single-line search input and snapshot-scoped match navigation.

use std::{
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gewu_core::{
    CellMatch, ColumnHighlight, ColumnId, Find, FindCounts, FindHighlight, MAX_FIND_HIGHLIGHT_ROWS,
    OrderedView,
};
use gpui_kit::{
    base::{
        self,
        input::{Input, InputEvent, InputState},
    },
    prelude::FluentBuilder,
    *,
};

use crate::data::BLOCK_ROWS;

pub(crate) const TYPING_DELAY: Duration = Duration::from_millis(150);

#[derive(Default)]
struct Highlights(HashMap<(ColumnId, u64), (u64, Rc<ColumnHighlight>)>);

struct MatchCounter {
    // Navigation can move and wrap while the tally for its starting cell is still running.
    starting_counts: Option<FindCounts>,
    movement: i128,
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
}

impl MatchCounter {
    fn position(&self) -> Option<(u64, u64)> {
        let FindCounts { through, total } = self.starting_counts?;
        if through == 0 || total == 0 {
            return None;
        }
        let zero_based = (i128::from(through - 1) + self.movement).rem_euclid(i128::from(total));
        Some((zero_based as u64 + 1, total))
    }
}

impl Drop for MatchCounter {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

enum FindMessage {
    Idle,
    Searching,
    NoMatches,
    Error(String),
    Location {
        source: gewu_core::SourceRow,
        name: SharedString,
        wrapped: bool,
    },
}

pub(crate) struct FindController {
    input: Entity<InputState>,
    _subscription: Subscription,
    pub(crate) open: bool,
    message: FindMessage,
    session: Option<Arc<Find>>,
    cursor: Option<CellMatch>,
    view: Option<Arc<OrderedView>>,
    columns: Vec<(ColumnId, SharedString)>,
    viewport_rows: Range<u64>,
    viewport_columns: Vec<ColumnId>,
    queued: i64,
    highlights: Highlights,
    navigation: Option<Task<()>>,
    counter: Option<MatchCounter>,
    reading: Option<Task<()>>,
    highlight_failed: bool,
}

impl FindController {
    pub(crate) fn text(&self, cx: &App) -> SharedString {
        self.input.read(cx).value()
    }

    #[cfg(test)]
    pub(crate) fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    fn invalidate(&mut self) {
        self.queued = 0;
        self.highlights.0.clear();
        self.reading = None;
        self.navigation = None;
        self.counter = None;
        self.highlight_failed = false;
        self.session = None;
        self.cursor = None;
        self.message = FindMessage::Idle;
    }

    pub(crate) fn can_navigate(&self) -> bool {
        self.navigation.is_some() || self.cursor.is_some()
    }

    /// None means the background search has not classified this cell yet.
    pub(crate) fn cached_match(&self, column: ColumnId, row: u64) -> Option<bool> {
        self.highlights.cached_match(column, row)
    }

    pub(crate) fn current_match(&self) -> Option<CellMatch> {
        self.cursor
    }

    pub(crate) fn session(&self) -> Option<&Find> {
        self.session.as_deref()
    }
}
impl Highlights {
    fn cached_match(&self, column: ColumnId, row: u64) -> Option<bool> {
        self.0
            .get(&(column, row / BLOCK_ROWS))
            .map(|(start, column)| column.matches((row - start) as usize))
    }

    fn store_highlights(&mut self, result: FindHighlight) {
        let first = result.start / BLOCK_ROWS;
        let count = u64::from(result.n_rows).div_ceil(BLOCK_ROWS);
        for column in result.columns {
            // A batched read can fill several cache slots with the same bitmap.
            let column = Rc::new(column);
            for block in first..first + count {
                self.0
                    .insert((column.column, block), (result.start, Rc::clone(&column)));
            }
        }
    }
}

impl EventEmitter<CellMatch> for FindController {}

impl FindController {
    pub(crate) fn set_view(&mut self, view: Option<Arc<OrderedView>>, cx: &mut Context<Self>) {
        self.view = view;
        self.restart_scope(None, Duration::ZERO, cx);
    }

    pub(crate) fn set_columns(
        &mut self,
        columns: Vec<(ColumnId, SharedString)>,
        retain_match: bool,
        cx: &mut Context<Self>,
    ) {
        let hit = self
            .cursor
            .filter(|c| retain_match && columns.iter().any(|(id, _)| *id == c.column));
        self.viewport_columns
            .retain(|id| columns.iter().any(|(column, _)| column == id));
        self.columns = columns;
        if self.view.is_some() {
            self.restart_scope(hit, Duration::ZERO, cx);
        }
    }

    fn restart_scope(&mut self, hit: Option<CellMatch>, delay: Duration, cx: &mut Context<Self>) {
        self.invalidate();
        if self.open && self.view.is_some() {
            if let Some(hit) = hit {
                self.show_match(hit, false);
                if let Some(session) = self.find_session(cx) {
                    self.start_count(session, hit, cx);
                }
            } else {
                self.advance_find(1, delay, cx);
            }
            self.load_highlights(cx);
        }
        cx.notify();
    }

    // Scope changes cancel navigation in invalidate(). A retained hit keeps its view and
    // is checked against the new columns by set_columns before it reaches this method.
    fn show_match(&mut self, hit: CellMatch, wrapped: bool) {
        let Some(source) = self
            .view
            .as_ref()
            .and_then(|view| view.source_row_at(hit.display_row))
        else {
            unreachable!("a Find hit belongs to its active ordered view");
        };
        let Some((_, name)) = self.columns.iter().find(|(id, _)| *id == hit.column) else {
            unreachable!("a Find hit belongs to its active column scope");
        };
        self.cursor = Some(hit);
        self.message = FindMessage::Location {
            source,
            name: name.clone(),
            wrapped,
        };
    }

    pub(crate) fn message(&self, cx: &App) -> String {
        match &self.message {
            FindMessage::Idle => String::new(),
            FindMessage::Searching => "Searching…".into(),
            FindMessage::NoMatches => "No matches".into(),
            FindMessage::Error(text) => text.clone(),
            FindMessage::Location {
                source,
                name,
                wrapped,
            } => {
                let prefix = if *wrapped { "Wrapped · " } else { "" };
                let row = source.0 + crate::preferences::Preferences::get(cx).row_number_base;
                format!("{prefix}Row {row} · {name}")
            }
        }
    }

    pub(crate) fn new(
        view: Arc<OrderedView>,
        columns: Vec<(ColumnId, SharedString)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in table"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |find, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Focus => {
                        find.open = true;
                        cx.notify();
                    }
                    InputEvent::Change => find.restart_scope(None, TYPING_DELAY, cx),
                    InputEvent::PressEnter { shift, .. } => find.find_match(*shift, cx),
                    _ => {}
                },
            );
        Self {
            input,
            _subscription: subscription,
            view: Some(view),
            columns,
            open: false,
            message: FindMessage::Idle,
            session: None,
            cursor: None,
            viewport_rows: 0..0,
            viewport_columns: vec![],
            queued: 0,
            highlights: Highlights::default(),
            navigation: None,
            counter: None,
            reading: None,
            highlight_failed: false,
        }
    }

    pub(crate) fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.open = true;
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.invalidate();
        cx.notify();
    }

    fn find_session(&mut self, cx: &App) -> Option<Arc<Find>> {
        if let Some(session) = &self.session {
            return Some(session.clone());
        }
        let text = self.text(cx);
        let view = self.view.as_ref()?;
        if text.trim().is_empty() || self.columns.is_empty() || self.highlight_failed {
            return None;
        }
        let ids = self.columns.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        match view.find(&text, &ids) {
            Ok(find) => Some(self.session.insert(Arc::new(find)).clone()),
            Err(err) => {
                self.message = FindMessage::Error(crate::errors::message(&err));
                self.highlight_failed = true;
                None
            }
        }
    }

    pub(crate) fn set_viewport(
        &mut self,
        rows: Range<u64>,
        columns: Vec<ColumnId>,
        cx: &mut Context<Self>,
    ) {
        self.viewport_rows = rows;
        self.viewport_columns = columns;
        self.load_highlights(cx);
    }

    fn load_highlights(&mut self, cx: &mut Context<Self>) {
        let rows = self.viewport_rows.clone();
        if !self.open || self.reading.is_some() || self.highlight_failed || rows.is_empty() {
            return;
        }
        let Some(find) = self.find_session(cx) else {
            return;
        };
        let first = rows.start / BLOCK_ROWS;
        let end = rows.end.div_ceil(BLOCK_ROWS);
        let cols = self.viewport_columns.clone();
        // Keep a few screens for backtracking. A jump through a large file should
        // never grow the highlight cache in proportion to the rows traversed.
        let screen_blocks = cols.len() * (end - first) as usize;
        if self.highlights.0.len() > screen_blocks.saturating_mul(4) {
            self.highlights
                .0
                .retain(|(id, b), _| cols.contains(id) && (first..end).contains(b));
        }
        let Some(block) = (first..end).find(|&b| {
            cols.iter()
                .any(|id| !self.highlights.0.contains_key(&(*id, b)))
        }) else {
            return;
        };
        let missing: Vec<_> = cols
            .into_iter()
            .filter(|id| !self.highlights.0.contains_key(&(*id, block)))
            .collect();
        let max = end.min(block + u64::from(MAX_FIND_HIGHLIGHT_ROWS) / BLOCK_ROWS);
        let stop = (block + 1..max)
            .find(|&b| {
                missing
                    .iter()
                    .any(|id| self.highlights.0.contains_key(&(*id, b)))
            })
            .unwrap_or(max);
        self.reading = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    find.highlight(
                        &missing,
                        block * BLOCK_ROWS,
                        ((stop - block) * BLOCK_ROWS) as u32,
                    )
                })
                .await;
            _ = this.update(cx, |table, cx| {
                table.reading = None;
                match result {
                    Ok(batch) => {
                        table.highlights.store_highlights(batch);
                        table.load_highlights(cx);
                    }
                    Err(error) => {
                        table.highlight_failed = true;
                        table.message = FindMessage::Error(crate::errors::message(&error.into()));
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn find_match(&mut self, previous: bool, cx: &mut Context<Self>) {
        self.advance_find(if previous { -1 } else { 1 }, Duration::ZERO, cx);
    }

    fn start_count(&mut self, session: Arc<Find>, hit: CellMatch, cx: &mut Context<Self>) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let worker = cx
            .background_executor()
            .spawn_with_priority(Priority::Low, async move {
                session.counts_at(hit, || stop.load(Ordering::Relaxed))
            });
        let task = cx.spawn(async move |this, cx| {
            let result = worker.await;
            _ = this.update(cx, |find, cx| {
                match result {
                    Ok(Some(counts)) => {
                        if let Some(counter) = &mut find.counter {
                            counter.starting_counts = Some(counts);
                        }
                    }
                    Ok(None) => return,
                    Err(error) => {
                        find.counter = None;
                        find.message = FindMessage::Error(crate::errors::message(&error.into()));
                    }
                }
                cx.notify();
            });
        });
        self.counter = Some(MatchCounter {
            starting_counts: None,
            movement: 0,
            cancelled,
            _task: task,
        });
    }

    fn advance_find(&mut self, by: i64, delay: Duration, cx: &mut Context<Self>) {
        if self.navigation.is_some() {
            self.queued = self.queued.saturating_add(by);
            return;
        }
        let Some(session) = self.find_session(cx) else {
            return;
        };
        let cursor = self.cursor;
        self.message = FindMessage::Searching;
        cx.notify();
        self.navigation = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = cx
                .background_spawn(async move {
                    let (hit, wrapped) = gewu_app::find::step(&session, cursor, by)?;
                    Ok::<_, gewu_core::Error>((session, hit, wrapped))
                })
                .await;
            _ = this.update(cx, |table, cx| {
                table.navigation = None;
                match result {
                    Ok((session, hit, wrapped)) => {
                        if let Some(hit) = hit {
                            table.show_match(hit, wrapped);
                            if let Some(counter) = &mut table.counter {
                                counter.movement += i128::from(by);
                            } else {
                                table.start_count(session, hit, cx);
                            }
                            cx.emit(hit);
                        } else {
                            table.cursor = None;
                            table.message = FindMessage::NoMatches;
                        }
                    }
                    Err(error) => {
                        table.message = FindMessage::Error(crate::errors::message(&error))
                    }
                }
                let queued = std::mem::take(&mut table.queued);
                if queued != 0 {
                    table.advance_find(queued, Duration::ZERO, cx);
                }
                cx.notify();
            });
        }));
    }
}

impl Render for FindController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = &self.input;
        let colors = crate::theme::GewuPalette::get(cx);
        let searching = self.open && !self.text(cx).is_empty();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let icon = canvas(
            |_, _, _| (),
            move |bounds, _, w, _| {
                let p = |x, y| bounds.origin + point(px(x), px(y));
                let radius = point(px(4.), px(4.));
                let mut path = PathBuilder::stroke(px(1.35));
                path.move_to(p(1.5, 5.5));
                path.arc_to(radius, px(0.), false, true, p(9.5, 5.5));
                path.arc_to(radius, px(0.), false, true, p(1.5, 5.5));
                path.move_to(p(8.5, 8.5));
                path.line_to(p(12.5, 12.5));
                if let Ok(path) = path.build() {
                    w.paint_path(path, colors.muted_foreground);
                }
            },
        )
        .size(px(14.))
        .flex_none();
        let notice = match &self.message {
            FindMessage::Error(text) => Some(text.as_str()),
            FindMessage::NoMatches => Some("No matches"),
            FindMessage::Location { wrapped: true, .. } => Some("Wrapped"),
            _ => None,
        };
        let count = self.counter.as_ref().and_then(|counter| {
            if let Some((current, total)) = counter.position() {
                let current = crate::messages::format_count(current);
                let total = crate::messages::format_count(total);
                Some((
                    format!("{current}/{total}"),
                    format!("Match {current} of {total}"),
                ))
            } else if counter.starting_counts.is_none() {
                Some(("…".into(), "Counting matches".into()))
            } else {
                None
            }
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
                let error = matches!(self.message, FindMessage::Error(_));
                div()
                    .id("find-notice")
                    .role(if error { Role::Alert } else { Role::Status })
                    .aria_label(text.to_owned())
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
                    .when(error, |notice| notice.child(crate::errors::warning_icon()))
                    .child(div().min_w_0().truncate().child(text.to_owned()))
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
                    && (!this.open || !this.input.read(cx).focus_handle(cx).is_focused(w))
                {
                    this.open = true;
                    this.input.update(cx, |input, cx| input.focus(w, cx));
                    cx.notify();
                }
            }));
        let mut field = div()
            .id("table-search")
            .flex_1()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .h(px(28.))
            .pl_2()
            .pr(px(if searching { 4. } else { 8. }))
            .bg(colors.canvas)
            .border_1()
            .border_color(colors.border)
            .hover(move |s| s.border_color(colors.input_hover()))
            .when(focused && window.is_window_active(), |field| {
                field.shadow(colors.focus_shadows())
            })
            .rounded(px(6.))
            .child(trigger);
        if searching {
            field = field.child(
                base::Button::new("close-find")
                    .accessibility_label("Clear search")
                    .size(px(20.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(colors.muted_foreground)
                    .hover(move |style| {
                        style
                            .bg(colors.button_secondary_hover)
                            .text_color(colors.foreground)
                    })
                    .active(move |style| style.bg(colors.secondary_pressed()))
                    .focus_visible(move |s| s.shadow(colors.focus_shadows()))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(base::input::Escape), cx)
                    })
                    .child("×"),
            );
        }
        let field = field.test_support();
        let mut bar = div()
            .id("find-toolbar")
            .role(Role::Group)
            .aria_label("Find in table")
            .aria_description(self.message(cx))
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .w(px(if searching { 300. } else { 240. }))
            .max_w(relative(0.62))
            .child(field);
        if searching {
            let enabled = self.can_navigate();
            let arrow = |id, label: &'static str, symbol: &'static str| {
                base::Button::new(id)
                    .accessibility_label(label)
                    .size(px(22.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded(px(6.))
                    .when(enabled, |button| {
                        button
                            .hover(move |style| style.bg(colors.button_secondary_hover))
                            .active(move |style| style.bg(colors.secondary_pressed()))
                            .focus_visible(move |s| s.shadow(colors.focus_shadows()))
                            .cursor_pointer()
                    })
                    .when(!enabled, |button| {
                        button
                            .opacity(crate::theme::DISABLED_OPACITY)
                            .cursor_default()
                    })
                    .disabled(!enabled)
                    .child(symbol)
            };
            bar = bar
                .child(
                    arrow("previous", "Previous match", "‹")
                        .on_click(cx.listener(|find, _, _, cx| find.find_match(true, cx))),
                )
                .child(
                    arrow("next", "Next match", "›")
                        .on_click(cx.listener(|find, _, _, cx| find.find_match(false, cx))),
                );
        }
        bar.into_any_element()
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
