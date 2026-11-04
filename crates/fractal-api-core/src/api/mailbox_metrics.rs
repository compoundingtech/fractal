//! Mailbox gauges fold off the scrape path; callbacks only read the last complete summary.

use super::{Store, client_now_ms};
use crate::CoordinationStore;
use opentelemetry::{KeyValue, metrics::Meter};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const REFRESH_PAUSE: Duration = Duration::from_secs(30);
const STATES: [&str; 3] = ["pending", "delivered", "held"];
const AGE_BANDS: [&str; 2] = ["recent", "stale"];

/// Declared agents whose member is owned by this daemon's origin. Keep this selection shared
/// with sibling per-agent gauges, so people, retired seats and remote members never leak in.
pub fn managed_agents_owned_by_local_host(store: &Store) -> anyhow::Result<Vec<String>> {
    Ok(store.desired_subjects()?.into_iter()
        .filter(|subject| subject.kind == "agent"
            && subject.member.as_ref().is_some_and(|member| member.host == store.origin()))
        .map(|subject| subject.subject)
        .collect())
}

#[derive(Default)]
pub struct Summary {
    series: Vec<Series>,
}

struct Series {
    attributes: [KeyValue; 3],
    count: u64,
    oldest_sent_at_unix_ms: Option<u128>,
}

impl Series {
    fn age_seconds(&self, now: u128) -> f64 {
        self.oldest_sent_at_unix_ms.map_or(0., |sent| {
            Duration::from_millis(u64::try_from(now.saturating_sub(sent)).unwrap_or(u64::MAX))
                .as_secs_f64()
        })
    }
}
fn append_agent_series(
    series: &mut Vec<Series>,
    agent: &str,
    rows: &[(String, &str, u64, u128)],
    include_held: bool,
) {
    for state in STATES.into_iter().filter(|state| include_held || *state != "held") {
        for band in AGE_BANDS {
            let row = rows.iter().find(|(name, age_band, _, _)| name == state && *age_band == band);
            series.push(Series {
                attributes: [KeyValue::new("st.agent", agent.to_owned()),
                    KeyValue::new("st.mailbox.state", state),
                    KeyValue::new("st.mailbox.age_band", band)],
                count: row.map_or(0, |(_, _, count, _)| *count),
                oldest_sent_at_unix_ms: row.map(|(_, _, _, sent)| *sent),
            });
        }
    }
}

/// Read one consistent cut, using the recipient index rather than scanning fleet messages.
/// Each agent contributes all six state/band pairs, except both held bands during bootstrap.
pub fn summarize(store: &Store) -> anyhow::Result<Summary> {
    let now = client_now_ms();
    store.read_snapshot(|_| {
        let include_held = store.coordination_backfill_status()?.2;
        let agents = managed_agents_owned_by_local_host(store)?;
        let mut series = Vec::with_capacity(agents.len().saturating_mul(STATES.len() * AGE_BANDS.len()));
        for agent in agents {
            let rows = store.mailbox_unread_summary(&agent, include_held, now)?;
            append_agent_series(&mut series, &agent, &rows, include_held);
        }
        Ok(Summary { series })
    })
}

/// Register against a supplied meter, permitting isolated exporter proofs without global state.
/// Host identity is inherited from the daemon's `host.name` resource, not point attributes.
pub fn register_metrics(meter: &Meter, summary: Arc<RwLock<Summary>>) {
    let counts = Arc::clone(&summary);
    meter.u64_observable_gauge("st.mailbox.unread")
        .with_unit("{message}")
        .with_description("Unread messages by locally owned declared agent, mailbox state and age band")
        .with_callback(move |observer| {
            let summary = counts.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for series in &summary.series {
                observer.observe(series.count, &series.attributes);
            }
        }).build();
    meter.f64_observable_gauge("st.mailbox.oldest_unread_age")
        .with_unit("s")
        .with_description("Oldest unread accepted message.sent age by agent, mailbox state and age band")
        .with_callback(move |observer| {
            let now = client_now_ms();
            let summary = summary.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for series in &summary.series {
                observer.observe(series.age_seconds(now), &series.attributes);
            }
        }).build();
}

/// One refresh at a time, followed by at least 30 seconds of rest (or its elapsed cost if
/// longer). Failures keep the last complete summary, never a partial count or false zero.
pub fn start(store: &Arc<Store>) {
    if !crate::otel::metrics_enabled() {
        return;
    }
    let store = Arc::downgrade(store);
    tokio::spawn(async move {
        let summary = Arc::new(RwLock::new(Summary::default()));
        let mut registered = false;
        loop {
            let Some(store) = store.upgrade() else { return; };
            let started = Instant::now();
            let result = tokio::task::spawn_blocking(move || {
                crate::task_cpu::profiled_blocking("mailbox_metrics/refresh", || summarize(&store))
            }).await;
            match result {
                Ok(Ok(next)) => {
                    *summary.write().unwrap_or_else(std::sync::PoisonError::into_inner) = next;
                    if !registered {
                        register_metrics(&opentelemetry::global::meter("st3"), Arc::clone(&summary));
                        registered = true;
                    }
                }
                Ok(Err(error)) => eprintln!("st3: WARN mailbox metrics refresh failed: {error:#}"),
                Err(error) => eprintln!("st3: WARN mailbox metrics refresh task failed: {error}"),
            }
            tokio::time::sleep(REFRESH_PAUSE.max(started.elapsed())).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_metrics_age_is_zero_for_empty_and_future_messages() {
        let mut series = Series {
            attributes: [KeyValue::new("st.agent", "agent/example.worker"),
                KeyValue::new("st.mailbox.state", "pending"),
                KeyValue::new("st.mailbox.age_band", "recent")],
            count: 0,
            oldest_sent_at_unix_ms: None,
        };
        assert_eq!(series.age_seconds(10_000), 0.);
        series.count = 1;
        series.oldest_sent_at_unix_ms = Some(11_000);
        assert_eq!(series.age_seconds(10_000), 0.);
        assert_eq!(series.age_seconds(12_250), 1.25);
    }

    #[test]
    fn mailbox_metrics_recent_age_is_not_masked_by_stale_backlog() {
        let now = crate::store::delivery::MAILBOX_STALE_AFTER_MS + 100;
        let mut series = Vec::new();
        append_agent_series(&mut series, "agent/example.worker", &[
            ("pending".to_owned(), "recent", 1, now - 1_800_000),
            ("pending".to_owned(), "stale", 1, 100),
        ], true);
        assert_eq!(series[0].age_seconds(now), 1_800.);
        assert_eq!(series[1].age_seconds(now), 86_400.);
        // Scraping advances ages without reclassifying the cached bands.
        assert_eq!(series[0].age_seconds(now + 1_000), 1_801.);
    }

    #[test]
    fn mailbox_metrics_summary_keeps_zeros_and_omits_only_bootstrapping_held() {
        for include_held in [false, true] {
            let mut series = Vec::new();
            append_agent_series(&mut series, "agent/example.worker",
                &[("delivered".to_owned(), "recent", 2, 1_000)], include_held);
            assert_eq!(series.len(), if include_held { 6 } else { 4 });
            for (index, series) in series.iter().enumerate() {
                assert_eq!(series.count, if index == 2 { 2 } else { 0 });
                assert_eq!(series.age_seconds(2_500), if index == 2 { 1.5 } else { 0. });
                assert_eq!(series.attributes[2],
                    KeyValue::new("st.mailbox.age_band", AGE_BANDS[index % 2]));
            }
            assert!(series.iter().all(|series| series.attributes[0]
                == KeyValue::new("st.agent", "agent/example.worker")));
        }
    }
}
