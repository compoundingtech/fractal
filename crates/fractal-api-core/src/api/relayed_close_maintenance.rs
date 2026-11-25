//! Daemon-owned discovery of terminal remote closes, driven by changed message/ownership keys.
//! Reads never start or feed this worker. Remote marks have their own authenticated change feed.

use super::{AppState, message_reads};
use crate::store::{relayed_close_work, relayed_closes::{self, RelayedClose}};
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use smallclaims::read_budget::{self, ReadBudget};

struct TurnBudget(ReadBudget);
impl Drop for TurnBudget {
    fn drop(&mut self) { self.0.cancel(); }
}

const TURN_LIMIT: Duration = Duration::from_secs(15);
const PAGE_LIMIT: usize = relayed_closes::QUEUED;

fn now_ms() -> anyhow::Result<u64> {
    Ok(u64::try_from(super::client_now_ms())?)
}

#[derive(Default)]
struct Work {
    discovery_passes: AtomicU64,
    candidate_messages: AtomicU64,
    remote_event_pages: AtomicU64,
    remote_changed_messages: AtomicU64,
    legacy_peer_discovery_passes: AtomicU64,
}

pub fn work_counts(store: &crate::store::Store) -> serde_json::Value {
    let work = store.extension::<Work>();
    serde_json::json!({
        "discovery_passes": work.discovery_passes.load(Ordering::Relaxed),
        "candidate_messages": work.candidate_messages.load(Ordering::Relaxed),
        "remote_event_pages": work.remote_event_pages.load(Ordering::Relaxed),
        "remote_changed_messages": work.remote_changed_messages.load(Ordering::Relaxed),
        "legacy_peer_discovery_passes": work.legacy_peer_discovery_passes.load(Ordering::Relaxed),
    })
}

/// Consume one bounded page of pending keys. An unchanged input does no discovery work.
pub async fn turn(state: &AppState) -> anyhow::Result<()> {
    let budget = TurnBudget(ReadBudget::new_quiet("maintenance.relayed_closes", TURN_LIMIT));
    discover(state, &budget.0).await
}

async fn discover(state: &AppState, budget: &ReadBudget) -> anyhow::Result<()> {
    // Durable terminal evidence must settle (or surface its local error) before another
    // bounded page is admitted. Failed reads remain durable until their network deadline.
    relayed_closes::wait_idle(&state.store).await?;
    let store = state.store.clone();
    let selecting = budget.clone();
    let (reads, legacy) = tokio::task::spawn_blocking(move || read_budget::with(Some(selecting), || -> anyhow::Result<_> {
        read_budget::check()?;
        relayed_close_work::discovery_batch(&store, PAGE_LIMIT)?;
        let reads = relayed_close_work::read_batch(&store, PAGE_LIMIT, now_ms()?)?;
        if !reads.is_empty() { return Ok((reads, None)); }
        let legacy = relayed_close_work::legacy_read_batch(&store, PAGE_LIMIT, now_ms()?)?;
        if let Some(page) = &legacy
            && message_reads::advertises(&store, &page.host, crate::peer::MESSAGE_MARK_EVENTS_FEATURE, 1)?
        {
            relayed_close_work::cancel_legacy_host(&store, &page.host)?;
            return Ok((Vec::new(), None));
        }
        let reads = legacy.as_ref().map(|page| page.messages.iter()
            .map(|message| (message.clone(), page.host.clone())).collect()).unwrap_or_default();
        Ok((reads, legacy))
    })).await??;
    let compatibility = legacy.is_some();
    if !reads.is_empty() {
        let work = state.store.extension::<Work>();
        work.discovery_passes.fetch_add(1, Ordering::Relaxed);
        work.candidate_messages.fetch_add(reads.len() as u64, Ordering::Relaxed);
    }
    let mut asks = BTreeMap::<String, Vec<String>>::new();
    for (subject, host) in reads { asks.entry(host).or_default().push(subject); }
    let mut asks = asks.into_iter();
    loop {
        let mut page = BTreeMap::new();
        let mut unsupported = Vec::new();
        for (host, subjects) in asks.by_ref().take(message_reads::MAX_HOSTS) {
            if message_reads::advertises(&state.store, &host, message_reads::MESSAGE_OFFSETS_FEATURE, 1)?
                && message_reads::advertises(&state.store, &host, crate::peer::SEAT_FRESHNESS_READS_FEATURE, 1)?
            {
                page.insert(host, subjects);
            } else {
                unsupported.extend(subjects);
            }
        }
        if page.is_empty() && unsupported.is_empty() { break; }
        let requested = page.values().flatten().cloned().collect::<Vec<_>>();
        let answers = message_reads::read_uncached(state, page, message_reads::CHECK_DEADLINE).await;
        let closes = pending_closes(&answers);
        let mut succeeded = unsupported;
        let mut failed = Vec::new();
        for subject in requested {
            if matches!(answers.get(&subject), Some(message_reads::Relayed::Answered { .. })) {
                succeeded.push(subject);
            } else { failed.push(subject); }
        }
        let store = state.store.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            // A successful marks answer is acknowledged only after terminal evidence has
            // committed to the durable close queue. Cancellation cannot lose either job.
            relayed_closes::queue(&store, closes)?;
            if !compatibility { relayed_close_work::read_succeeded(&store, &succeeded)?; }
            relayed_close_work::read_failed(&store, &failed, now_ms()?)
        }).await??;
        relayed_closes::wait_idle(&state.store).await?;
    }
    if let Some(page) = legacy {
        let store = state.store.clone();
        tokio::task::spawn_blocking(move || relayed_close_work::acknowledge_legacy_page(&store, &page)).await??;
    }
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

