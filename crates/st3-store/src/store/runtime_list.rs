//! Complete current runtime resources, published off the HTTP request path.
use super::*;

pub struct RuntimeRows {
    pub items: Arc<Vec<Value>>,
    /// Computed once by the refresher, never by a page read.
    pub digest: String,
    pub time_unix_ms: u128,
}

impl Store {
    pub fn published_runtime_list(&self) -> &published_list::PublishedList<RuntimeRows> {
        &self.smalltalk.published_runtimes
    }
}
