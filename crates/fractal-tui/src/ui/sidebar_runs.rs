//! Related runs are read on attachment and graph frames, never on a timer.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Reads {
    epoch: u64,
    asked: BTreeMap<String, u64>,
    pending: BTreeSet<String>,
}
impl Reads {
    pub fn invalidate(&mut self) { self.epoch += 1; }
    pub fn reset(&mut self) { self.invalidate(); self.asked.clear(); }
    pub fn begin(&mut self, agent: &str) -> Option<u64> {
        if self.pending.contains(agent) || self.asked.get(agent) == Some(&self.epoch) { return None; }
        self.pending.insert(agent.into());
        self.asked.insert(agent.into(), self.epoch);
        Some(self.epoch)
    }
    pub fn finish(&mut self, agent: &str, epoch: u64) -> bool {
        self.pending.remove(agent);
        epoch == self.epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_frames_do_not_poll_and_graph_changes_coalesce() {
        let mut reads = Reads::default();
        let first = reads.begin("agent/example").unwrap();
        assert_eq!(reads.begin("agent/example"), None);
        reads.invalidate(); reads.invalidate();
        assert_eq!(reads.begin("agent/example"), None);
        assert!(!reads.finish("agent/example", first));
        let next = reads.begin("agent/example").unwrap();
        assert!(reads.finish("agent/example", next));
        assert_eq!(reads.begin("agent/example"), None);
    }
    #[test]
    fn reconnect_rejects_old_member_results_and_reads_again() {
        let mut reads = Reads::default();
        let old = reads.begin("agent/example").unwrap();
        reads.reset();
        assert!(!reads.finish("agent/example", old));
        assert!(reads.begin("agent/example").is_some());
    }
}
