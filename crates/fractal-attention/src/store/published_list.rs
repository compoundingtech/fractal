//! The part of `store::published_list` that moved up from st3-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
pub use fractal_seats::store::published_list::*;
pub use fractal_seats::store::published_list::{ASKS_BY_REQUESTERS, ASKS_FROM_STEPS, ASK_ORIGIN_AND_RUN, CHILD_RUNS, CLAIMS_SINCE, ListRead, Publication, PublicationId, PublishedList, RUN_STEPS, decides_asks};

pub trait PublishedListStore {
    /// How far replicated claims are projected: claims after this index that arrived by
    /// replication may still wait in the backlog, and a pass that projects them moves it on. A
    /// list that folded past them before then reads their claims again once it moves.
    fn projection_frontier(&self) -> Result<u64>;
}

impl PublishedListStore for Store {

    fn projection_frontier(&self) -> Result<u64> {
        let connection = self.readers.get();
        let frontier = connection
            .prepare_cached(
                "SELECT COALESCE(MAX(last_good_store_index), 0) FROM projection_health
                 WHERE aggregate='graph'",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h46c1989a6246 owner=fractal_attention::store::published_list TODO=trace-statement-and-value-representation */ [], |row| row.get::<_, u64>(0))?;
        Ok(frontier)
    }
}

