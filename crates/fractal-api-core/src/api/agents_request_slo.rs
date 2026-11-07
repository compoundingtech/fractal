//! Local GET-agents counts from the HTTP telemetry boundary, retained by a single minute writer.
mod ledger;

use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use parking_lot::Mutex;
use serde::Serialize;

use crate::otel::RequestOutcome;
use ledger::Ledger;

const MINUTE_MS: u64 = 60_000;
const WINDOW: u64 = 7 * 24 * 60;
const FAST_LIMIT: Duration = Duration::from_millis(100);
const GOAL_PERCENT: u128 = 95;
const LEDGER_NAME: &str = "agents-request-slo-v1.sqlite3";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Receipt {
    slot: u64,
    eligible: u64,
    fast: u64,
}

#[derive(Clone, Copy)]
struct Boundary {
    slot: u64,
    at: Instant,
    aligned: bool,
}

#[derive(Default)]
struct Pending {
    serving: bool,
    minutes: [Option<Receipt>; 2],
    latest_slot: Option<u64>,
    rewind_from: Option<u64>,
}

impl Pending {
    fn start_serving(&mut self) {
        *self = Self {
            serving: true,
            ..Self::default()
        };
    }

    fn stop_serving(&mut self) {
        *self = Self::default();
    }

    fn observe_slot(&mut self, slot: u64) {
        if self.latest_slot.is_some_and(|latest| slot < latest) {
            self.minutes = [None; 2];
            self.rewind_from = Some(self.rewind_from.map_or(slot, |prior| prior.min(slot)));
        }
        self.latest_slot = Some(slot);
    }

    fn record(&mut self, slot: u64, observed_slot: u64, fast: bool) {
        if !self.serving {
            return;
        }
        self.observe_slot(observed_slot);
        if !(observed_slot.saturating_sub(1)..=observed_slot).contains(&slot) {
            return;
        }
        let index = (slot % 2) as usize;
        let receipt = match &mut self.minutes[index] {
            Some(receipt) if receipt.slot == slot => receipt,
            entry => entry.insert(Receipt {
                slot,
                ..Receipt::default()
            }),
        };
        receipt.eligible += 1;
        receipt.fast += u64::from(fast);
    }

