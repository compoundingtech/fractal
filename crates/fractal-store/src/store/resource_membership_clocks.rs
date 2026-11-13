//! Timeout clocks are independent of telemetry and never constitute admission authority.
use super::{Store, Owner, authorized, fenced, read};
use std::collections::BTreeMap;
use std::time::Instant;

struct ClockAnchor {
    anchor: Instant,
    prior_ms: u64,
    uncertain: bool,
}

#[derive(Default)]
pub(in crate::store) struct Clocks {
    admission: BTreeMap<String, ClockAnchor>,
    execution: BTreeMap<String, BTreeMap<String, ClockAnchor>>,
}

impl Clocks {
    fn elapsed(&mut self, id: &str, queued_at: u64, now_ms: u64) -> (u64, bool) {
        if !self.admission.contains_key(id) {
            self.admission.insert(id.to_owned(), ClockAnchor {
                anchor: Instant::now(),
                prior_ms: now_ms.saturating_sub(queued_at),
                // Historical age is wall-clock evidence, not a monotonic observation.
                uncertain: now_ms != queued_at,
            });
        }
        let clock = self.admission.get_mut(id).expect("clock was just installed");
        clock.uncertain |= now_ms < queued_at;
        let elapsed = u64::try_from(clock.anchor.elapsed().as_millis()).unwrap_or(u64::MAX);
        (clock.prior_ms.saturating_add(elapsed), clock.uncertain)
    }
}

impl Store {
    /// The first observation rebases persisted age; later wall-clock changes cannot reset or
    /// accelerate this boot's queue budget. The caller also records authority-epoch uncertainty.
    pub(in crate::store) fn resource_admission_elapsed(&self, id: &str, queued_at: u64, now_ms: u64) -> (u64, bool) {
        let clocks = self.resource_clocks.get_or_init(Default::default);
        let mut clocks = clocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        clocks.elapsed(id, queued_at, now_ms)
    }

    /// Anchor each physical root at confirmed payload release, independently of signal export.
    pub(in crate::store) fn anchor_resource_execution_clock(&self, id: &str, runtime_id: &str, released_at: u64, now_ms: u64) -> (u64, bool) {
        let clocks = self.resource_clocks.get_or_init(Default::default);
        let mut clocks = clocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !clocks.execution.contains_key(id) { clocks.execution.insert(id.to_owned(), BTreeMap::new()); }
        let roots = clocks.execution.get_mut(id).expect("execution clocks were just installed");
        if !roots.contains_key(runtime_id) {
            roots.insert(runtime_id.to_owned(), ClockAnchor {
                anchor: Instant::now(), prior_ms: now_ms.saturating_sub(released_at), uncertain: now_ms < released_at,
            });
        }
        let clock = &roots[runtime_id];
        (clock.prior_ms.saturating_add(u64::try_from(clock.anchor.elapsed().as_millis()).unwrap_or(u64::MAX)), clock.uncertain)
    }

    pub fn resource_execution_elapsed(&self, id: &str, owner: &Owner, runtime_id: &str, now_ms: u64) -> anyhow::Result<Option<u64>> {
        let member = read(&self.readers.get(), id)?;
        authorized(&member, owner)?;
        if let Some(grant) = &member.grant { fenced(&self.readers.get(), &member, grant)?; }
        let root = member.roots.iter().find(|root| root.runtime_id == runtime_id)
            .ok_or_else(|| anyhow::anyhow!("execution root missing"))?;
        let Some(released_at) = root.payload_released_at else { return Ok(None); };
        if let Some(ended_at) = root.execution_ended_at {
            return ended_at.checked_sub(released_at).map(Some)
                .ok_or_else(|| anyhow::anyhow!("execution clock uncertain: end precedes confirmed payload release"));
        }
        let (elapsed, uncertain) = self.anchor_resource_execution_clock(id, runtime_id, released_at, now_ms);
        anyhow::ensure!(!uncertain, "execution clock uncertain: cannot reconstruct elapsed payload age from backwards wall time");
        Ok(Some(elapsed))
    }

    pub(in crate::store) fn finish_resource_execution_clock(&self, id: &str, runtime_id: &str) {
        if let Some(clocks) = self.resource_clocks.get() {
            let mut clocks = clocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(roots) = clocks.execution.get_mut(id) {
                roots.remove(runtime_id);
                if roots.is_empty() { clocks.execution.remove(id); }
            }
        }
    }

    #[cfg(test)]
    pub(in crate::store) fn advance_resource_admission_clock(&self, id: &str, elapsed_ms: u64) {
        let mut clocks = self.resource_clocks.get().unwrap().lock().unwrap();
        clocks.admission.get_mut(id).unwrap().prior_ms += elapsed_ms;
    }

    pub(in crate::store) fn finish_resource_admission_clock(&self, id: &str) {
        if let Some(clocks) = self.resource_clocks.get() {
            clocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).admission.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_changes_do_not_replace_a_live_monotonic_anchor() {
        let mut clocks = Clocks::default();
        let (initial, uncertain) = clocks.elapsed("request", 100, 100);
        assert!(!uncertain);
        let anchor = clocks.admission["request"].anchor;
        let (after_jump, uncertain) = clocks.elapsed("request", 100, u64::MAX);
        assert!(!uncertain);
        assert!(after_jump >= initial);
        assert_eq!(clocks.admission["request"].anchor, anchor);
        assert_eq!(clocks.admission["request"].prior_ms, 0);
        let (_, uncertain) = clocks.elapsed("request", 100, 99);
        assert!(uncertain);
    }

    #[test]
    fn restored_age_is_retained_and_backwards_wall_time_is_explicit() {
        let mut clocks = Clocks::default();
        let (elapsed, uncertain) = clocks.elapsed("restored", 100, 400);
        assert!(elapsed >= 300);
        assert!(uncertain);
        let (_, uncertain) = clocks.elapsed("backwards", 400, 100);
        assert_eq!(clocks.admission["backwards"].prior_ms, 0);
        assert!(uncertain);
    }
}
