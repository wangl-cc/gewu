//! Query specification, cell Find and sorting.

pub mod filter;
pub mod find;
pub(crate) mod sort;
pub(crate) mod spec;

pub use filter::{Filter, FilterCondition, TextMatch};
pub use find::{CellMatch, Find, FindHighlight, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE};
pub(crate) use sort::sort_rows;
pub use spec::{ColSort, ColSpec, QuerySpec};