    fn boundary(
        &mut self,
        previous: Option<Boundary>,
        now_ms: u64,
        at: Instant,
        in_flight: bool,
    ) -> (Boundary, Option<Receipt>, Option<u64>) {
        let slot = now_ms / MINUTE_MS;
        self.observe_slot(slot);
        let rewind = self.rewind_from.take();
        let boundary = Boundary {
            slot,
            at,
            // Even an aligned startup tick starts a partial minute.
            aligned: self.serving && previous.is_some() && now_ms % MINUTE_MS <= 1_000,
        };
        let receipt = previous
            .filter(|previous| {
                !in_flight
                    && rewind.is_none()
                    && slot == previous.slot + 1
                    && previous.aligned
                    && boundary.aligned
                    && (Duration::from_secs(59)..=Duration::from_secs(61))
                        .contains(&at.duration_since(previous.at))
            })
            .map(|previous| {
                let index = (previous.slot % 2) as usize;
                self.minutes[index]
                    .take()
                    .filter(|receipt| receipt.slot == previous.slot)
                    .unwrap_or(Receipt {
                        slot: previous.slot,
                        ..Receipt::default()
                    })
            });
        (boundary, receipt, rewind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Verdict {
    Met,
    Missed,
    Unknown,
}

#[derive(Clone, Copy, Serialize)]
pub(super) struct Reading {
    through_unix_ms: u64,
    window_minutes: u64,
    covered_minutes: u64,
    eligible: u128,
    fast: u128,
    verdict: Verdict,
    storage_ready: bool,
}

impl Reading {
    fn new(end: u64, storage_ready: bool) -> Self {
        Self {
            through_unix_ms: end * MINUTE_MS,
            window_minutes: WINDOW,
            covered_minutes: 0,
            eligible: 0,
            fast: 0,
            verdict: Verdict::Unknown,
            storage_ready,
        }
    }

    fn add(&mut self, eligible: u64, fast: u64) {
        self.covered_minutes += 1;
        self.eligible += u128::from(eligible);
        self.fast += u128::from(fast);
    }

    fn finish(mut self) -> Self {
        if self.covered_minutes == WINDOW && self.eligible > 0 {
            self.verdict = if self.fast * 100 >= self.eligible * GOAL_PERCENT {
                Verdict::Met
            } else {
                Verdict::Missed
            };
        }
        self
    }
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

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct Completion;

impl Completion {
    fn reserve() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

static PENDING: LazyLock<Mutex<Pending>> = LazyLock::new(|| Mutex::new(Pending::default()));
static PUBLISHED: Mutex<Option<Reading>> = Mutex::new(None);

fn now_ms() -> u64 {
    super::client_now_ms() as u64
}

pub(super) fn record(method: &str, route: &str, outcome: RequestOutcome, elapsed: Duration) {
    if let Some(fast) = count(method, route, outcome, elapsed) {
        // Reserve before capturing completion UTC so the writer can detect an
        // earlier minute's request still waiting for the counter lock.
        let reservation = Completion::reserve();
        let completed = now_ms() / MINUTE_MS;
        let mut pending = PENDING.lock();
        pending.record(completed, now_ms() / MINUTE_MS, fast);
        drop(reservation);
    }
}

pub(super) fn read() -> Reading {
    let pending = PENDING.lock();
    let end = now_ms() / MINUTE_MS;
    let published = *PUBLISHED.lock();
    published
        .filter(|reading| {
            pending.serving
                && pending.rewind_from.is_none()
                && reading.through_unix_ms == end * MINUTE_MS
        })
        .unwrap_or_else(|| Reading::new(end, published.is_some()))
}

/// Owns agent-list measurement for the native listeners' serving lifetime.
#[must_use = "dropping the guard stops measuring listener coverage"]
pub struct AgentsRequestSloServing(tokio::task::JoinHandle<()>);

impl Drop for AgentsRequestSloServing {
    fn drop(&mut self) {
        PENDING.lock().stop_serving();
        self.0.abort();
    }
}

/// Start once, when the native listeners enter service.
pub fn start_agents_request_slo(state_dir: PathBuf, node: String) -> AgentsRequestSloServing {
    PENDING.lock().start_serving();
    AgentsRequestSloServing(tokio::spawn(smallclaims::task_cpu::account(
        "slo.agents_request_minutes",
        run(state_dir, node),
    )))
}

// Awaiting each blocking update bounds the writer to one receipt in flight.
async fn run(state_dir: PathBuf, node: String) {
    let path = std::sync::Arc::new(state_dir.join(LEDGER_NAME));
    let node = std::sync::Arc::new(node);
    let mut previous = None;
    let mut invalidate_from = None;
    let mut ledger = None;
    loop {
        let (boundary, receipt) = {
            let mut pending = PENDING.lock();
            if !pending.serving {
                return;
            }
            let (boundary, receipt, rewind) = pending.boundary(
                previous,
                now_ms(),
                Instant::now(),
                IN_FLIGHT.load(Ordering::SeqCst) != 0,
            );
            if let Some(slot) = rewind.or_else(|| previous.is_none().then_some(boundary.slot)) {
                invalidate_from = Some(invalidate_from.map_or(slot, |prior: u64| prior.min(slot)));
                *PUBLISHED.lock() = None;
            }
            (boundary, receipt)
        };
        previous = Some(boundary);
        let db_path = path.clone();
        let db_node = node.clone();
        let result = tokio::task::spawn_blocking(move || {
            smallclaims::task_cpu::blocking("slo.agents_request_minutes", || {
                let mut ledger = match ledger {
                    Some(ledger) => ledger,
                    None => Ledger::open(&db_path, &db_node)?,
                };
                let reading = ledger.update(boundary.slot, receipt, invalidate_from)?;
                Ok::<_, anyhow::Error>((ledger, reading))
            })
        })
        .await
        .context("join agents request SLO writer")
        .and_then(|result| result);
        match result {
            Ok((stored, reading)) => {
                ledger = Some(stored);
                invalidate_from = None;
                *PUBLISHED.lock() = Some(reading);
            }
            Err(error) => {
                ledger = None;
                *PUBLISHED.lock() = None;
                tracing::warn!(%error, "agents request SLO ledger unavailable");
            }
        }
        let now = now_ms();
        tokio::time::sleep(Duration::from_millis(MINUTE_MS - now % MINUTE_MS + 5)).await;
    }
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
        for (method, route) in [
            ("POST", "/v1/client/agents"),
            ("GET", "/v1/client/agents/{id}"),
        ] {
            assert_eq!(
                count(method, route, RequestOutcome::Ok, Duration::ZERO),
                None
            );
        }
    }

    #[test]
    fn only_complete_minutes_emit_receipts_and_covered_idle_is_zero() {
        let mut pending = Pending::default();
        pending.start_serving();
        let start = Instant::now();
        let mut previous = None;
        let mut tick = |pending: &mut Pending, millis| {
            let (boundary, receipt, rewind) = pending.boundary(
                previous,
                millis,
                start + Duration::from_millis(millis),
                false,
            );
            previous = Some(boundary);
            (receipt, rewind)
        };

        assert_eq!(tick(&mut pending, 20_000), (None, None));
        pending.record(0, 0, false);
        assert_eq!(tick(&mut pending, 60_005), (None, None));
        pending.record(1, 1, true);
        pending.record(2, 2, false);
        assert_eq!(
            tick(&mut pending, 120_005),
            (
                Some(Receipt {
                    slot: 1,
                    eligible: 1,
                    fast: 1
                }),
                None
            )
        );
        assert_eq!(tick(&mut pending, 240_005), (None, None));
        assert_eq!(
            tick(&mut pending, 300_005),
            (
                Some(Receipt {
                    slot: 4,
                    eligible: 0,
                    fast: 0
                }),
                None
            )
        );
        pending.record(2, 2, false);
        assert_eq!(tick(&mut pending, 360_005), (None, Some(2)));
    }

    #[test]
    fn crossing_the_lock_minute_preserves_the_completion_slot_or_leaves_a_gap() {
        let start = Instant::now();
        let run = |in_flight_at_cut: bool| {
            let mut pending = Pending::default();
            pending.start_serving();
            let (startup, _, _) = pending.boundary(None, 5, start, false);
            let (previous, _, _) = pending.boundary(
                Some(startup),
                60_005,
                start + Duration::from_secs(60),
                false,
            );
            // Completion stamped just before 120_000, then obtained the lock after it.
            if !in_flight_at_cut {
                pending.record(1, 2, true);
            }
            let (current, closed, _) = pending.boundary(
                Some(previous),
                120_005,
                start + Duration::from_secs(120),
                in_flight_at_cut,
            );
            if in_flight_at_cut {
                pending.record(1, 2, true);
            }
            let (_, next, _) = pending.boundary(
                Some(current),
                180_005,
                start + Duration::from_secs(180),
                false,
            );
            assert_eq!(
                next,
                Some(Receipt {
                    slot: 2,
                    eligible: 0,
                    fast: 0
                })
            );
            closed
        };
        assert_eq!(
            run(false),
            Some(Receipt {
                slot: 1,
                eligible: 1,
                fast: 1
            })
        );
        assert_eq!(run(true), None);
    }

    #[test]
    fn startup_and_shutdown_drain_do_not_emit_idle_receipts() {
        let mut pending = Pending::default();
        let start = Instant::now();
        let mut previous = None;
        let tick = |pending: &mut Pending, previous: &mut Option<Boundary>, minute| {
            let (boundary, receipt, _) = pending.boundary(
                *previous,
                minute * MINUTE_MS + 5,
                start + Duration::from_secs(minute * 60),
                false,
            );
            *previous = Some(boundary);
            receipt
        };
        pending.record(0, 0, false);
        assert_eq!(tick(&mut pending, &mut previous, 0), None);
        assert_eq!(tick(&mut pending, &mut previous, 1), None);
        assert_eq!(tick(&mut pending, &mut previous, 2), None);

        pending.start_serving();
        previous = None;
        assert_eq!(tick(&mut pending, &mut previous, 3), None);
        assert_eq!(tick(&mut pending, &mut previous, 4), None);
        assert_eq!(
            tick(&mut pending, &mut previous, 5),
            Some(Receipt {
                slot: 4,
                eligible: 0,
                fast: 0
            })
        );

        pending.stop_serving();
        pending.record(5, 5, false);
        assert_eq!(tick(&mut pending, &mut previous, 6), None);
        assert_eq!(tick(&mut pending, &mut previous, 7), None);
        assert_eq!(pending.minutes, [None; 2]);
    }

    #[test]
    fn requires_full_coverage_and_eligible_traffic() {
        let reading = |covered, eligible, fast| {
            Reading {
                covered_minutes: covered,
                eligible,
                fast,
                ..Reading::new(WINDOW, true)
            }
            .finish()
        };
        assert_eq!(reading(WINDOW - 1, 20, 19).verdict, Verdict::Unknown);
        assert_eq!(reading(WINDOW, 0, 0).verdict, Verdict::Unknown);
        assert_eq!(reading(WINDOW, 20, 19).verdict, Verdict::Met);
        assert_eq!(reading(WINDOW, 21, 19).verdict, Verdict::Missed);
    }
}
