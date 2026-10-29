//! The part of `maintenance` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::ProjectionDivergenceStore;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::store::Store;
pub use fractal_store::maintenance::*;
pub use fractal_store::maintenance::{DIVERGENCE_CHECK_ENV, EventMigrationReport, divergence_check_interval, migrate_event_payloads};

/// The first wait while the daemon is busy; it doubles until the next check is due.
const DIVERGENCE_BUSY_BACKOFF: Duration = Duration::from_secs(60);

/// Whether the daemon is too busy for background checks: the store's writer or a write
/// transaction ran over its target in the last minute, or replication is catching up or has
/// put projection off.
pub fn daemon_busy(store: &Store) -> bool {
    use smallclaims::windows::{StoreWork, store_series};
    let now = Instant::now();
    let max_ms = crate::slo::targets().transaction.max_ms as f64;
    let over = |kind| {
        let minute = &store_series(kind).snapshot(now)["1m"];
        minute["over_target"].as_u64().unwrap_or(0) > 0
            || minute["max_ms"].as_f64().unwrap_or(0.0) > max_ms
    };
    over(StoreWork::WriterHold)
        || over(StoreWork::WriteTransaction)
        || store.replication_catching_up()
        || store.replication_projection_deferred()
}

/// Look for projection divergences every `interval`, skipping a round the daemon stays busy
/// through. Each check reads comparisons exchanges already made and the open faults; it records a
/// fault for each new divergence and ends each fault whose members agree again.
pub async fn check_projection_divergences(store: Arc<Store>, interval: Duration) {
    store.capture_divergences(true);
    loop {
        tokio::time::sleep(interval).await;
        let mut backoff = DIVERGENCE_BUSY_BACKOFF;
        while daemon_busy(&store) && backoff < interval {
            tokio::time::sleep(backoff).await;
            backoff *= 2;
        }
        if daemon_busy(&store) {
            continue;
        }
        let checked = store.clone();
        let started = Instant::now();
        let result =
            tokio::task::spawn_blocking(move || checked.check_projection_divergences()).await;
        let elapsed = started.elapsed();
        match result {
            Ok(Ok(check)) if check.opened + check.recovered > 0 => eprintln!(
                "st3: projection divergence check in {elapsed:?}: {} divergent, {} opened, {} ended",
                check.divergences, check.opened, check.recovered
            ),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => eprintln!("st3: projection divergence check failed: {error}"),
            Err(error) => eprintln!("st3: projection divergence check stopped: {error}"),
        }
    }
}
