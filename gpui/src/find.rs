//! Native single-line search input and snapshot-scoped match navigation.

use std::{collections::HashMap, ops::Range, rc::Rc, sync::Arc};

use gewu_core::{
    CellMatch, ColumnHighlight, ColumnId, Find, FindHighlight, MAX_FIND_HIGHLIGHT_ROWS,
};
use gpui_kit::{
    base::input::{Input, InputEvent, InputState},
    *,
};

use crate::{
    data::BLOCK_ROWS,
    selection::{Cell, Selection},
    table::TableView,
};

#[derive(Default)]
pub(crate) struct TableFind {
    input: Option<Entity<InputState>>,
    subscription: Option<Subscription>,
    pub(crate) open: bool,
    pub(crate) message: String,
    pub(crate) pending: bool,
    session: Option<Arc<Find>>,
    cursor: Option<CellMatch>,
    generation: u64,
    queued: Option<bool>,
    highlights: HashMap<(ColumnId, u64), (u64, Rc<ColumnHighlight>)>,
    reading: bool,
    highlight_failed: bool,
}

impl TableFind {
    pub(crate) fn text(&self, cx: &App) -> SharedString {
        self.input
            .as_ref()
            .map(|input| input.read(cx).value())
            .unwrap_or_default()
    }

    pub(crate) fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.queued = None;
        self.highlights.clear();
        self.reading = false;
        self.highlight_failed = false;
        self.session = None;
        self.cursor = None;
        self.pending = false;
        self.message.clear();
    }

    pub(crate) fn matched(&self, column: ColumnId, row: u64) -> bool {
        self.open
            && self
                .highlights
                .get(&(column, row / BLOCK_ROWS))
                .is_some_and(|(start, column)| column.matches((row - start) as usize))
    }

    fn store_highlights(&mut self, result: FindHighlight) {
        let first = result.start / BLOCK_ROWS;
        let count = u64::from(result.n_rows).div_ceil(BLOCK_ROWS);
        for column in result.columns {
            // A batched read can fill several cache slots with the same bitmap.
            let column = Rc::new(column);
            for block in first..first + count {
                self.highlights
                    .insert((column.column, block), (result.start, Rc::clone(&column)));
            }
        }
    }
}

