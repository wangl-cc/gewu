//! Snapshot-owned cell Find with bounded directional scans and highlights.

use gewu_core as core;

use crate::{ColumnId, GewuError};

/// A matching cell and exclusive directional cursor in the captured view.
#[derive(uniffi::Record, Clone, Copy)]
pub struct CellMatch {
    pub display_row: u64,
    pub column: ColumnId,
}

/// Match flags for a block of display rows, one bitmap per requested column.
#[derive(uniffi::Record)]
pub struct FindHighlight {
    pub start: u64,
    pub n_rows: u32,
    /// One entry per requested column, in request order.
    pub columns: Vec<ColumnHighlight>,
}

/// Match flags for one column of a [`FindHighlight`].
#[derive(uniffi::Record)]
pub struct ColumnHighlight {
    pub column: ColumnId,
    /// One bit per row starting at the block's first row, low bit first.
    pub matched: Vec<u8>,
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

/// An immutable view/query/column scope.
#[derive(uniffi::Object)]
pub struct FindSession {
    pub(crate) inner: core::Find,
}

#[uniffi::export]
impl FindSession {
    /// Return at most `limit` matches strictly after `after`, in display order.
    pub fn next(&self, after: Option<CellMatch>, limit: u32) -> Result<Vec<CellMatch>, GewuError> {
        let after = after.map(Into::into);
        Ok(self
            .inner
            .next(after.as_ref(), limit)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Return at most `limit` matches strictly before `before`, in display order.
    pub fn previous(
        &self,
        before: Option<CellMatch>,
        limit: u32,
    ) -> Result<Vec<CellMatch>, GewuError> {
        let before = before.map(Into::into);
        Ok(self
            .inner
            .previous(before.as_ref(), limit)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Match flags for the requested columns, such as those on screen, over a
    /// block of display rows. Every column must belong to the captured scope.
    pub fn highlight(
        &self,
        columns: Vec<ColumnId>,
        start: u64,
        count: u32,
    ) -> Result<FindHighlight, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(self.inner.highlight(&columns, start, count)?.into())
    }
}

impl From<core::CellMatch> for CellMatch {
    fn from(hit: core::CellMatch) -> Self {
        Self {
            display_row: hit.display_row,
            column: hit.column.into(),
        }
    }
}

impl From<CellMatch> for core::CellMatch {
    fn from(hit: CellMatch) -> Self {
        Self {
            display_row: hit.display_row,
            column: hit.column.into(),
        }
    }
}

impl From<core::FindHighlight> for FindHighlight {
    fn from(highlight: core::FindHighlight) -> Self {
        Self {
            start: highlight.start,
            n_rows: highlight.n_rows,
            columns: highlight
                .columns
                .into_iter()
                .map(|column| ColumnHighlight {
                    column: column.column.into(),
                    matched: column.matched,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_limits_match_the_exported_core_constants() {
        let limits = find_limits();
        assert_eq!(limits.max_page_size, core::MAX_FIND_PAGE_SIZE);
        assert_eq!(limits.max_highlight_rows, core::MAX_FIND_HIGHLIGHT_ROWS);
    }

    #[test]
    fn find_pages_and_highlights_cross_the_boundary() {
        let source =
            std::env::temp_dir().join(format!("gewu-uniffi-find-{}.csv", std::process::id()));
        std::fs::write(&source, "value,other\nhit,12\nmiss,120\nhit,12\n").unwrap();
        let dataset =
            crate::open_dataset(source.to_str().unwrap().to_owned(), Default::default()).unwrap();
        let c0 = dataset.column_id("value".into()).unwrap();
        let c1 = dataset.column_id("other".into()).unwrap();
        let doc = dataset.unfiltered().unsorted();
        let find = doc.find("12".into(), vec![c0, c1]).unwrap();

        let page = find.next(None, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!((page[0].display_row, page[0].column), (0, c1));
        assert_eq!((page[1].display_row, page[1].column), (2, c1));
        let highlight = find.highlight(vec![c1, c0], 0, 3).unwrap();
        let columns: Vec<_> = highlight
            .columns
            .iter()
            .map(|column| (column.column, column.matched.clone()))
            .collect();
        assert_eq!(columns, [(c1, vec![0b101]), (c0, vec![0])]);
        assert_eq!(find.previous(None, 1).unwrap()[0].display_row, 2);
        assert_eq!(doc.source_rows(0, 10).unwrap().count(), 3);
        std::fs::remove_file(source).unwrap();
    }
}
