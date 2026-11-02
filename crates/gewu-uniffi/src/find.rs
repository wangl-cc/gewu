//! Snapshot-owned cell Find with bounded directional scans and highlights.

use std::sync::Arc;

use gewu_core as core;

use crate::{Cancellation, GewuError};

/// A matching cell and exclusive directional cursor in the captured view.
#[derive(uniffi::Record, Clone, Copy)]
pub struct CellMatch {
    pub display_row: u64,
    pub column: u32,
}

/// Row-major match flags in the Find session's captured column order.
#[derive(uniffi::Record)]
pub struct FindHighlight {
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    pub matched: Vec<bool>,
}

/// Bounded Find request sizes, exported so frontends never duplicate Core limits.
#[derive(uniffi::Record)]
pub struct FindLimits {
    /// Maximum cell cursors returned by one directional page.
    pub max_page_size: u32,
    /// Maximum display rows covered by one highlight request.
    pub max_highlight_rows: u32,
}

#[uniffi::export]
pub fn find_limits() -> FindLimits {
    FindLimits {
        max_page_size: core::MAX_FIND_PAGE_SIZE,
        max_highlight_rows: core::MAX_FIND_HIGHLIGHT_ROWS,
    }
}

/// An immutable view/query/column scope with independent terminal cancellation.
#[derive(uniffi::Object)]
pub struct FindSession {
    pub(crate) inner: core::Find,
}

#[uniffi::export]
impl FindSession {
    /// Next page with per-request cancellation that leaves the session usable.
    pub fn next(
        &self,
        after: Option<CellMatch>,
        limit: u32,
        cancellation: Arc<Cancellation>,
    ) -> Result<Vec<CellMatch>, GewuError> {
        let after = after.map(Into::into);
        Ok(self
            .inner
            .next(after.as_ref(), limit, &cancellation.inner)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Previous page with per-request cancellation that leaves the session usable.
    pub fn previous(
        &self,
        before: Option<CellMatch>,
        limit: u32,
        cancellation: Arc<Cancellation>,
    ) -> Result<Vec<CellMatch>, GewuError> {
        let before = before.map(Into::into);
        Ok(self
            .inner
            .previous(before.as_ref(), limit, &cancellation.inner)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Highlight with per-request cancellation that leaves the session usable.
    pub fn highlight(
        &self,
        start: u64,
        count: u32,
        cancellation: Arc<Cancellation>,
    ) -> Result<FindHighlight, GewuError> {
        Ok(self
            .inner
            .highlight(start, count, &cancellation.inner)?
            .into())
    }

    /// Cancel only this Find session, not table queries or newer Find sessions.
    pub fn cancel(&self) {
        self.inner.cancel();
    }
}

impl From<core::CellMatch> for CellMatch {
    fn from(hit: core::CellMatch) -> Self {
        Self {
            display_row: hit.display_row,
            column: hit.column,
        }
    }
}

impl From<CellMatch> for core::CellMatch {
    fn from(hit: CellMatch) -> Self {
        Self {
            display_row: hit.display_row,
            column: hit.column,
        }
    }
}

impl From<core::FindHighlight> for FindHighlight {
    fn from(highlight: core::FindHighlight) -> Self {
        Self {
            version: highlight.version,
            start: highlight.start,
            n_rows: highlight.n_rows,
            n_cols: highlight.n_cols,
            matched: highlight.matched,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GewuSession;

    #[test]
    fn find_limits_match_the_exported_core_constants() {
        let limits = find_limits();
        assert_eq!(limits.max_page_size, core::MAX_FIND_PAGE_SIZE);
        assert_eq!(limits.max_highlight_rows, core::MAX_FIND_HIGHLIGHT_ROWS);
    }

    #[test]
    fn find_pages_highlights_and_cancellation_cross_the_boundary() {
        let source =
            std::env::temp_dir().join(format!("gewu-uniffi-find-{}.csv", std::process::id()));
        std::fs::write(&source, "value,other\nhit,12\nmiss,120\nhit,12\n").unwrap();
        let doc = GewuSession::open(source.to_str().unwrap().to_owned()).unwrap();
        let stale = doc.prepare_find(0, "miss".into(), vec![0, 1]).unwrap();
        let find = doc.prepare_find(0, "12".into(), vec![0, 1]).unwrap();
        stale.cancel();
        assert!(matches!(
            stale.next(None, 1, Cancellation::new()),
            Err(GewuError::Cancelled)
        ));
        let obsolete = Cancellation::new();
        obsolete.cancel();
        assert!(matches!(
            find.next(None, 1, obsolete),
            Err(GewuError::Cancelled)
        ));

        let page = find.next(None, 2, Cancellation::new()).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!((page[0].display_row, page[0].column), (0, 1));
        assert_eq!((page[1].display_row, page[1].column), (2, 1));
        let highlight = find.highlight(0, 3, Cancellation::new()).unwrap();
        assert_eq!(highlight.matched, [false, true, false, false, false, true]);
        assert_eq!(
            find.previous(None, 1, Cancellation::new()).unwrap()[0].display_row,
            2
        );
        assert!(doc.rows(0, 0, 10, vec![0]).unwrap().source_rows.len() == 3);
        std::fs::remove_file(source).unwrap();
    }
}
