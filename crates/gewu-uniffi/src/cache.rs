//! Analysis results kept with the view they describe.

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Mutex, PoisonError},
};

use gewu_core::{
    AxisScale, ColumnId, FilterChoices, Histogram, LengthSummary, NumericStatistics,
    RangeHistogram, ValueSummary,
};

use crate::ScatterResult;

/// One result per slot, for the parameters it was computed with. A request
/// for a slot with other parameters replaces its result, so a panel changing
/// its bin count does not accumulate stale results and the cache stays
/// bounded by the number of columns.
///
/// The lock is not held while computing: concurrent first requests for one
/// slot may both compute, and the later result is kept.
pub(crate) struct Slots<K, P, V> {
    entries: Mutex<HashMap<K, (P, V)>>,
}

impl<K, P, V> Default for Slots<K, P, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
        }
    }
}

impl<K: Eq + Hash, P: PartialEq, V: Clone> Slots<K, P, V> {
    pub(crate) fn get_or_compute<E>(
        &self,
        slot: K,
        params: P,
        compute: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E> {
        if let Some((cached, value)) = self.lock().get(&slot) {
            if *cached == params {
                return Ok(value.clone());
            }
        }
        let value = compute()?;
        self.lock().insert(slot, (params, value.clone()));
        Ok(value)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<K, (P, V)>> {
        // A panic while holding the lock cannot leave a map entry half written.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Scatter plot parameters: x column (`None` for the row number), y column,
/// point limit and axis scales.
pub(crate) type ScatterParams = (Option<ColumnId>, ColumnId, u32, AxisScale, AxisScale);

/// Every analysis of one filtered view. Analyses do not depend on row order,
/// so every ordering of the view shares them.
#[derive(Default)]
pub(crate) struct Analyses {
    pub(crate) null_counts: Slots<ColumnId, (), u64>,
    pub(crate) statistics: Slots<ColumnId, (), NumericStatistics>,
    pub(crate) distributions: Slots<ColumnId, (u32, AxisScale), Histogram>,
    pub(crate) values: Slots<ColumnId, (), ValueSummary>,
    pub(crate) lengths: Slots<ColumnId, u32, LengthSummary>,
    pub(crate) scatter: Slots<(), ScatterParams, ScatterResult>,
    pub(crate) filter_choices: Slots<ColumnId, (), FilterChoices>,
    pub(crate) filter_distributions: Slots<ColumnId, (u32, AxisScale), Option<RangeHistogram>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_keeps_one_result_and_recomputes_for_new_parameters() {
        let slots = Slots::<u8, u32, u32>::default();
        let mut computed = 0;
        let mut get = |params| {
            slots
                .get_or_compute(0, params, || {
                    computed += 1;
                    Ok::<_, ()>(params * 10)
                })
                .unwrap()
        };
        assert_eq!([get(1), get(1), get(2), get(1)], [10, 10, 20, 10]);
        assert_eq!(computed, 3);
        assert_eq!(slots.lock().len(), 1);
        assert!(slots.get_or_compute(1, 0, || Err(())).is_err());
        assert_eq!(slots.lock().len(), 1, "failures are not kept");
    }
}
