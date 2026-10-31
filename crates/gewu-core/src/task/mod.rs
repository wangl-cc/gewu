//! Background and asynchronous computational tasks over tabular snapshots.

pub mod analysis;
pub mod export;
pub mod find;
pub mod scatter;
pub mod text_choices;

pub use analysis::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, ColumnAnalysis, ColumnSummary,
    LengthSummary, LengthUnit, NumericStatistics, ValueSummary,
};
pub use export::{Export, ExportProgress};
pub use find::{
    CellMatch, Find, FindHighlight, FindOperation, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};
pub use scatter::{ScatterAnalysis, ScatterPoint, ScatterResult, MAX_SCATTER_POINTS};
pub use text_choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};
