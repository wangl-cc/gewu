//! Bounded string candidate pages, with exact nullable text identities.

use std::sync::Arc;

use gewu_core as core;

use crate::GewuError;

#[derive(uniffi::Record)]
pub struct TextChoice {
    pub value: String,
    pub count: u64,
}

#[derive(uniffi::Record)]
pub struct TextChoiceCursor {
    pub value: String,
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
    /// Call off the UI thread. Pages contain non-null strings in UTF-8 order.
    pub fn page(
        &self,
        query: String,
        after: Option<TextChoiceCursor>,
        limit: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<TextChoicePage, GewuError> {
        let after = after.map(|cursor| core::TextChoiceCursor {
            value: cursor.value,
        });
        let page = self
            .inner
            .page(&query, after.as_ref(), limit, &cancellation.inner)?;
        Ok(page.into())
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
