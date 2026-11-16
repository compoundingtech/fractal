//! The part of `store::published_views` that moved up from fractal-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_seats::store::published_views::*;
pub use fractal_seats::store::published_views::{PUBLISHED_COLLECTIONS, PublishedViews, Revisions, View, revision};

pub trait PublishedViewsAttentionStore {
    /// Say that `collection`'s refresher has swapped in a newer published view. Call it after
    /// every publication, including one at the same graph index after a deadline passed.
    #[cfg_attr(not(test), allow(dead_code, reason = "the missions, work and attention refreshers call it"))]
    fn publish_collection_view(&self, collection: &str);
    /// Say that `collection`'s refresher stopped serving its view, because it exited or keeps
    /// failing: its windows go back to following commits until it publishes again.
    #[cfg_attr(not(test), allow(dead_code, reason = "the missions, work and attention refreshers call it"))]
    fn withdraw_collection_view(&self, collection: &str);
}

impl PublishedViewsAttentionStore for Store {

    #[cfg_attr(not(test), allow(dead_code, reason = "the missions, work and attention refreshers call it"))]
    fn publish_collection_view(&self, collection: &str) {
        self.fractal.published_views.publish(collection);
    }

    #[cfg_attr(not(test), allow(dead_code, reason = "the missions, work and attention refreshers call it"))]
    fn withdraw_collection_view(&self, collection: &str) {
        self.fractal.published_views.withdraw(collection);
    }
}

