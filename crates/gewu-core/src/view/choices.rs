//! Bounded string choices over an immutable source column and captured view.
//!
//! Pages use value cursors, not row offsets: repeated values cannot reappear on
//! later pages. Each page scans the selected rows but retains at most `limit + 1`
//! borrowed candidates, without building a document-wide distinct index.

use arrow::array::{Array, ArrayRef};
use regex::RegexBuilder;

use crate::{column::filter::text, scope::Selection, Cancellation, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoice {
    pub value: String,
    /// Rows passing the view's filters.
    pub count: u64,
}

/// An exclusive non-null keyset cursor. An absent cursor starts at the beginning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoiceCursor {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChoicePage {
    /// Non-null source values in case-sensitive UTF-8 lexical order.
    pub values: Vec<TextChoice>,
    /// Present only when another matching distinct value follows this page.
    pub next: Option<TextChoiceCursor>,
}

/// Reusable candidate query for one captured view. Run pages off the UI thread.
/// Cancellation is independent of table queries. `cancel` closes the session;
/// individual pages receive their own cancellation without discarding the captured data.
pub struct TextChoices {
    array: ArrayRef,
    rows: Selection,
    cancellation: Cancellation,
}

impl TextChoices {
    /// Capture the selected rows in any display order without copying Arrow buffers.
    pub(crate) fn new(array: ArrayRef, rows: Selection) -> Result<Self> {
        let is_text = crate::types::domain(array.data_type()) == Some(crate::types::Domain::Text);
        if !is_text {
            return Err(Error::InvalidArgument(format!(
                "{} is not a string column",
                array.data_type()
            )));
        }
        Ok(Self {
            array,
            rows,
            cancellation: Cancellation::default(),
        })
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Cancelling this request never affects a later page or its session.
    pub fn page(
        &self,
        query: &str,
        after: Option<&TextChoiceCursor>,
        limit: u32,
        cancel: &Cancellation,
    ) -> Result<TextChoicePage> {
        let token = self.cancellation.combined_with(cancel);
        self.page_checked(query, after, limit, || token.check())
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
        let keep = limit as usize + 1;
        let mut candidates: Vec<(&str, u64)> = Vec::with_capacity(keep);
        for (index, row) in self.rows.iter().enumerate() {
            if index.is_multiple_of(4096) {
                check()?;
            }
            let Some(value) = read(row) else { continue };
            if after.is_some_and(|cursor| value <= cursor.value.as_str())
                || matcher
                    .as_ref()
                    .is_some_and(|matcher| !matcher.is_match(value))
            {
                continue;
            }
            if candidates.len() == keep && candidates.last().is_some_and(|(last, _)| value > *last)
            {
                continue;
            }
            match candidates.binary_search_by_key(&value, |&(key, _)| key) {
                Ok(index) => candidates[index].1 += 1,
                Err(index) => {
                    // Evict before insertion to reuse the one bounded buffer.
                    if candidates.len() == keep {
                        candidates.pop();
                    }
                    candidates.insert(index, (value, 1));
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
                value: value.to_owned(),
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
        let obsolete = Cancellation::default();
        let (reached, running) = std::sync::mpsc::sync_channel(0);
        let (resume, resumed) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let choices = &choices;
            let obsolete = &obsolete;
            let worker = scope.spawn(move || {
                let mut checks = 0;
                let result = choices.page_checked("", None, 10, || {
                    checks += 1;
                    // Start + row zero + row 4096: pause an actual partial scan.
                    if checks == 3 {
                        reached.send(()).unwrap();
                        resumed.recv().unwrap();
                    }
                    choices.cancellation.check()?;
                    obsolete.check()
                });
                assert_eq!(checks, 3, "obsolete scan must stop at its next checkpoint");
                result
            });
            running.recv().unwrap();
            obsolete.cancel();
            let current = Cancellation::default();
            resume.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
            assert!(matches!(
                choices.page("", None, 10, obsolete),
                Err(Error::Cancelled)
            ));
            let page = choices.page("value-11999", None, 10, &current).unwrap();
            assert_eq!(page.values, [TextChoice {
                value: "value-11999".into(),
                count: 1
            }]);
            choices.cancel();
            let after_close = Cancellation::default();
            assert!(matches!(
                choices.page("", None, 10, &after_close),
                Err(Error::Cancelled)
            ));
        });
    }

    #[test]
    fn keyset_pages_count_only_selected_rows_in_any_display_order() {
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
            TextChoices::new(array, Selection::Rows(Arc::from(vec![7, 5, 3, 2, 1, 0]))).unwrap();
        let mut cursor = None;
        let mut found = Vec::new();
        loop {
            let page = choices
                .page("", cursor.as_ref(), 2, &Cancellation::default())
                .unwrap();
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
            ("a".into(), 2),
            ("b".into(), 2),
            ("z".into(), 2)
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
        let first = choices.page("", None, 1, &Cancellation::default()).unwrap();
        assert_eq!(first.values[0].value, "");
        let next = choices
            .page("", first.next.as_ref(), 1, &Cancellation::default())
            .unwrap();
        assert_eq!(next.values[0].value, "(null)");
        assert_eq!(
            choices
                .page("K", None, 10, &Cancellation::default())
                .unwrap()
                .values
                .len(),
            2
        );
        assert_eq!(
            choices
                .page("%_[", None, 10, &Cancellation::default())
                .unwrap()
                .values[0]
                .value
                .as_str(),
            "a%_[b"
        );
        let nulls = choices
            .page("(NULL)", None, 10, &Cancellation::default())
            .unwrap();
        assert_eq!(nulls.values.len(), 1);
        assert_eq!(nulls.values[0].value, "(null)");
        let missing = choices
            .page("absent", None, 10, &Cancellation::default())
            .unwrap();
        assert!(missing.values.is_empty());
        assert!(missing.next.is_none());
        for limit in [0, 201] {
            assert!(matches!(
                choices.page("", None, limit, &Cancellation::default()),
                Err(Error::InvalidArgument(_))
            ));
        }
        choices.cancel();
        assert!(matches!(
            choices.page("", None, 1, &Cancellation::default()),
            Err(Error::Cancelled)
        ));
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
        let page = choices
            .page("", None, 10, &Cancellation::default())
            .unwrap();
        assert_eq!(
            page.values
                .iter()
                .map(|value| (value.value.as_str(), value.count))
                .collect::<Vec<_>>(),
            [("a", 1), ("b", 2)]
        );
    }
}
