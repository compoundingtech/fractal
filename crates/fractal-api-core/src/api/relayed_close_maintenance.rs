//! Daemon-owned discovery of terminal remote closes. Reads never start or feed this worker.

use super::{AppState, message_reads};
use crate::store::relayed_closes::{self, RelayedClose};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::watch;
use smallclaims::read_budget::{self, ReadBudget};

struct TurnBudget(ReadBudget);
impl Drop for TurnBudget {
    fn drop(&mut self) { self.0.cancel(); }
}

const INTERVAL: Duration = Duration::from_secs(30);
const TURN_LIMIT: Duration = Duration::from_secs(15);
const PAGE_LIMIT: usize = relayed_closes::QUEUED;

/// A retained-header cursor, independent of which messages somebody happened to read.
#[derive(Default)]
pub struct Cursor {
    after: String,
}

/// Discover at most one page, within one time budget. A timed-out read cannot later queue work.
pub async fn turn(state: &AppState, cursor: &mut Cursor) -> anyhow::Result<()> {
    let budget = TurnBudget(ReadBudget::new_quiet("maintenance.relayed_closes", TURN_LIMIT));
    tokio::time::timeout(TURN_LIMIT, discover(state, cursor, &budget.0)).await??;
    Ok(())
}

async fn discover(state: &AppState, cursor: &mut Cursor, budget: &ReadBudget) -> anyhow::Result<()> {
    let store = state.store.clone();
    let after = cursor.after.clone();
    let selecting = budget.clone();
    let candidates = tokio::task::spawn_blocking(move || read_budget::with(Some(selecting), || -> anyhow::Result<_> {
        read_budget::check()?;
        let hosts = message_reads::relay_hosts(&store)?;
        let host_of = hosts.into_iter().flat_map(|(host, seats)| {
            seats.into_iter().map(move |seat| (seat, host.clone()))
        }).collect::<BTreeMap<_, _>>();
        if host_of.is_empty() {
            return Ok(Vec::new());
        }
        let recipients = serde_json::to_string(&host_of.keys().collect::<Vec<_>>())?;
        let connection = store.readers.get();
        // Include superseded reminder members, not just the currently selected winner.
        let mut select = connection.prepare_cached(
            "SELECT subject,recipient FROM local_client_message_selectors_v1
             WHERE subject>?1 AND born_index>=0 AND retired_index IS NULL AND closed=0
               AND recipient IN (SELECT value FROM json_each(?2))
               AND NOT EXISTS(SELECT 1 FROM local_relayed_message_closes WHERE message=subject)
             ORDER BY subject LIMIT ?3",
        )?;
        let rows = select.query_map(rusqlite::params![after, recipients, PAGE_LIMIT as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|(subject, recipient)| {
            host_of.get(&recipient).map(|host| (subject, host.clone()))
        }).collect::<Vec<_>>())
    })).await??;
    if candidates.is_empty() {
        cursor.after.clear();
        return Ok(());
    }
    let mut asks = BTreeMap::<String, Vec<String>>::new();
    for (subject, host) in candidates {
        // Leave this row for the next turn rather than starving hosts past the relay bound.
        if !asks.contains_key(&host) && asks.len() == message_reads::MAX_HOSTS {
            break;
        }
        cursor.after.clone_from(&subject);
        asks.entry(host).or_default().push(subject);
    }
    let answers = message_reads::read_uncached(state, asks, message_reads::CHECK_DEADLINE).await;
    budget.check()?;
    read_budget::with(Some(budget.clone()), || {
        relayed_closes::queue(&state.store, pending_closes(&answers))
    })?;
    Ok(())
}

fn pending_closes(answers: &BTreeMap<String, message_reads::Relayed>) -> Vec<RelayedClose> {
    answers.iter().filter_map(|(subject, answer)| match answer {
        message_reads::Relayed::Answered {
            host, held: true, status: Some("closed"), latest: Some((receipt, at)), ..
        } => Some(RelayedClose {
            message: subject.clone(), host: host.clone(), receipt: receipt.clone(),
            closed_at_unix_ms: *at,
        }),
        _ => None,
    }).collect()
}

/// Run only from the daemon's serving lifetime. Cancellation drops an in-flight discovery;
/// already admitted close writes finish before this worker acknowledges shutdown.
pub async fn run(state: AppState, mut stopped: watch::Receiver<bool>) {
    let mut cursor = Cursor::default();
    loop {
        if *stopped.borrow() { break; }
        tokio::select! {
            biased;
            _ = stopped.changed() => break,
            result = turn(&state, &mut cursor) => {
                if let Err(error) = result {
                    tracing::warn!(error = %format!("{error:#}"), "relayed close maintenance turn failed");
                }
            }
        }
        tokio::select! {
            biased;
            _ = stopped.changed() => break,
            _ = tokio::time::sleep(INTERVAL) => {}
        }
    }
    while !relayed_closes::idle(&state.store) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_helpers_have_no_close_writer_capability() {
        let reads = include_str!("message_reads.rs");
        assert!(!reads.contains("relayed_closes"));
    }

    #[test]
    fn only_held_closed_answers_with_receipts_are_pending() {
        use super::*;
        use message_reads::Relayed;
        let closed = Relayed::Answered {
            host: "remote".into(), held: true, status: Some("closed"), consumed: None,
            latest: Some(("offset-receipt/closed".into(), 7)),
        };
        let mut not_held = closed.clone();
        if let Relayed::Answered { held, .. } = &mut not_held { *held = false; }
        let mut no_receipt = closed.clone();
        if let Relayed::Answered { latest, .. } = &mut no_receipt { *latest = None; }
        let mut open = closed.clone();
        if let Relayed::Answered { status, .. } = &mut open { *status = Some("read"); }
        let answers = BTreeMap::from([
            ("closed".into(), closed), ("not-held".into(), not_held),
            ("no-receipt".into(), no_receipt), ("open".into(), open),
            ("unknown".into(), Relayed::Unknown { host: "remote".into(), reason: "silent".into() }),
        ]);
        assert_eq!(pending_closes(&answers), vec![RelayedClose {
            message: "closed".into(), host: "remote".into(),
            receipt: "offset-receipt/closed".into(), closed_at_unix_ms: 7,
        }]);
    }

}
