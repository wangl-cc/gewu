//! Query specification, row selection, and batch predicates.

pub mod filter;
pub mod find;
pub(crate) mod predicate;
pub(crate) mod scope;
pub(crate) mod selection;
pub(crate) mod sort;
pub(crate) mod view;

pub use filter::{Filter, FilterCondition};
pub use find::{CellMatch, Find, FindHighlight, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE};
pub(crate) use sort::sort_rows;
pub use view::{ColSort, ColSpec, QuerySpec, RowBlock};