struct RemoteChange {
    host: String,
    reset: bool,
    messages: Vec<String>,
    consumers: Vec<String>,
    admitted: tokio::sync::oneshot::Sender<()>,
}

async fn follow_host(state: AppState, host: String, changes: mpsc::Sender<RemoteChange>) {
    let Some(relay) = state.client_relay.clone() else { return; };
    let mut cursor = None::<String>;
    let mut retry = Duration::from_secs(1);
    loop {
        match relay.read_message_mark_events(&host, cursor.as_deref(), PAGE_LIMIT, Duration::from_secs(70)).await {
            Ok(Some(page)) => {
                retry = Duration::from_secs(1);
                if page.reset || !page.messages.is_empty() || !page.consumers.is_empty() {
                    let (admitted, acknowledged) = tokio::sync::oneshot::channel();
                    if changes.send(RemoteChange {
                        host: host.clone(), reset: page.reset, messages: page.messages, consumers: page.consumers, admitted,
                    }).await.is_err() || acknowledged.await.is_err() { return; }
                }
                // Do not advance remote evidence until its local changed-key admission commits.
                cursor = Some(page.cursor);
            }
            result => {
                tracing::warn!(host, error = ?result, "remote message event subscription disconnected");
                // A network reconnect is not a discovery timer. Resetting the cursor causes
                // one host-scoped reconciliation when connectivity is actually restored.
                cursor = None;
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_secs(60));
            }
        }
    }
}

fn sync_followers(
    state: &AppState,
    followers: &mut BTreeMap<String, tokio::task::JoinHandle<()>>,
    changes: &mpsc::Sender<RemoteChange>,
) -> anyhow::Result<Vec<String>> {
    let hosts = relayed_close_work::active_hosts(&state.store)?.into_iter()
        .map(|host| {
            message_reads::advertises(&state.store, &host, crate::peer::MESSAGE_MARK_EVENTS_FEATURE, 1)
                .map(|supported| (host, supported))
        }).collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    followers.retain(|host, task| {
        let keep = hosts.get(host) == Some(&true);
        if !keep { task.abort(); }
        keep
    });
    let mut legacy = Vec::new();
    if state.client_relay.is_some() {
        for (host, supported) in hosts {
            if supported {
                if !followers.contains_key(&host) {
                    followers.insert(host.clone(), tokio::spawn(follow_host(state.clone(), host, changes.clone())));
                }
            } else if message_reads::advertises(&state.store, &host, message_reads::MESSAGE_OFFSETS_FEATURE, 1)?
                && message_reads::advertises(&state.store, &host, crate::peer::SEAT_FRESHNESS_READS_FEATURE, 1)?
            {
                legacy.push(host);
            }
        }
    }
    Ok(legacy)
}

async fn wait_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

fn read_deadline(store: &crate::store::Store) -> anyhow::Result<Option<tokio::time::Instant>> {
    let now = now_ms()?;
    Ok(relayed_close_work::next_read_deadline(store)?.map(|at| {
        tokio::time::Instant::now() + Duration::from_millis(at.saturating_sub(now))
    }))
}

