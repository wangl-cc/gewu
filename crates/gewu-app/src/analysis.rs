//! Row-order-independent analyses, shared across windows and orderings of the same result.
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use gewu_core::{
    AxisScale, ColumnId, FilterChoices, FilteredView, Histogram, LengthSummary, NumericStatistics,
    RangeHistogram, Result, ScatterResult, ValueSummary,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Entry<V> = Arc<Mutex<Option<Arc<V>>>>;

/// Keep one parameter set per column. Concurrent requests share a calculation, but a new
/// parameter set can proceed independently. An old completion fills only its own entry.
struct Slots<K, P, V>(Mutex<HashMap<K, (P, Entry<V>)>>);
impl<K, P, V> Default for Slots<K, P, V> {
    fn default() -> Self {
        Self(Mutex::default())
    }
}
impl<K: Eq + Hash, P: PartialEq, V> Slots<K, P, V> {
    fn read<E>(
        &self,
        key: K,
        params: P,
        compute: impl FnOnce() -> std::result::Result<V, E>,
    ) -> std::result::Result<Arc<V>, E> {
        let entry = {
            let mut entries = lock(&self.0);
            match entries.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut existing) => {
                    if existing.get().0 != params {
                        existing.insert((params, Entry::default()));
                    }
                    existing.get().1.clone()
                }
                std::collections::hash_map::Entry::Vacant(missing) => {
                    missing.insert((params, Entry::default())).1.clone()
                }
            }
        };
        let mut result = lock(&entry);
        if let Some(value) = &*result {
            return Ok(value.clone());
        }
        let value = Arc::new(compute()?);
        *result = Some(value.clone());
        Ok(value)
    }
}

type ScatterParams = (Option<ColumnId>, ColumnId, u32, AxisScale, AxisScale);

#[cfg(test)]
#[path = "analysis_tests.rs"]
mod tests;

/// Cached analyses of a single filtered row set. Failed computations can be retried.
pub struct Analyses {
    pub(super) rows: Arc<FilteredView>,
    count_nulls: Slots<ColumnId, (), u64>,
    statistics: Slots<ColumnId, (), NumericStatistics>,
    distribution: Slots<ColumnId, (u32, AxisScale), Histogram>,
    values: Slots<ColumnId, (), ValueSummary>,
    lengths: Slots<ColumnId, u32, LengthSummary>,
    filter_choices: Slots<ColumnId, (), FilterChoices>,
    filter_distribution: Slots<ColumnId, (u32, AxisScale), Option<RangeHistogram>>,
    scatter: Slots<(), ScatterParams, ScatterResult>,
}
impl Analyses {
    pub(super) fn new(rows: Arc<FilteredView>) -> Self {
        Self {
            rows,
            count_nulls: Slots::default(),
            statistics: Slots::default(),
            distribution: Slots::default(),
            values: Slots::default(),
            lengths: Slots::default(),
            filter_choices: Slots::default(),
            filter_distribution: Slots::default(),
            scatter: Slots::default(),
        }
    }

    pub fn count_nulls(&self, column: ColumnId) -> Result<u64> {
        self.count_nulls
            .read(column, (), || self.rows.count_nulls(column))
            .map(|n| *n)
    }

    pub fn statistics(&self, column: ColumnId) -> Result<Arc<NumericStatistics>> {
        self.statistics
            .read(column, (), || self.rows.statistics(column))
    }

    pub fn distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Arc<Histogram>> {
        self.distribution.read(column, (bins, scale), || {
            self.rows.distribution(column, bins, scale)
        })
    }

    pub fn values(&self, column: ColumnId) -> Result<Arc<ValueSummary>> {
        self.values.read(column, (), || self.rows.values(column))
    }

    pub fn lengths(&self, column: ColumnId, bins: u32) -> Result<Arc<LengthSummary>> {
        self.lengths
            .read(column, bins, || self.rows.lengths(column, bins))
    }

    pub fn filter_choices(&self, column: ColumnId) -> Result<Arc<FilterChoices>> {
        self.filter_choices
            .read(column, (), || self.rows.filter_choices(column))
    }

    pub fn filter_distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Arc<Option<RangeHistogram>>> {
        self.filter_distribution.read(column, (bins, scale), || {
            self.rows.filter_distribution(column, bins, scale)
        })
    }

    pub fn scatter(
        &self,
        x: Option<ColumnId>,
        y: ColumnId,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<Arc<ScatterResult>> {
        self.scatter.read((), (x, y, limit, x_scale, y_scale), || {
            self.rows.scatter(x, y, limit, x_scale, y_scale)
        })
    }
}