impl TableView {
    pub(crate) fn reset_find_for_view(&mut self) {
        self.find.invalidate();
    }

    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self
            .find
            .input
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("Find in table"))
            })
            .clone();
        if self.find.subscription.is_none() {
            self.find.subscription = Some(cx.subscribe_in(
                &input,
                window,
                |table, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        table.find.invalidate();
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift, .. } => table.find_match(*shift, cx),
                    _ => {}
                },
            ));
        }
        self.find.open = true;
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = false;
        let cursor = self.find.cursor;
        self.find.invalidate();
        self.find.cursor = cursor;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn find_session(&mut self, cx: &App) -> Option<Arc<Find>> {
        if let Some(session) = &self.find.session {
            return Some(session.clone());
        }
        let text = self.find.text(cx);
        if text.trim().is_empty() || self.find.highlight_failed {
            return None;
        }
        let columns = self.data.column_ids(0..self.data.columns.len());
        match self.data.view().find(&text, &columns) {
            Ok(find) => Some(self.find.session.insert(Arc::new(find)).clone()),
            Err(err) => {
                self.find.message = err.to_string();
                self.find.highlight_failed = true;
                None
            }
        }
    }

    pub(crate) fn request_highlights(
        &mut self,
        rows: Range<u64>,
        visible: &[usize],
        cx: &mut Context<Self>,
    ) {
        if !self.find.open || self.find.reading || self.find.highlight_failed || rows.is_empty() {
            return;
        }
        let Some(find) = self.find_session(cx) else {
            return;
        };
        let first = rows.start / BLOCK_ROWS;
        let end = rows.end.div_ceil(BLOCK_ROWS);
        let cols: Vec<_> = visible.iter().map(|i| self.data.columns[*i].id).collect();
        // Keep a few screens for backtracking. A jump through a large file should
        // never grow the highlight cache in proportion to the rows traversed.
        let screen_blocks = cols.len() * (end - first) as usize;
        if self.find.highlights.len() > screen_blocks.saturating_mul(4) {
            self.find
                .highlights
                .retain(|(id, b), _| cols.contains(id) && (first..end).contains(b));
        }
        let Some(block) = (first..end).find(|&b| {
            cols.iter()
                .any(|id| !self.find.highlights.contains_key(&(*id, b)))
        }) else {
            return;
        };
        let missing: Vec<_> = cols
            .into_iter()
            .filter(|id| !self.find.highlights.contains_key(&(*id, block)))
            .collect();
        let max = end.min(block + u64::from(MAX_FIND_HIGHLIGHT_ROWS) / BLOCK_ROWS);
        let stop = (block + 1..max)
            .find(|&b| {
                missing
                    .iter()
                    .any(|id| self.find.highlights.contains_key(&(*id, b)))
            })
            .unwrap_or(max);
        let generation = self.find.generation;
        self.find.reading = true;
        cx.spawn(async move |this, cx| {
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
                if table.find.generation != generation {
                    return;
                }
                table.find.reading = false;
                match result {
                    Ok(batch) => table.find.store_highlights(batch),
                    Err(error) => {
                        table.find.highlight_failed = true;
                        table.find.message = error.to_string();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn find_match(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.find.pending {
            self.find.queued = Some(previous);
            return;
        }
        let Some(session) = self.find_session(cx) else {
            return;
        };
        let view = self.data.view().clone();
        let cursor = self.find.cursor;
        let generation = self.find.generation;
        self.find.pending = true;
        self.find.message = "Searching…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let scan = |after| {
                        if previous {
                            session.previous(after, 1)
                        } else {
                            session.next(after, 1)
                        }
                    };
                    let mut hit = scan(cursor.as_ref())?.into_iter().next();
                    let wrapped = cursor.is_some() && hit.is_none();
                    if wrapped {
                        hit = scan(None)?.into_iter().next();
                    }
                    Ok::<_, gewu_core::Error>((session, hit, wrapped))
                })
                .await;
            _ = this.update(cx, |table, cx| {
                if table.find.generation != generation || !Arc::ptr_eq(table.data.view(), &view) {
                    return;
                }
                table.find.pending = false;
                match result {
                    Ok((session, hit, wrapped)) => {
                        table.find.session = Some(session);
                        table.find.cursor = hit;
                        if let Some(hit) = hit {
                            if let Some(index) =
                                table.data.columns.iter().position(|c| c.id == hit.column)
                            {
                                let cell = Cell {
                                    row: hit.display_row,
                                    column: index,
                                };
                                table.selection = Some(Selection::single(cell));
                                table.reveal(cell);
                                table.find.message = format!(
                                    "{}Row {} · {}",
                                    if wrapped { "Wrapped · " } else { "" },
                                    hit.display_row + 1,
                                    table.data.columns[index].name
                                );
                            }
                        } else {
                            table.find.message = "No matches".into();
                        }
                    }
                    Err(error) => table.find.message = error.to_string(),
                }
                if let Some(previous) = table.find.queued.take() {
                    table.find_match(previous, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let input = self.find.input.as_ref().filter(|_| self.find.open)?;
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_2()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0xe5e9ed)))
                .child(label)
        };
        Some(
            div()
                .h(px(40.))
                .w_full()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .bg(rgb(0xf0f1f3))
                .text_color(rgb(0x1f2328))
                .text_sm()
                .child("Find")
                .child(
                    div()
                        .w(px(230.))
                        .h(px(28.))
                        .px_2()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0x9aa2ad))
                        .rounded_sm()
                        .child(Input::new(input)),
                )
                .child(
                    button("previous", "Previous")
                        .on_click(cx.listener(|table, _, _, cx| table.find_match(true, cx))),
                )
                .child(
                    button("next", "Next")
                        .on_click(cx.listener(|table, _, _, cx| table.find_match(false, cx))),
                )
                .child(
                    button("close-find", "Done")
                        .on_click(cx.listener(|table, _, window, cx| table.close_find(window, cx))),
                )
                .child(self.find.message.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnHighlight, ColumnId, FindHighlight, TableFind};

    #[test]
    fn batched_highlight_uses_relative_rows_across_cache_blocks() {
        let column = ColumnId::from_raw(3);
        let mut find = TableFind {
            open: true,
            ..TableFind::default()
        };
        // A 67-row result starting at display row 64, with matches straddling
        // the cache boundary and a final match in the partial block.
        find.store_highlights(FindHighlight {
            start: 64,
            n_rows: 67,
            columns: vec![ColumnHighlight {
                column,
                matched: vec![0, 0, 0, 0, 0, 0, 0, 0x80, 0x05],
            }],
        });
        let hits: Vec<_> = (0..192).filter(|&row| find.matched(column, row)).collect();
        assert_eq!(hits, [127, 128, 130]);
    }
}
