use gewu_app::{QueryCompletion, QueryJob, QueryResult};

use super::*;

/// The status bar's report of the last table operation. One slot: a newer report replaces
/// an older one, and an operation that succeeds clears its own kind of failure.
pub(super) enum Notice {
    SelectCellToFilter,
    FilterFailed(gewu_core::Error),
    CopyFailed(CopyFailure),
}

pub(crate) enum CopyFailure {
    /// Encoding the selection as TSV failed.
    Encode(gewu_core::Error),
    Pasteboard(crate::platform::clipboard::PasteboardError),
}

impl TableView {
    pub(super) fn requested_query(&self) -> &TableQuery {
        self.data.queries().requested()
    }

    /// Advance the requested ordering even while an earlier sort is running.
    pub(super) fn cycle_sort(&mut self, column: usize, multiple: bool, cx: &mut Context<Self>) {
        let id = self.data.columns[column].id;
        self.edit_query(cx, |query| {
            let sorts = &mut query.sorts;
            let position = sorts.iter().position(|s| s.column == id);
            let direction = match position.map(|index| sorts[index].descending) {
                None => Some(false),
                Some(false) => Some(true),
                Some(true) => None,
            };
            if !multiple {
                sorts.clear();
            }
            match (position.filter(|_| multiple), direction) {
                (Some(i), Some(descending)) => sorts[i].descending = descending,
                (Some(i), None) => {
                    sorts.remove(i);
                }
                (None, Some(descending)) => sorts.push(ColumnSort {
                    column: id,
                    descending,
                }),
                (None, None) => {}
            }
        });
    }

    pub(crate) fn filter_selected(&mut self, cx: &mut Context<Self>) {
        let Some(cell) = self.grid.selection().map(|s| s.head) else {
            self.notice = Some(Notice::SelectCellToFilter);
            cx.notify();
            return;
        };
        if matches!(
            self.notice,
            Some(Notice::SelectCellToFilter | Notice::FilterFailed(_))
        ) {
            self.notice = None;
            cx.notify();
        }
        let id = self.data.columns[cell.column].id;
        match self.data.view().exact_filter_for_cell(id, cell.row) {
            Ok(Some(filter)) => self.set_filter(filter.column, Some(filter.filter), cx),
            Ok(None) => (),
            Err(error) => {
                self.notice = Some(Notice::FilterFailed(error));
                cx.notify();
            }
        }
    }

    pub(crate) fn clear_filters(&mut self, cx: &mut Context<Self>) {
        self.edit_query(cx, |q| q.filters.clear());
    }

    pub(super) fn edit_query(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut TableQuery),
    ) {
        if let Some(mut job) = self.data.edit_query(edit) {
            self.find.update(cx, |find, cx| find.set_view(None, cx));
            self.query_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    let result = cx.background_spawn(async move { job.run() }).await;
                    let next = this.update(cx, |table, cx| table.finish_query(result, cx));
                    match next {
                        Ok(Some(next)) => job = next,
                        _ => break,
                    }
                }
            }));
        }
        cx.notify();
    }

    fn finish_query(&mut self, result: QueryResult, cx: &mut Context<Self>) -> Option<QueryJob> {
        match self.data.finish_query(result) {
            QueryCompletion::Run(next) => return Some(next),
            QueryCompletion::Ignored => return None,
            QueryCompletion::Installed => {
                self.grid.set_rows(self.data.row_count());
                self.set_selection(None, cx);
                self.apply_pending_hides(cx);
                self.grid_changed(cx);
                self.refresh_query_drawer(cx);
            }
            QueryCompletion::Reverted => {
                if self.apply_pending_hides(cx) {
                    self.grid_changed(cx);
                }
            }
            QueryCompletion::Failed => self.controls.pending_hides.clear(),
        }
        self.query_task = None;
        self.find.update(cx, |find, cx| {
            find.set_view(Some(self.data.view().clone()), cx)
        });
        cx.notify();
        None
    }
}
