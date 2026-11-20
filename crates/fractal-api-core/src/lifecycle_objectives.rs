//! Deterministic latency budgets from durable lifecycle facts, including unfinished work.
use serde_json::{Value, json};

use crate::declared_objectives::{BudgetEvidence, classify_budget};
use crate::objective::{BudgetPolicy, BurnWindow};

#[derive(Clone, Copy, Debug)]
pub(crate) enum LifecycleTerminal {
    Accepted(u64),
    Failed(u64),
    Open,
    Excluded,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LifecycleSample {
    pub onset_ms: u64,
    pub terminal: LifecycleTerminal,
}

impl LifecycleSample {
    // A late completion cannot move a failure out of its original deadline bucket.
    fn outcome(self, deadline_ms: u64, now: u64) -> Option<(u64, bool)> {
        if self.onset_ms > now { return None; }
        let deadline = self.onset_ms.saturating_add(deadline_ms);
        match self.terminal {
            LifecycleTerminal::Excluded => None,
            LifecycleTerminal::Accepted(at) if at >= self.onset_ms && at <= now && at <= deadline => Some((at, false)),
            LifecycleTerminal::Failed(at) if at >= self.onset_ms && at <= now => Some((at.min(deadline), true)),
            _ if deadline < now => Some((deadline, true)),
            _ => None,
        }
    }
}

/// `coverage_known` certifies the complete source population at this cut, not just readable samples.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ratio_row(
    id: &str,
    about: &str,
    now: u64,
    deadline_ms: u64,
    allowance: f64,
    samples: &[LifecycleSample],
    coverage_known: bool,
) -> Value {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const HORIZON: u64 = 30 * 24 * HOUR;
    let policy = BudgetPolicy {
        allowance, horizon_ms: HORIZON, min_coverage: 1.0, min_samples: 1,
        fast: BurnWindow { burn_rate: 14.4, long_ms: HOUR, short_ms: 5 * MINUTE },
        slow: BurnWindow { burn_rate: 6.0, long_ms: 6 * HOUR, short_ms: 30 * MINUTE },
    };
    let measure = |width: u64| {
        let from = now.saturating_sub(width);
        let (mut bad, mut total) = (0_u64, 0_u64);
        for sample in samples {
            if let Some((at, failed)) = sample.outcome(deadline_ms, now)
                && at >= from && at < now
            {
                total += 1;
                bad += u64::from(failed);
            }
        }
        BudgetEvidence { bad: bad as f64, total: total as f64, eligible: coverage_known && total > 0 }
    };
    let horizon = measure(HORIZON);
    let fast = (measure(policy.fast.long_ms), measure(policy.fast.short_ms));
    let slow = (measure(policy.slow.long_ms), measure(policy.slow.short_ms));
    let (status, remaining) = if coverage_known {
        classify_budget(&policy, horizon, fast, slow, "no-qualifying-events")
    } else { ("no-data", None) };
    let open = samples.iter().filter(|sample| matches!(sample.terminal, LifecycleTerminal::Open)).count();
    let excluded = samples.iter().filter(|sample| matches!(sample.terminal, LifecycleTerminal::Excluded)).count();
    let render_window = |evidence: BudgetEvidence| json!({
        "bad": evidence.bad, "total": evidence.total, "eligible": evidence.eligible,
        "burn_rate": (evidence.total > 0.0).then(|| evidence.bad / evidence.total / allowance),
    });
    json!({
        "id": id, "kind": "objective", "about": about,
        "target": format!("≥ {:.0}% accepted within {}s", (1.0 - allowance) * 100.0, deadline_ms / 1000),
        "current": format!("{} bad / {} eligible; {open} open; {excluded} excluded", horizon.bad, horizon.total),
        "window": "30d", "status": status, "remaining_percent": remaining,
        "evidence": {
            "as_of_unix_ms": now, "coverage_known": coverage_known,
            "bad": horizon.bad, "total": horizon.total, "open": open, "excluded": excluded,
            "fast": { "long": render_window(fast.0), "short": render_window(fast.1) },
            "slow": { "long": render_window(slow.0), "short": render_window(slow.1) },
        },
        "explain": "Durable lifecycle facts; unfinished or late work fails once at its deadline, without moving the failure bucket after eventual acceptance. Early terminal failures fail at their terminal timestamp. Inclusive latency deadline, half-open accounting windows. Missing source coverage is unknown, never recovery. 30d budget; paired burns 14.4x (1h+5m), 6x (6h+30m); minimum one sample.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_and_late_work_share_the_same_failure_bucket() {
        let open = LifecycleSample { onset_ms: 100, terminal: LifecycleTerminal::Open };
        let late = LifecycleSample { terminal: LifecycleTerminal::Accepted(500), ..open };
        assert_eq!(open.outcome(200, 600), Some((300, true)));
        assert_eq!(late.outcome(200, 600), Some((300, true)));
        assert_eq!(open.outcome(200, 300), None);
        assert_eq!(LifecycleSample { terminal: LifecycleTerminal::Accepted(300), ..open }.outcome(200, 301), Some((300, false)));
    }

    #[test]
    fn terminal_failure_is_not_a_successful_stop() {
        let sample = LifecycleSample { onset_ms: 100, terminal: LifecycleTerminal::Failed(150) };
        assert_eq!(sample.outcome(200, 600), Some((150, true)));
        assert_eq!(LifecycleSample { terminal: LifecycleTerminal::Excluded, ..sample }.outcome(200, 600), None);
    }

    #[test]
    fn missing_coverage_cannot_recover_or_report_remaining_budget() {
        let samples = [LifecycleSample { onset_ms: 100, terminal: LifecycleTerminal::Accepted(150) }];
        let row = ratio_row("acme/latency", "test", 600, 200, 0.05, &samples, false);
        assert_eq!(row["status"], "no-data");
        assert!(row["remaining_percent"].is_null());
        assert_eq!(row["evidence"]["total"], 1.0);
    }

    #[test]
    fn paired_burns_use_their_own_windows_not_the_horizon_fraction() {
        const HOUR: u64 = 3_600_000;
        let now = 10 * HOUR;
        let mut samples = vec![LifecycleSample {
            onset_ms: HOUR,
            terminal: LifecycleTerminal::Accepted(HOUR + 1),
        }; 100];
        samples.push(LifecycleSample { onset_ms: now - 100, terminal: LifecycleTerminal::Failed(now - 50) });
        let row = ratio_row("acme/latency", "test", now, 200, 0.05, &samples, true);
        assert_eq!(row["status"], "burning");
        assert_eq!(row["evidence"]["bad"], 1.0);
        assert_eq!(row["evidence"]["total"], 101.0);
        assert_eq!(row["evidence"]["fast"]["short"]["total"], 1.0);
    }

    #[test]
    fn an_open_failure_is_counted_once_and_ages_out_without_becoming_good() {
        const DAY: u64 = 86_400_000;
        let sample = LifecycleSample { onset_ms: 100, terminal: LifecycleTerminal::Open };
        let row = ratio_row("acme/latency", "test", 1_000, 200, 0.05, &[sample], true);
        assert_eq!(row["evidence"]["bad"], 1.0);
        assert_eq!(row["evidence"]["total"], 1.0);
        let old = ratio_row("acme/latency", "test", 31 * DAY, 200, 0.05, &[sample], true);
        assert_eq!(old["evidence"]["total"], 0.0);
        assert_eq!(old["evidence"]["open"], 1);
        assert_eq!(old["status"], "no-qualifying-events");
    }
}
