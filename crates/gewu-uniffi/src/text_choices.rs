//! Bounded string candidate pages, with exact nullable text identities.

use gewu_core as core;

use crate::GewuError;

#[derive(uniffi::Record)]
pub struct TextChoice {
    pub value: Option<String>,
    pub count: u64,
}

#[derive(uniffi::Record)]
pub struct TextChoiceCursor {
    pub value: Option<String>,
}

#[derive(uniffi::Record)]
pub struct TextChoicePage {
    pub values: Vec<TextChoice>,
    pub next: Option<TextChoiceCursor>,
}

#[derive(uniffi::Object)]
pub struct TextChoices {
    pub(crate) inner: core::TextChoices,
}

#[uniffi::export]
impl TextChoices {
    /// Call off the UI thread. Pages use null-first literal string ordering.
    pub fn page(
        &self,
        query: String,
        after: Option<TextChoiceCursor>,
        limit: u32,
    ) -> Result<TextChoicePage, GewuError> {
        let after = after.map(|cursor| core::TextChoiceCursor {
            value: cursor.value,
        });
        let page = self.inner.page(&query, after.as_ref(), limit)?;
        Ok(page.into())
    }

    /// Cancel obsolete pages without discarding the captured facet. Capture
    /// the returned generation before queueing the replacement request.
    pub fn invalidate_pages(&self) -> u64 {
        self.inner.invalidate_pages()
    }

    /// A queued request must use its captured generation, never acquire a new
    /// one when it finally runs. Stale requests cannot invalidate newer pages.
    pub fn page_for_generation(
        &self,
        query: String,
        after: Option<TextChoiceCursor>,
        limit: u32,
        generation: u64,
    ) -> Result<TextChoicePage, GewuError> {
        let after = after.map(|cursor| core::TextChoiceCursor {
            value: cursor.value,
        });
        Ok(self
            .inner
            .page_for_generation(&query, after.as_ref(), limit, generation)?
            .into())
    }

    /// Terminal cancellation for the session, not just the current page.
    pub fn cancel(&self) {
        self.inner.cancel();
    }
}

impl From<core::TextChoicePage> for TextChoicePage {
    fn from(page: core::TextChoicePage) -> Self {
        Self {
            values: page
                .values
                .into_iter()
                .map(|value| TextChoice {
                    value: value.value,
                    count: value.count,
                })
                .collect(),
            next: page.next.map(|cursor| TextChoiceCursor {
                value: cursor.value,
            }),
        }
    }
}
