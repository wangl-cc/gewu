//! Bounded string choices over an immutable source column and captured facet.
//!
//! Pages use value cursors, not row offsets: repeated values cannot reappear on
//! later pages. Each page scans the source but retains at most `limit + 1`
//! borrowed candidates, without building a document-wide distinct index.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use arrow::array::{Array, ArrayRef};
use regex::RegexBuilder;

use crate::{column::filter::text, scope::Selection, Error, LogicalType, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoice {
    pub value: Option<String>,
    /// Rows passing the other columns' explicit filters.
    pub count: u64,
}

/// An exclusive keyset cursor. An absent cursor starts at the beginning;
/// a cursor containing `None` starts immediately after the null value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoiceCursor {
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoicePage {
    /// Source values in null-first, case-sensitive UTF-8 lexical order.
    pub values: Vec<TextChoice>,
    /// Present only when another matching distinct value follows this page.
    pub next: Option<TextChoiceCursor>,
}

/// Reusable candidate query for one captured facet. Run pages off the UI thread.
/// Cancellation is independent of table queries. `cancel` closes the session;
/// `invalidate_pages` cancels obsolete pages while retaining the facet.
pub struct TextChoices {
    array: ArrayRef,
    rows: Selection,
    cancelled: AtomicBool,
    generation: AtomicU64,
}

impl TextChoices {
    /// Facet row IDs must be unique and ascending, as returned by Session's
    /// source-order filter scan. The selection and Arrow buffers are shared.
    pub(crate) fn new(array: ArrayRef, rows: Selection) -> Result<Self> {
        if LogicalType::from_arrow(array.data_type()) != LogicalType::Text {
            return Err(Error::InvalidArgument(format!(
                "{} is not a string column",
                array.data_type()
            )));
        }
        if let Selection::Rows(selected) = &rows {
            debug_assert!(
                selected.windows(2).all(|pair| pair[0] < pair[1])
                    && selected
                        .last()
                        .is_none_or(|&row| (row as usize) < array.len()),
                "facet rows must be unique, ascending source rows"
            );
        }
        Ok(Self {
            array,
            rows,
            cancelled: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Invalidate all previously captured page generations and return the new
    /// one. Capture this when accepting a request, before queueing its scan.
    /// Executing a queued page never advances this generation.
    pub fn invalidate_pages(&self) -> u64 {
        self.generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1)
    }

    fn check(&self, generation: u64) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed)
            || self.generation.load(Ordering::SeqCst) != generation
        {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Return up to `limit` values in bytewise string order, with null first.
    /// `after` is exclusive, and a cursor whose value is null means "after null".
    /// Search is a case-insensitive literal substring; null is searched as
    /// `(null)`. Candidates include zero-count values; counts use the captured
    /// facet. Limits outside 1..=200 are rejected. There is no full distinct count.
    pub fn page(
        &self,
        query: &str,
        after: Option<&TextChoiceCursor>,
        limit: u32,
    ) -> Result<TextChoicePage> {
        self.page_for_generation(query, after, limit, self.generation.load(Ordering::SeqCst))
    }

    /// Scan only while the captured generation remains current. A stale page
    /// returns `Cancelled` without invalidating any newer page or the session.
    pub fn page_for_generation(
        &self,
        query: &str,
        after: Option<&TextChoiceCursor>,
        limit: u32,
        generation: u64,
    ) -> Result<TextChoicePage> {
        self.page_checked(query, after, limit, || self.check(generation))
    }

    fn page_checked(
        &self,
        query: &str,
        after: Option<&TextChoiceCursor>,
        limit: u32,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<TextChoicePage> {
        check()?;
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidArgument(
                "text choice page size must be between 1 and 200".into(),
            ));
        }
        let matcher = if query.is_empty() {
            None
        } else {
            Some(
                RegexBuilder::new(&regex::escape(query))
                    .case_insensitive(true)
                    .build()
                    .map_err(|error| {
                        Error::InvalidArgument(format!("cannot prepare text search: {error}"))
                    })?,
            )
        };
        let read = text::accessor(&self.array)?;
        let mut counted_rows = self.rows.iter().peekable();
        let keep = limit as usize + 1;
        let mut candidates: Vec<(Option<&str>, u64)> = Vec::with_capacity(keep);
        for row in 0..self.array.len() {
            if row.is_multiple_of(4096) {
                check()?;
            }
            let counted = counted_rows.next_if_eq(&row).is_some();
            let value = read(row);
            if after.is_some_and(|cursor| value <= cursor.value.as_deref())
                || matcher
                    .as_ref()
                    .is_some_and(|matcher| !matcher.is_match(value.unwrap_or("(null)")))
            {
                continue;
            }
            if candidates.len() == keep && candidates.last().is_some_and(|(last, _)| value > *last)
            {
                continue;
            }
            match candidates.binary_search_by_key(&value, |&(key, _)| key) {
                Ok(index) => candidates[index].1 += u64::from(counted),
                Err(index) => {
                    // Evict before insertion to reuse the one bounded buffer.
                    if candidates.len() == keep {
                        candidates.pop();
                    }
                    candidates.insert(index, (value, u64::from(counted)));
                }
            }
            // The largest retained value only decreases. An evicted value can
            // never return, so every final candidate has counted all occurrences.
        }
        check()?;
        let has_more = candidates.len() > limit as usize;
        if has_more {
            candidates.pop();
        }
        let values: Vec<_> = candidates
            .into_iter()
            .map(|(value, count)| TextChoice {
                value: value.map(str::to_owned),
                count,
            })
            .collect();
        let next = has_more.then(|| TextChoiceCursor {
            value: values
                .last()
                .expect("nonempty page when another value exists")
                .value
                .clone(),
        });
        Ok(TextChoicePage { values, next })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{DictionaryArray, Int8Array, StringArray},
        datatypes::Int8Type,
    };

    use super::*;

    #[test]
    fn invalidating_a_running_page_preserves_the_session_and_rejects_queued_pages() {
        let array: ArrayRef = Arc::new(StringArray::from_iter_values(
            (0..12_000).map(|row| format!("value-{row:05}")),
        ));
        let choices = TextChoices::new(array, Selection::All(12_000)).unwrap();
        let obsolete = choices.invalidate_pages();
        let (reached, running) = std::sync::mpsc::sync_channel(0);
        let (resume, resumed) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let choices = &choices;
            let worker = scope.spawn(move || {
                let mut checks = 0;
                let result = choices.page_checked("", None, 10, || {
                    checks += 1;
                    // Start + row zero + row 4096: pause an actual partial scan.
                    if checks == 3 {
                        reached.send(()).unwrap();
                        resumed.recv().unwrap();
                    }
                    choices.check(obsolete)
                });
                assert_eq!(checks, 3, "obsolete scan must stop at its next checkpoint");
                result
            });
            running.recv().unwrap();
            let current = choices.invalidate_pages();
            resume.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
            assert!(matches!(
                choices.page_for_generation("", None, 10, obsolete),
                Err(Error::Cancelled)
            ));
            let page = choices
                .page_for_generation("value-11999", None, 10, current)
                .unwrap();
            assert_eq!(page.values, [TextChoice {
                value: Some("value-11999".into()),
                count: 1
            }]);
            choices.cancel();
            let after_close = choices.invalidate_pages();
            assert!(matches!(
                choices.page_for_generation("", None, 10, after_close),
                Err(Error::Cancelled)
            ));
        });
    }

    #[test]
    fn keyset_pages_deduplicate_and_keep_complete_facet_counts() {
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("z"),
            Some("b"),
            Some("a"),
            Some("z"),
            None,
            Some("b"),
            Some("c"),
            Some("a"),
            Some(""),
        ]));
        let choices =
            TextChoices::new(array, Selection::Rows(Arc::new(vec![0, 1, 2, 3, 5, 7]))).unwrap();
        let mut cursor = None;
        let mut found = Vec::new();
        loop {
            let page = choices.page("", cursor.as_ref(), 2).unwrap();
            assert!(page.values.len() <= 2);
            found.extend(
                page.values
                    .into_iter()
                    .map(|value| (value.value, value.count)),
            );
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(found, vec![
            (None, 0),
            (Some("".into()), 0),
            (Some("a".into()), 2),
            (Some("b".into()), 2),
            (Some("c".into()), 0),
            (Some("z".into()), 2)
        ]);
    }

    #[test]
    fn literal_search_null_cursor_and_empty_results_are_distinct() {
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            None,
            Some(""),
            Some("Kilo"),
            Some("kilo"),
            Some("a%_[b"),
            Some("(null)"),
        ]));
        let choices = TextChoices::new(array, Selection::All(6)).unwrap();
        let first = choices.page("", None, 1).unwrap();
        assert_eq!(first.values[0].value, None);
        let next = choices.page("", first.next.as_ref(), 1).unwrap();
        assert_eq!(next.values[0].value.as_deref(), Some(""));
        assert_eq!(choices.page("K", None, 10).unwrap().values.len(), 2);
        assert_eq!(
            choices.page("%_[", None, 10).unwrap().values[0]
                .value
                .as_deref(),
            Some("a%_[b")
        );
        let nulls = choices.page("(NULL)", None, 10).unwrap();
        assert_eq!(nulls.values.len(), 2);
        assert_eq!(nulls.values[0].value, None);
        let missing = choices.page("absent", None, 10).unwrap();
        assert!(missing.values.is_empty());
        assert!(missing.next.is_none());
        for limit in [0, 201] {
            assert!(matches!(
                choices.page("", None, limit),
                Err(Error::InvalidArgument(_))
            ));
        }
        choices.cancel();
        assert!(matches!(choices.page("", None, 1), Err(Error::Cancelled)));
    }

    #[test]
    fn dictionary_slices_merge_null_keys_and_values() {
        let values: ArrayRef = Arc::new(StringArray::from(vec![
            Some("unused"),
            Some("b"),
            None,
            Some("a"),
        ]));
        let array = DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), Some(1), None, Some(2), Some(3), Some(1)]),
            values,
        )
        .unwrap();
        let array: ArrayRef = Arc::new(array.slice(1, 5));
        let choices = TextChoices::new(array, Selection::All(5)).unwrap();
        let page = choices.page("", None, 10).unwrap();
        assert_eq!(
            page.values
                .iter()
                .map(|value| (value.value.as_deref(), value.count))
                .collect::<Vec<_>>(),
            [(None, 2), (Some("a"), 1), (Some("b"), 2)]
        );
    }
}
