use std::sync::Arc;

/// Create before queueing work; cancelling is terminal and thread-safe.
#[derive(Default, uniffi::Object)]
pub struct Cancellation {
    pub(crate) inner: gewu_core::Cancellation,
}

#[uniffi::export]
impl Cancellation {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }
}