/// Modern peers are event-driven. Only explicitly capability-limited legacy peers retain
/// a compatibility schedule, scoped to their already known pending message candidates.
pub async fn run(state: AppState, mut stopped: watch::Receiver<bool>) {
    let wake = Arc::new(tokio::sync::Notify::new());
    let changed = wake.clone();
    let _observer = state.store.observe_tables(&[
        relayed_close_work::WORK_TABLE, relayed_close_work::READ_TABLE,
    ], move |_| changed.notify_one());
    let (changes, mut remote) = mpsc::channel::<RemoteChange>(64);
    let mut pending_remote = None;
    let mut followers = BTreeMap::new();
    let mut initial = true;
    let mut legacy_hosts: Vec<String> = Vec::new();
    let mut legacy_deadline = None;
    let mut recovery = None;
    wake.notify_one(); // Register before bootstrap/restart recovery; no lost initial changes.
    loop {
        if *stopped.borrow() { break; }
        let retry = if recovery.is_some() { None } else {
            match read_deadline(&state.store) {
                Ok(deadline) => deadline,
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "relayed close retry deadline read failed");
                    None
                }
            }
        };
        tokio::select! {
            biased;
            _ = stopped.changed() => break,
            Some(change) = remote.recv(), if pending_remote.is_none() => pending_remote = Some(change),
            _ = wait_deadline(legacy_deadline), if recovery.is_none() => {
                for host in &legacy_hosts {
                    match relayed_close_work::enqueue_legacy_host(&state.store, host) {
                        Ok(()) => { state.store.extension::<Work>().legacy_peer_discovery_passes.fetch_add(1, Ordering::Relaxed); }
                        Err(error) => tracing::warn!(error = %format!("{error:#}"), "legacy peer close admission failed"),
                    }
                }
                legacy_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(30));
            }
            _ = wait_deadline(retry) => {}
            _ = wake.notified() => {}
        }
        let mut changed = std::mem::take(&mut initial);
        let mut force_turn = changed || recovery.is_some();
        loop {
            let mut admission_error = None;
            if let Some(change) = pending_remote.take() {
                if followers.contains_key(&change.host) {
                    let store = state.store.clone();
                    let admitted = tokio::task::spawn_blocking(move || {
                        let result = if change.reset {
                            relayed_close_work::enqueue_host(&store, &change.host)
                        } else {
                            relayed_close_work::enqueue_messages(&store, &change.messages)
                                .and_then(|()| change.consumers.iter().try_for_each(|recipient| {
                                    relayed_close_work::enqueue_recipient(&store, recipient)
                                }))
                        };
                        (result, change)
                    }).await;
                    match admitted {
                        Ok((Ok(()), change)) => {
                            let work = state.store.extension::<Work>();
                            work.remote_event_pages.fetch_add(1, Ordering::Relaxed);
                            work.remote_changed_messages.fetch_add(change.messages.len() as u64, Ordering::Relaxed);
                            let _ = change.admitted.send(());
                        }
                        Ok((Err(error), change)) => {
                            pending_remote = Some(change);
                            admission_error = Some(error);
                        }
                        Err(error) => admission_error = Some(error.into()),
                    }
                } else {
                    let _ = change.admitted.send(()); // No longer owned/relevant here.
                }
            }
            let pending = relayed_close_work::has_pending(&state.store).and_then(|pending| {
                let now = now_ms()?;
                Ok(pending || relayed_close_work::next_read_deadline(&state.store)?
                    .is_some_and(|at| at <= now))
            });
            if admission_error.is_none() && matches!(pending, Ok(false)) && !force_turn { break; }
            let result = match admission_error {
                Some(error) => Err(error),
                None => match pending {
                    Err(error) => Err(error),
                    Ok(_) => {
                        changed = true;
                        tokio::select! {
                            biased;
                            _ = stopped.changed() => break,
                            result = turn(&state) => result,
                        }
                    }
                },
            };
            match result {
                Ok(()) => {
                    force_turn = false;
                    recovery = None;
                }
                Err(error) => {
                    tracing::error!(error = %format!("{error:#}"), "relayed close maintenance blocked on local recovery");
                    if recovery.is_none() {
                        let recovered = wake.clone();
                        recovery = Some(state.store.observe_commits(move |_| recovered.notify_one()));
                        // Register then recheck once: a repair commit before registration
                        // cannot strand durable work. Later attempts require actual writer
                        // activity; there is no local-failure retry timer.
                        force_turn = true;
                        continue;
                    }
                    break;
                }
            }
            if *stopped.borrow() { break; }
        }
        if *stopped.borrow() { break; }
        if changed && recovery.is_none() {
            match sync_followers(&state, &mut followers, &changes) {
                Ok(hosts) => {
                    legacy_hosts = hosts;
                    if legacy_hosts.is_empty() { legacy_deadline = None; }
                    else if legacy_deadline.is_none() {
                        legacy_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(30));
                    }
                }
                Err(error) => tracing::warn!(error = %format!("{error:#}"), "relayed close subscription update failed"),
            }
        }
    }
    for (_, task) in followers { task.abort(); }
    if let Err(error) = relayed_closes::wait_idle(&state.store).await {
        tracing::error!(error = %format!("{error:#}"), "relayed close writes remain durable for restart recovery");
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
