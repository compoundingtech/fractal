//! A projection divergence is a bug, never a heal. Projections are pure functions of the claims,
//! so two members that hold the same envelopes and project one table differently have found a
//! projection bug. It is recorded once, as a P1 operational fault for the fleet's fault agent,
//! with what a reproduction needs. Nothing repairs it from a peer; the fault ends when a later
//! comparison at the same envelopes finds the two members agree.
//!
//! Looking for one stays rare and cheap. The check is off unless the daemon sets
//! `ST3_DIVERGENCE_CHECK_INTERVAL_MS`, runs at most hourly, reads only the digest comparisons
//! exchanges already made (in memory) and one indexed read of the divergence faults on the
//! reader pool, and skips while the daemon is busy. It writes only to open or end a fault, on the
//! recording member's own `daemon/NODE` subject.
use super::*;
use smallclaims::replication::ProjectionDivergence;

/// Every divergence episode starts with this, so its failure and recovery read apart from the
/// daemon's other faults.
pub const EPISODE_PREFIX: &str = "projection-divergence/";

/// One divergence per member pair and projection, whichever member records it first. A
/// divergence that lasts while the members keep writing stays the one episode.
pub fn episode(local: &str, divergence: &ProjectionDivergence) -> String {
    let (first, second) = if local <= divergence.peer.as_str() {
        (local, divergence.peer.as_str())
    } else {
        (divergence.peer.as_str(), local)
    };
    let digest = hex::encode(Sha256::digest(
        format!("{first}\n{second}\n{}", divergence.projection).as_bytes(),
    ));
    format!("{EPISODE_PREFIX}{}", &digest[..32])
}

impl Store {
    /// Open divergence faults at the last check, or `None` when this daemon has not checked.
    /// Reads nothing from the store, so the SLO can show it on every sample.
    pub fn open_divergences_at_last_check(&self) -> Option<u64> {
        *self.fractal.open_divergences.lock().unwrap()
    }

}
