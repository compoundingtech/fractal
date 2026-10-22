//! The part of `store::published_views` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_attention::store::published_views::*;
pub use fractal_attention::store::published_views::{PUBLISHED_COLLECTIONS, PublishedViews, Revisions, View, revision};

pub trait PublishedViewsStore {
    /// Say that `collection`'s running refresher governs its view while it has nothing to serve:
    /// see [`PublishedViews::hold`].
    fn hold_collection_view(&self, collection: &str);
    /// Whether a refresher serves or governs `collection`'s view: its windows then reread only
    /// when the view changes again.
    fn collection_view_published(&self, collection: &str) -> bool;
    /// Follows every collection's publication revision.
    fn subscribe_collection_views(&self) -> tokio::sync::watch::Receiver<Revisions>;
}

impl PublishedViewsStore for Store {

    fn hold_collection_view(&self, collection: &str) {
        self.smalltalk.published_views.hold(collection);
    }

    fn collection_view_published(&self, collection: &str) -> bool {
        self.smalltalk.published_views.published(collection)
    }

    fn subscribe_collection_views(&self) -> tokio::sync::watch::Receiver<Revisions> {
        self.smalltalk.published_views.subscribe()
    }
}

