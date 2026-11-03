//! Query input specifications.

pub mod filter;
pub(crate) mod spec;

pub use filter::{Filter, FilterCondition, TextMatch};
pub use spec::{ColSort, ColSpec, QuerySpec};
