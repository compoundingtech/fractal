//! The part of `store::message_bodies` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::path::PathBuf;
use std::time::Duration;
use crate::message_body::{BodyFile, bodies, retention};
pub use fractal_store::store::message_bodies::*;

/// Remove the bodies in `state_dir` that no message claim here names for their sender, now and
/// once an hour while the daemon runs. A body is written only after its claim, so what this
/// finds is a body whose claim a checkpoint dropped, never a send in flight.
pub fn spawn_reconciler(store: std::sync::Arc<super::Store>, state_dir: PathBuf) {
    tokio::spawn(crate::task_cpu::account("message_body.reconcile", async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let (store, state_dir) = (store.clone(), state_dir.clone());
            let _ = tokio::task::spawn_blocking(move || crate::task_cpu::blocking("message_body.reconcile", || {
                let bodies = bodies(&state_dir);
                let keep = retention();
                bodies.reconcile(
                    |file| if body_is_pending(&store, file) { None } else { keep },
                    |file| body_is_named(&store, file),
                );
            }))
            .await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }));
}

/// Whether a delivered message or pending schedule still names this body. Failed reads
/// cannot establish that a body is orphaned.
pub fn body_is_named(store: &super::Store, file: &BodyFile) -> bool {
    match store.message(&format!("message/{}", file.id)) {
        Ok(None) => body_is_pending(store, file),
        Ok(Some(_)) | Err(_) => true,
    }
}

/// A pending durable schedule pins its body until delivery or cancellation. Failed reads
/// cannot establish that a pin ended, so the sweep leaves those files alone.
pub fn body_is_pending(store: &super::Store, file: &BodyFile) -> bool {
    let Some(hash) = file.id.strip_prefix("scheduled-") else { return false };
    match store.scheduled_message(&format!("message-schedule/{hash}")) {
        Ok(Some(schedule)) => schedule.state == "pending",
        Ok(None) => false,
        Err(_) => true,
    }
}
