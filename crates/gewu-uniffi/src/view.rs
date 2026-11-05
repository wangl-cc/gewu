//! Formatted rows read from an immutable view.

#[derive(uniffi::Record)]
pub struct RowBlock {
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major, `n_rows * n_cols` cells; `nil` is null.
    pub cells: Vec<Option<String>>,
    pub source_rows: Vec<u64>,
}

impl From<gewu_core::RowBlock> for RowBlock {
    fn from(block: gewu_core::RowBlock) -> Self {
        Self {
            start: block.start,
            n_rows: block.n_rows,
            n_cols: block.n_cols,
            cells: block.cells,
            source_rows: block.source_rows,
        }
    }
}
