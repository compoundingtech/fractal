//! Observe completed local GET-agents requests on the same boundary as HTTP telemetry.
//! This shadow meter is process-local. A restart or missing boundary leaves unknown minutes.
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use crate::otel::RequestOutcome;

const MINUTE_MS: u64 = 60_000;
const WINDOW: u64 = 7 * 24 * 60;
const CAPACITY: usize = WINDOW as usize + 1;
const FAST_LIMIT: Duration = Duration::from_millis(100);
const GOAL_PERCENT: u128 = 95;

#[derive(Clone, Default)]
struct Minute {
    slot: u64,
    complete: bool,
    eligible: u64,
    fast: u64,
}

struct Boundary {
    slot: u64,
    at: Instant,
    aligned: bool,
}

struct Meter {
    minutes: Box<[Minute]>,
    boundary: Option<Boundary>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            minutes: vec![Minute::default(); CAPACITY].into_boxed_slice(),
            boundary: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Verdict {
    Met,
    Missed,
    Unknown,
}

#[derive(Serialize)]
pub(super) struct Reading {
    through_unix_ms: u64,
    window_minutes: u64,
    covered_minutes: u64,
    eligible: u128,
    fast: u128,
    verdict: Verdict,
}

fn count(method: &str, route: &str, outcome: RequestOutcome, elapsed: Duration) -> Option<bool> {
    if (method, route) != ("GET", "/v1/client/agents") {
        return None;
    }
    match outcome {
        RequestOutcome::Ok => Some(elapsed <= FAST_LIMIT),
        RequestOutcome::ServerError | RequestOutcome::Deadline => Some(false),
        RequestOutcome::Cancelled | RequestOutcome::ClientError => None,
    }
}

impl Meter {
    fn minute(&mut self, slot: u64) -> &mut Minute {
        let minute = &mut self.minutes[(slot % CAPACITY as u64) as usize];
        if minute.slot != slot {
            *minute = Minute {
                slot,
                ..Minute::default()
            };
        }
        minute
    }

    fn record(&mut self, slot: u64, fast: bool) {
        let minute = self.minute(slot);
        if !minute.complete {
            minute.eligible += 1;
            minute.fast += u64::from(fast);
        }
    }

    fn boundary(&mut self, now_ms: u64, at: Instant) {
        let slot = now_ms / MINUTE_MS;
        // The startup call is a partial boundary even if startup coincides with the UTC edge.
        let aligned = self.boundary.is_some() && now_ms % MINUTE_MS <= 1_000;
        if let Some(previous) = self.boundary.replace(Boundary { slot, at, aligned }) {
            if slot < previous.slot {
                // A backwards wall-clock step would give new requests old slot identities.
                self.minutes.fill(Minute::default());
            } else if slot == previous.slot + 1
                && previous.aligned
                && aligned
                && (Duration::from_secs(59)..=Duration::from_secs(61))
                    .contains(&at.duration_since(previous.at))
            {
                self.minute(previous.slot).complete = true;
            }
        }
    }

    fn read(&self, end: u64) -> Reading {
        let mut reading = Reading {
            through_unix_ms: end * MINUTE_MS,
            window_minutes: WINDOW,
            covered_minutes: 0,
            eligible: 0,
            fast: 0,
            verdict: Verdict::Unknown,
        };
        for minute in &self.minutes {
            if minute.complete && (end.saturating_sub(WINDOW)..end).contains(&minute.slot) {
                reading.covered_minutes += 1;
                reading.eligible += u128::from(minute.eligible);
                reading.fast += u128::from(minute.fast);
            }
        }
        if reading.covered_minutes == WINDOW && reading.eligible > 0 {
            reading.verdict = if reading.fast * 100 >= reading.eligible * GOAL_PERCENT {
                Verdict::Met
            } else {
                Verdict::Missed
            };
        }
        reading
    }
}

static METER: LazyLock<Mutex<Meter>> = LazyLock::new(|| Mutex::new(Meter::default()));

pub(super) fn record(method: &str, route: &str, outcome: RequestOutcome, elapsed: Duration) {
    if let Some(fast) = count(method, route, outcome, elapsed) {
        METER
            .lock()
            .record(super::client_now_ms() as u64 / MINUTE_MS, fast);
    }
}

pub(super) fn boundary(now_ms: u64, at: Instant) {
    METER.lock().boundary(now_ms, at);
}

pub(super) fn read(now_ms: u64) -> Reading {
    METER.lock().read(now_ms / MINUTE_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_exact_route_and_inclusive_latency_boundary() {
        let classify = |outcome, elapsed| count("GET", "/v1/client/agents", outcome, elapsed);
        assert_eq!(classify(RequestOutcome::Ok, FAST_LIMIT), Some(true));
        assert_eq!(
            classify(RequestOutcome::Ok, FAST_LIMIT + Duration::from_nanos(1)),
            Some(false)
        );
        for outcome in [RequestOutcome::ServerError, RequestOutcome::Deadline] {
            assert_eq!(classify(outcome, Duration::ZERO), Some(false));
        }
        for outcome in [RequestOutcome::Cancelled, RequestOutcome::ClientError] {
            assert_eq!(classify(outcome, Duration::ZERO), None);
        }
        assert_eq!(
            count(
                "POST",
                "/v1/client/agents",
                RequestOutcome::Ok,
                Duration::ZERO
            ),
            None
        );
        assert_eq!(
            count(
                "GET",
                "/v1/client/agents/{id}",
                RequestOutcome::Ok,
                Duration::ZERO
            ),
            None
        );
    }

    #[test]
    fn complete_idle_minutes_count_as_coverage_but_partial_and_skipped_do_not() {
        let mut meter = Meter::default();
        let at = Instant::now();
        meter.boundary(20_000, at);
        meter.record(0, false);
        meter.boundary(60_005, at + Duration::from_secs(40));
        meter.record(1, true);
        meter.boundary(120_005, at + Duration::from_secs(100));
        meter.boundary(240_005, at + Duration::from_secs(220));
        meter.boundary(300_005, at + Duration::from_secs(280));
        let reading = meter.read(5);
        assert_eq!(
            (reading.covered_minutes, reading.eligible, reading.fast),
            (2, 1, 1)
        );
        assert_eq!(reading.verdict, Verdict::Unknown);
    }

    #[test]
    fn requires_full_window_and_nonzero_eligible_traffic() {
        let mut meter = Meter::default();
        let at = Instant::now();
        for slot in 0..=WINDOW + 1 {
            meter.boundary(slot * MINUTE_MS, at + Duration::from_secs(slot * 60));
        }
        assert_eq!(meter.read(WINDOW + 1).verdict, Verdict::Unknown);

        let mut meter = Meter::default();
        for slot in 0..=WINDOW + 2 {
            meter.boundary(slot * MINUTE_MS, at + Duration::from_secs(slot * 60));
            if slot == 1 {
                meter.record(slot, false);
            }
            if slot == 2 {
                for _ in 0..19 {
                    meter.record(slot, true);
                }
            }
            if slot == WINDOW + 1 {
                meter.record(slot, false);
                meter.record(slot, false);
            }
        }
        assert_eq!(meter.read(WINDOW).verdict, Verdict::Unknown);
        assert_eq!(meter.read(WINDOW + 1).verdict, Verdict::Met);
        assert_eq!(meter.read(WINDOW + 2).eligible, 21);
        assert_eq!(meter.read(WINDOW + 2).verdict, Verdict::Missed);
        assert_eq!(Meter::default().read(WINDOW + 2).verdict, Verdict::Unknown);
    }
}
