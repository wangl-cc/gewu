//! Scalar values, exact endpoints and text parsing shared by filters and statistics.

pub(crate) mod bound;
pub(crate) mod decimal;
pub(crate) mod float;
pub(crate) mod temporal;

pub(crate) use bound::Bound;
