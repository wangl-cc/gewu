//! Query input specifications.

mod error;
pub mod filter;
pub(crate) mod spec;

pub use error::QueryError;
pub use filter::{Filter, FilterCondition, TextMatch};
pub use spec::{ColSort, ColSpec, QuerySpec};
