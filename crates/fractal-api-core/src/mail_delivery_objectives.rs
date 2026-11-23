//! Owner-host mail latency from transaction-fed, retained source facts. Reads never sample mail.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rusqlite::OptionalExtension as _;
use serde_json::{Value, json};

use crate::lifecycle_objectives::{LifecycleSample, LifecycleTerminal, ratio_row};
use crate::store::Store;

const DEADLINE: u64 = 15 * 60_000;
const FRESHNESS: u64 = 120_000;
const HORIZON: u64 = 30 * 86_400_000;

#[derive(Default)]
struct Publication {
    started: AtomicBool,
    snapshot: Mutex<Option<Snapshot>>,
    changed: Mutex<Option<tokio::sync::watch::Receiver<u64>>>,
    incarnation: Mutex<Option<String>>,
    work: Work,
    #[cfg(test)]
    captures: std::sync::atomic::AtomicUsize,

}
#[derive(Debug)]
struct Snapshot {
    observed_ms: u64,
    generation: u64,
    covered_from_ms: u64,
    source_through_ms: u64,
    recapture_at_ms: Option<u64>,
    samples: Vec<LifecycleSample>,
    held: usize,
    unknown: usize,
    recipients: BTreeSet<String>,
    source_cut: u64,
    error: Option<String>,
    #[cfg(test)]
    source_work: [usize; 3],
}

#[derive(Default)]
struct Work {
    full_rebuilds: AtomicU64,
    messages_evaluated: AtomicU64,
    source_rows: AtomicU64,
    acceptance_rows: AtomicU64,
    lifecycle_rows: AtomicU64,
    owner_lookups: AtomicU64,
}

/// Cumulative source work, independent of request/render frequency.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct WorkCounts {
    pub full_rebuilds: u64,
    pub messages_evaluated: u64,
    pub source_rows: u64,
    pub acceptance_rows: u64,
    pub lifecycle_rows: u64,
    pub owner_lookups: u64,
}

pub fn work_counts(store: &Store) -> WorkCounts {
    let publication = store.extension::<Publication>();
    let work = &publication.work;
    WorkCounts {
        full_rebuilds: work.full_rebuilds.load(Ordering::Relaxed),
        messages_evaluated: work.messages_evaluated.load(Ordering::Relaxed),
        source_rows: work.source_rows.load(Ordering::Relaxed),
        acceptance_rows: work.acceptance_rows.load(Ordering::Relaxed),
        lifecycle_rows: work.lifecycle_rows.load(Ordering::Relaxed),
        owner_lookups: work.owner_lookups.load(Ordering::Relaxed),
    }
}

/// Retained per-message reductions and their recipient dependencies. Source commits
/// replace only the affected reductions; timer expiry uses the same changed-key path.
#[derive(Default)]
struct Incremental {
    generation: Option<u64>,
    messages: BTreeMap<String, Snapshot>,
    recipients: BTreeMap<String, BTreeSet<String>>,
    facts: BTreeMap<String, Facts>,
}

impl Incremental {
    fn capture(&mut self, store: &Store, at: u64) -> anyhow::Result<Snapshot> {
        let receiver = store.subscribe_mail_objective_inputs();
        let (generation, delta) = store.mail_objective_changes_since(self.generation.unwrap_or(0));
        let work = store.extension::<Publication>();
        let mut changed = BTreeSet::new();
        if self.generation.is_none() || delta.is_none() {
            work.work.full_rebuilds.fetch_add(1, Ordering::Relaxed);
            self.messages.clear();
            self.facts.clear();
            self.recipients.clear();
            let raw = source_rows(store,
                at.saturating_sub(HORIZON.saturating_add(DEADLINE).saturating_add(FRESHNESS)), &mut || {})?;
            work.work.source_rows.fetch_add(raw.len() as u64, Ordering::Relaxed);
            for row in raw {
                changed.insert(row.0.clone());
                self.facts.entry(row.0.clone()).or_default().source.get_or_insert_with(Vec::new).push(row);
            }
        } else if let Some((messages, recipients)) = delta {
            for message in &messages { self.facts.remove(message); }
            changed.extend(messages);
            for recipient in recipients {
                if let Some(messages) = self.recipients.get(&recipient) {
                    changed.extend(messages.iter().cloned());
                }
            }
        }
        changed.extend(self.messages.iter().filter(|(_, snapshot)|
            snapshot.recapture_at_ms.is_some_and(|deadline| deadline <= at))
            .map(|(message, _)| message.clone()));
        let mut recipient_inputs = BTreeMap::new();
        for message in changed {
            work.work.messages_evaluated.fetch_add(1, Ordering::Relaxed);
            let snapshot = capture_message(store, at, Some(&message), Some(self.facts.entry(message.clone()).or_default()), &mut recipient_inputs, || {})?;
            if let Some(previous) = self.messages.remove(&message) {
                for recipient in previous.recipients {
                    if let Some(messages) = self.recipients.get_mut(&recipient) {
                        messages.remove(&message);
                        if messages.is_empty() { self.recipients.remove(&recipient); }
                    }
                }
            }
            if !snapshot.recipients.is_empty() {
                for recipient in &snapshot.recipients {
                    self.recipients.entry(recipient.clone()).or_default().insert(message.clone());
                }
                self.messages.insert(message, snapshot);
            } else {
                self.facts.remove(&message);
            }
        }
        self.generation = Some(generation);
        let mut snapshot = Snapshot {
            observed_ms: at, generation, covered_from_ms: at, source_through_ms: at,
            recapture_at_ms: None, samples: Vec::new(), held: 0, unknown: 0,
            recipients: BTreeSet::new(), source_cut: store.index()?, error: None,
            #[cfg(test)]
            source_work: [0; 3],
        };
        for part in self.messages.values() {
            snapshot.covered_from_ms = snapshot.covered_from_ms.min(part.covered_from_ms);
            snapshot.source_through_ms = snapshot.source_through_ms.min(part.source_through_ms);
            snapshot.recapture_at_ms = snapshot.recapture_at_ms.into_iter().chain(part.recapture_at_ms).min();
            snapshot.samples.extend_from_slice(&part.samples);
            snapshot.held += part.held;
            snapshot.unknown += part.unknown;
            #[cfg(test)]
            for (total, count) in snapshot.source_work.iter_mut().zip(part.source_work) { *total += count; }
        }
        // A recipient's newer frontier cannot extend another recipient's certified cut.
        snapshot.unknown += snapshot.samples.iter().filter(|sample|
            !matches!(sample.terminal, LifecycleTerminal::Excluded) && sample.onset_ms > snapshot.source_through_ms).count();
        let connection = store.readers.get();
        let shard = crate::store::messages_shard::mailbox_reads::position(store)?;
        let mirrored = crate::store::stream_mirror::mail_source_sent_through(&connection)?;
        let lifecycle: Option<String> = connection.query_row(
            "SELECT value FROM meta WHERE key='mail_delivery_source_v1_stream_through'", [], |row| row.get(0)).optional()?;
        if mirrored < shard.admitted || lifecycle.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0) < shard.lifecycle {
            snapshot.unknown += 1;
        }
        if let Some(shard) = crate::store::messages_shard::shard(store) { drop(shard.connection.write()); }
        drop(store.connection.write());
        if *receiver.borrow() != generation || store.mail_objective_input_coverage_failed() {
            snapshot.unknown += 1;
        }
        Ok(snapshot)
    }
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

/// One source-specific subscription owns capture. Changes coalesce while capture runs;
/// unrelated API wakes never recapture. Timers only renew the producer heartbeat or
/// evaluate actual lifecycle/budget boundaries, including retained-source expiry.
pub fn start(state: &crate::api::AppState) {
    let publication = state.store.extension::<Publication>();
    if publication.started.swap(true, Ordering::AcqRel) { return; }
    *publication.incarnation.lock().unwrap_or_else(PoisonError::into_inner) = Some(crate::model::new_request_id());
    let state = state.clone();
    let mut changed = state.store.subscribe_mail_objective_inputs();
    *publication.changed.lock().unwrap_or_else(PoisonError::into_inner) = Some(changed.clone());
    tokio::spawn(async move {
        let mut objective = Incremental::default();
        loop {
            let generation = *changed.borrow_and_update();
            #[cfg(test)]
            publication.captures.fetch_add(1, Ordering::AcqRel);
            let store = state.store.clone();
            let captured = tokio::task::spawn_blocking(move || {
                let result = objective.capture(&store, now());
                (objective, result)
            }).await;
            let mut snapshot = match captured {
                Ok((next, result)) => {
                    objective = next;
                    match result {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            objective = Incremental::default();
                            Snapshot { observed_ms: now(), generation, samples: Vec::new(), held: 0, unknown: 1,
                                covered_from_ms: now(), source_through_ms: 0, recapture_at_ms: None,
                                recipients: BTreeSet::new(),
                                #[cfg(test)]
                                source_work: [0; 3],
                                source_cut: 0, error: Some(format!("mail source capture unavailable: {error:?}")) }
                        }
                    }
                }
                Err(error) => {
                    objective = Incremental::default();
                    Snapshot { observed_ms: now(), generation, samples: Vec::new(), held: 0, unknown: 1,
                        covered_from_ms: now(), source_through_ms: 0, recapture_at_ms: None,
                        recipients: BTreeSet::new(),
                        #[cfg(test)]
                        source_work: [0; 3],
                        source_cut: 0, error: Some(format!("mail source capture unavailable: {error:?}")) }
                }
            };
            snapshot.generation = generation;
            *publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
            let store = state.store.clone();
            let at = now();
            let reported = tokio::task::spawn_blocking(move || {
                let native = rows(&store, at)?;
                crate::declared_objectives::check_native_and_report(&store, at, "agents/mail-delivery", &native)?;
                Ok::<_, anyhow::Error>(())
            }).await;
            match reported {
                Ok(Ok(())) => {}
                error => tracing::warn!(error = ?error, "mail objective owner episode evaluation failed"),
            }
            loop {
                let at = now();
                let boundary = {
                    let guard = publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
                    guard.as_ref().and_then(|snapshot| next_boundary(&snapshot.samples, at)
                        .into_iter().chain(snapshot.recapture_at_ms).min())
                };
                let delay = boundary.map_or(FRESHNESS / 2, |deadline| deadline.saturating_sub(at).min(FRESHNESS / 2));
                tokio::select! {
                    result = changed.changed() => {
                        if result.is_err() { return; }
                        break;
                    }
                    () = tokio::time::sleep(std::time::Duration::from_millis(delay.max(1))) => {
                        // An idle, subscribed producer can renew its own heartbeat without
                        // re-reading an unchanged source or fabricating an observation.
                        if let Some(snapshot) = publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                            snapshot.observed_ms = now();
                        }
                        if boundary.is_some_and(|deadline| deadline <= now()) {
                            // Retained-source expiry needs a new population; budget boundaries
                            // only rerender the same samples. Neither depends on broad API wakes.
                            let recapture = publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
                                .as_ref().and_then(|snapshot| snapshot.recapture_at_ms)
                                .is_some_and(|deadline| deadline <= now());
                            if recapture { break; }
                            crate::api::signal_local_change(&state);
                            let store = state.store.clone();
                            let reported = tokio::task::spawn_blocking(move || {
                                let at = now();
                                let native = rows(&store, at)?;
                                crate::declared_objectives::check_native_and_report(&store, at, "agents/mail-delivery", &native)
                            }).await;
                            if !matches!(reported, Ok(Ok(()))) {
                                tracing::warn!(error = ?reported, "mail objective boundary episode evaluation failed");
                            }
                        }
                    }
                }
            }
        }
    });
}

fn next_boundary(samples: &[LifecycleSample], now: u64) -> Option<u64> {
    samples.iter().filter_map(|sample| {
        let deadline = sample.onset_ms.saturating_add(DEADLINE);
        match sample.terminal {
            LifecycleTerminal::Accepted(at) | LifecycleTerminal::Failed(at) => Some(at.min(deadline)),
            LifecycleTerminal::Open => Some(deadline),
            LifecycleTerminal::Excluded => None,
        }
    }).flat_map(|at| [0, 5 * 60_000, 30 * 60_000, 3_600_000, 6 * 3_600_000, HORIZON]
        .into_iter().map(move |width| at.saturating_add(width).saturating_add(1)))
        .filter(|boundary| *boundary > now).min()
}

/// Pure rendering of a producer publication, including an honest unknown startup row.
pub fn rows(store: &Store, now: u64) -> anyhow::Result<Vec<Value>> {
    let publication = store.extension::<Publication>();
    let guard = publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = guard.as_ref();
    let generation = publication.changed.lock().unwrap_or_else(PoisonError::into_inner)
        .as_ref().map(|changed| *changed.borrow());
    let known = snapshot.is_some_and(|snapshot| snapshot.error.is_none() && snapshot.unknown == 0
        && generation == Some(snapshot.generation)
        && !store.mail_objective_input_coverage_failed()
        && now.saturating_sub(snapshot.observed_ms) <= FRESHNESS
        && now.saturating_sub(snapshot.source_through_ms) <= FRESHNESS);
    let samples = snapshot.map_or(&[][..], |snapshot| snapshot.samples.as_slice());
    let accounting_cut = snapshot.map_or(now, |snapshot| now.min(snapshot.source_through_ms));
    let mut row = ratio_row(&format!("agents/mail-delivery/{}", store.origin()),
        "Durable agent send to confirmed native harness input acceptance", accounting_cut, DEADLINE, 0.05, samples, known);
    let mut latencies = samples.iter().filter_map(|sample| match sample.terminal {
        LifecycleTerminal::Accepted(at) if at >= now.saturating_sub(86_400_000) && at <= now =>
            Some(at.saturating_sub(sample.onset_ms)),
        LifecycleTerminal::Open if now.saturating_sub(sample.onset_ms) > DEADLINE =>
            Some(now.saturating_sub(sample.onset_ms)),
        _ => None,
    }).collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95 = latencies.get((latencies.len() * 95).div_ceil(100).saturating_sub(1)).copied();
    row["diagnostics"] = json!({
        "window_ms": 86_400_000, "p95_ms": p95, "censored_open": true,
        "excluded_undeclared_recipient": samples.iter()
            .filter(|sample| matches!(sample.terminal, LifecycleTerminal::Excluded)).count(),
        "oldest_open_ms": samples.iter().filter(|sample| matches!(sample.terminal, LifecycleTerminal::Open))
            .map(|sample| now.saturating_sub(sample.onset_ms)).max(),
    });
    row["source"] = json!({
        "kind": "native-mail-lifecycle", "host": store.origin(),
        "coverage_known": known, "observed_at_unix_ms": snapshot.map(|snapshot| snapshot.observed_ms),
        "freshness_ttl_ms": FRESHNESS, "source_cut": snapshot.map(|snapshot| snapshot.source_cut),
        "producer_incarnation": publication.incarnation.lock().unwrap_or_else(PoisonError::into_inner).as_deref(),
        "producer_ready": known,
        "capture_ready": snapshot.is_some_and(|snapshot| snapshot.error.is_none()),
        "covered_from_unix_ms": snapshot.map(|snapshot| snapshot.covered_from_ms),
        "covered_through_unix_ms": snapshot.map(|snapshot| snapshot.source_through_ms),
        "held": snapshot.map_or(0, |snapshot| snapshot.held),
        "unknown": snapshot.map_or(1, |snapshot| snapshot.unknown),
        "error": snapshot.and_then(|snapshot| snapshot.error.as_deref()),
    });
    let explain = row["explain"].as_str().unwrap_or_default();
    row["explain"] = json!(format!("{explain} One sample per message and canonical agent recipient on its current declared host. Agent recipients without a declared host are excluded, visible in excluded_undeclared_recipient diagnostics, not successful delivery or unknown coverage. Generic delivered/read assertions never certify acceptance. Native receipts were checked against the physical owner and recipient incarnation when committed; later restarts do not erase historical acceptance. Explicit next-turn holds remain visible separately until their first durable eligibility edge; after release latency still begins at the ORIGINAL durable send, without subtracting hold time. Terminal close/expiry without certified input acceptance is failure, not a successful stop. Malformed source clocks or historical delivered/read without certified acceptance prevents recovery. Budget evaluation stops at the minimum current native producer frontier, not the daemon cache heartbeat; a missing/stale frontier for an owned recipient is unknown. Source capture is reactive and request reads never sample mail."));
    Ok(vec![row])
}

fn canonical(connection: &rusqlite::Connection, cache: &mut BTreeMap<String, String>, recipient: &str) -> anyhow::Result<String> {
    if let Some(canonical) = cache.get(recipient) { return Ok(canonical.clone()); }
    let canonical = crate::store::identities::resolve_reference_conn(connection,
        &crate::store::normalize_message_party(recipient))?;
    cache.insert(recipient.to_owned(), canonical.clone());
    Ok(canonical)
}

fn native_frontier(store: &Store, connection: &rusqlite::Connection, recipient: &str) -> anyhow::Result<Option<u64>> {
    let Some(runtime) = store.latest_claim(recipient, Some("runtime.observed"))? else { return Ok(None) };
    let fields = runtime.body.get("fields").unwrap_or(&runtime.body);
    if fields["status"] != "running" { return Ok(None); }
    let Some(incarnation) = fields["incarnation_id"].as_str() else { return Ok(None) };
    Ok(connection.query_row(
        "SELECT frontier.observed_through_unix_ms FROM local_mailbox_owners owner
         JOIN local_mail_acceptance_frontier_v1 frontier
           ON frontier.recipient=owner.subject AND frontier.incarnation=owner.incarnation
         WHERE owner.subject=?1 AND owner.component='delivery' AND owner.incarnation=?2",
        rusqlite::params![recipient, incarnation], |row| row.get(0),
    ).optional()?)
}

#[cfg(test)]
fn capture(store: &Store, now: u64) -> anyhow::Result<Snapshot> {
    capture_with_progress(store, now, || {})
}

const SOURCE_PAGE: usize = 128;
type SourceRow = (String, String, u64, String);

#[derive(Default)]
struct Facts {
    source: Option<Vec<SourceRow>>,
    acceptance: BTreeMap<String, Vec<(String, u64)>>,
    steps: BTreeMap<String, Vec<(String, String, u64)>>,
}

/// Each indexed page is fully consumed before processing or reading another page.
/// In particular, no read transaction spans the retained-history scan.
fn source_rows(store: &Store, cutoff: u64, progress: &mut impl FnMut()) -> anyhow::Result<Vec<SourceRow>> {
    let connection = store.readers.get();
    let mut result = Vec::new();
    let mut cursor = (cutoff, -1_i64);
    loop {
        let page = {
            let mut query = connection.prepare_cached(
                "SELECT rowid,message,recipient,sent_ms,fields FROM local_mail_delivery_source_v1
                 WHERE (sent_ms,rowid)>(?1,?2) ORDER BY sent_ms,rowid LIMIT ?3")?;
            query.query_map(rusqlite::params![cursor.0, cursor.1, SOURCE_PAGE], |row|
                Ok((row.get::<_, i64>(0)?, (row.get(1)?, row.get(2)?, row.get::<_, u64>(3)?, row.get(4)?))))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        progress();
        let Some((rowid, (_, _, sent, _))) = page.last() else { break };
        cursor = (*sent, *rowid);
        result.extend(page.into_iter().map(|(_, row)| row));
    }
    let mut cursor = (String::new(), String::new());
    loop {
        let page = {
            let mut query = connection.prepare_cached(
                "SELECT source.message,source.recipient,source.sent_ms,source.fields
                 FROM (SELECT message,recipient FROM local_mail_delivery_pending_v1
                       WHERE (message,recipient)>(?1,?2) ORDER BY message,recipient LIMIT ?3) pending
                 JOIN local_mail_delivery_source_v1 source USING(message,recipient)
                 ORDER BY pending.message,pending.recipient")?;
            query.query_map(rusqlite::params![cursor.0, cursor.1, SOURCE_PAGE], |row|
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        progress();
        let Some((message, recipient, _, _)) = page.last() else { break };
        cursor = (message.clone(), recipient.clone());
        // Recent pending rows already appeared in the sent-time scan.
        result.extend(page.into_iter().filter(|(_, _, sent, _)| *sent < cutoff));
    }
    Ok(result)
}

#[cfg(test)]
fn capture_with_progress(store: &Store, now: u64, progress: impl FnMut()) -> anyhow::Result<Snapshot> {
    capture_message(store, now, None, None, &mut BTreeMap::new(), progress)
}

fn capture_message(store: &Store, now: u64, message: Option<&str>, facts: Option<&mut Facts>,
    recipient_inputs: &mut BTreeMap<String, (Option<String>, Option<u64>)>, mut progress: impl FnMut()) -> anyhow::Result<Snapshot> {
        // Incremental capture owns one fence for the whole changed-key cut.
        let changed = message.is_none().then(|| store.subscribe_mail_objective_inputs());
        let generation = changed.as_ref().map_or(0, |changed| *changed.borrow());
        let source_cut = if message.is_none() { store.index()? } else { 0 };
        let connection = store.readers.get();
        let publication = store.extension::<Publication>();
        let mut transient = Facts::default();
        let facts = facts.unwrap_or(&mut transient);
        if facts.source.is_none() {
        let raw = match message {
            Some(message) => {
                let cutoff = now.saturating_sub(HORIZON.saturating_add(DEADLINE).saturating_add(FRESHNESS));
                connection.prepare_cached(
                    "SELECT message,recipient,sent_ms,fields FROM local_mail_delivery_source_v1 source
                     WHERE message=?1 AND (sent_ms>=?2 OR EXISTS(
                         SELECT 1 FROM local_mail_delivery_pending_v1 pending
                         WHERE pending.message=source.message AND pending.recipient=source.recipient))")?
                    .query_map(rusqlite::params![message, cutoff], |row|
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<SourceRow>>>()?
            }
            None => source_rows(store,
                now.saturating_sub(HORIZON.saturating_add(DEADLINE).saturating_add(FRESHNESS)), &mut progress)?,
        };
            publication.work.source_rows.fetch_add(raw.len() as u64, Ordering::Relaxed);
            facts.source = Some(raw);
        }
        let raw = facts.source.as_ref().unwrap();
        #[cfg(test)]
        let mut source_work = [raw.len(), 0, 0];
        let mut source = BTreeMap::<(String, String), (u64, Value)>::new();
        let mut unknown = 0;
        let mut excluded = BTreeMap::<(String, String), u64>::new();
        let mut covered_from_ms = now;
        let mut recapture_at_ms: Option<u64> = None;
        let mut source_through_ms = now;
        let mut checked_recipients = std::collections::BTreeSet::new();
        let mut recipients = BTreeSet::new();
        let mut uncovered_recipients = std::collections::BTreeSet::new();
        // Resolve each distinct reference once per capture. Concurrent relevant input
        // changes invalidate the publication below, including its identity mappings.
        let mut identities = BTreeMap::new();
        for (message, recipient, sent, fields) in raw {
            let sent = *sent;
            let cutoff = now.saturating_sub(HORIZON.saturating_add(DEADLINE).saturating_add(FRESHNESS));
            if sent < cutoff && !connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM local_mail_delivery_pending_v1 WHERE message=?1 AND recipient=?2)",
                rusqlite::params![message, recipient], |row| row.get::<_, bool>(0))? { continue; }
            let recipient = canonical(&connection, &mut identities, recipient)?;
            if !recipient.starts_with("agent/") { continue; }
            recipients.insert(recipient.clone());
            covered_from_ms = covered_from_ms.min(sent);
            let expiry = sent.saturating_add(DEADLINE).saturating_add(HORIZON).saturating_add(FRESHNESS).saturating_add(1);
            if expiry > now { recapture_at_ms = Some(recapture_at_ms.map_or(expiry, |at| at.min(expiry))); }
            if sent > now { unknown += 1; continue; }
            if !recipient_inputs.contains_key(&recipient) {
                publication.work.owner_lookups.fetch_add(1, Ordering::Relaxed);
                let member = connection.query_row("SELECT member FROM desired WHERE subject=?1 AND kind='agent'",
                    [&recipient], |row| row.get::<_, Option<String>>(0)).optional()?.flatten();
                let host = member.map(|member| serde_json::from_str::<Value>(&member))
                    .transpose()?.and_then(|member| member["host"].as_str().map(str::to_owned));
                let frontier = if host.as_deref() == Some(store.origin()) {
                    native_frontier(store, &connection, &recipient)?
                } else { None };
                recipient_inputs.insert(recipient.clone(), (host, frontier));
            }
            let Some((Some(host), frontier)) = recipient_inputs.get(&recipient) else {
                excluded.entry((message.clone(), recipient)).and_modify(|first| *first = (*first).min(sent)).or_insert(sent);
                continue;
            };
            if host != store.origin() { continue; }
            if checked_recipients.insert(recipient.clone()) {
                match *frontier {
                    Some(through) if through <= now => {
                        source_through_ms = source_through_ms.min(through);
                        let expiry = through.saturating_add(FRESHNESS).saturating_add(1);
                        if expiry > now {
                            recapture_at_ms = Some(recapture_at_ms.map_or(expiry, |at| at.min(expiry)));
                        } else { unknown += 1; }
                    }
                    _ => { unknown += 1; uncovered_recipients.insert(recipient.clone()); }
                }
            }
            let fields: Value = serde_json::from_str(fields)?;
            match source.entry((message.clone(), recipient)) {
                std::collections::btree_map::Entry::Vacant(entry) => { entry.insert((sent, fields)); }
                std::collections::btree_map::Entry::Occupied(mut entry) if sent < entry.get().0 => {
                    entry.insert((sent, fields));
                }
                _ => {}
            }
        }
        // Both ledgers retain audit history indefinitely. Read only PK ranges for this
        // host's relevant source population, never global aggregates or identity lookups.
        let messages = source.keys().map(|(message, _)| message).collect::<std::collections::BTreeSet<_>>();
        let mut acceptance_query = connection.prepare_cached(
            "SELECT recipient,MIN(accepted_at_unix_ms) FROM local_mail_input_acceptance_v1
             WHERE message=?1 GROUP BY recipient")?;
        let mut step_query = connection.prepare_cached(
            "SELECT consumer,kind,at_ms FROM local_mail_delivery_steps_v1 WHERE message=?1")?;
        let mut accepted = BTreeMap::<(String, String), u64>::new();
        let mut steps = BTreeMap::<(String, String), BTreeMap<String, u64>>::new();
        for message in messages {
            if !facts.acceptance.contains_key(message) {
                let raw = acceptance_query.query_map([message], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                publication.work.acceptance_rows.fetch_add(raw.len() as u64, Ordering::Relaxed);
                facts.acceptance.insert(message.clone(), raw);
            }
            let raw = &facts.acceptance[message];
            #[cfg(test)]
            { source_work[1] += raw.len(); }
            for (recipient, at) in raw {
                let key = (message.clone(), canonical(&connection, &mut identities, recipient)?);
                accepted.entry(key).and_modify(|first| *first = (*first).min(*at)).or_insert(*at);
            }
            if !facts.steps.contains_key(message) {
                let raw = step_query.query_map([message], |row| Ok((row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?, row.get::<_, u64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                publication.work.lifecycle_rows.fetch_add(raw.len() as u64, Ordering::Relaxed);
                facts.steps.insert(message.clone(), raw);
            }
            let raw = &facts.steps[message];
            #[cfg(test)]
            { source_work[2] += raw.len(); }
            for (consumer, kind, at) in raw {
                // Terminal lifecycle state closes the message, including a runtime refusal
                // before any harness handoff; its actor is not the target input consumer.
                let consumer = if consumer.is_empty() || matches!(kind.as_str(), "message.closed" | "message.expired") {
                    String::new()
                } else { canonical(&connection, &mut identities, consumer)? };
                steps.entry((message.clone(), consumer)).or_default().entry(kind.clone())
                    .and_modify(|first| *first = (*first).min(*at)).or_insert(*at);
            }
            progress();
        }
        // The mirror's own cut must include every stream item visible to its shard.
        if message.is_none() {
            let shard = crate::store::messages_shard::mailbox_reads::position(store)?;
            let mirrored = crate::store::stream_mirror::mail_source_sent_through(&connection)?;
            let lifecycle: Option<String> = connection.query_row(
                "SELECT value FROM meta WHERE key='mail_delivery_source_v1_stream_through'", [], |row| row.get(0)).optional()?;
            if mirrored < shard.admitted || lifecycle.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0) < shard.lifecycle {
                unknown += 1;
            }
        }
        let mut samples = Vec::with_capacity(source.len() + excluded.len());
        samples.extend(excluded.into_values().map(|onset_ms|
            LifecycleSample { onset_ms, terminal: LifecycleTerminal::Excluded }));
        let mut held = 0;
        for ((message, recipient), (sent, fields)) in source {
            if uncovered_recipients.contains(&recipient) { continue; }
            let lifecycle = steps.get(&(message.clone(), recipient.clone()));
            let any = steps.get(&(message.clone(), String::new()));
            let step = |kind: &str| lifecycle.and_then(|steps| steps.get(kind)).into_iter()
                .chain(any.and_then(|steps| steps.get(kind))).copied().min();
            if sent > source_through_ms || fields["sent_clock_valid"].as_i64() != Some(1)
                || step("source.clock-invalid").is_some()
                || step("message.staged").is_some_and(|at| at < sent || at > now) {
                unknown += 1;
                continue;
            }
            let tags = fields["tags"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
            let silent = tags.iter().any(|tag| crate::silent::is_silent_tag(tag))
                && !crate::silent::always_wakes(fields["from"].as_str().unwrap_or_default(), &tags);
            let acceptance = accepted.get(&(message, recipient)).copied();
            if acceptance.is_some_and(|at| at < sent || at > now) {
                unknown += 1;
                continue;
            }
            if acceptance.is_none()
                && (step("message.delivered").is_some() || step("message.read").is_some()) {
                unknown += 1;
                continue;
            }
            let terminal_failure = step("message.closed").into_iter().chain(step("message.expired")).min();
            if terminal_failure.is_some_and(|at| at < sent || at > now) {
                unknown += 1;
                continue;
            }
            let onset = if silent {
                if step("message.staged").into_iter().chain(acceptance).any(|at| at <= source_through_ms) { sent }
                else {
                    held += 1;
                    // Never hide an undelivered terminal outcome as a permanent hold.
                    if let Some(failed) = terminal_failure.filter(|at| *at <= source_through_ms) {
                        samples.push(LifecycleSample { onset_ms: sent, terminal: LifecycleTerminal::Failed(failed) });
                    }
                    continue;
                }
            } else { sent };
            let terminal = match (acceptance, terminal_failure) {
                (Some(accepted), Some(failed)) if failed < accepted => LifecycleTerminal::Failed(failed),
                (Some(accepted), _) => LifecycleTerminal::Accepted(accepted),
                (_, Some(failed)) => LifecycleTerminal::Failed(failed),
                _ => LifecycleTerminal::Open,
            };
            if onset.saturating_add(DEADLINE).saturating_add(HORIZON) >= now.min(source_through_ms)
                || matches!(terminal, LifecycleTerminal::Open) {
                samples.push(LifecycleSample { onset_ms: onset, terminal });
            }
        }
        // Paged reads do not share a SQLite snapshot. Only a relevant input advance
        // fences this cut; unrelated graph claims cannot change these source facts.
        // A writer publishes native generations when its loan ends, after COMMIT.
        // Resolve every loan that could have committed between pages before certifying.
        // All statements have been consumed; no SQLite read transaction is held here.
        if let Some(changed) = changed {
            if let Some(shard) = crate::store::messages_shard::shard(store) {
                drop(shard.connection.write());
            }
            drop(store.connection.write());
            if *changed.borrow() != generation || store.mail_objective_input_coverage_failed() {
                unknown += 1;
            }
        }
        if message.is_some() && checked_recipients.is_empty() {
            // Remote-only source rows do not constrain this host's certified frontier.
            source_through_ms = u64::MAX;
        }
        Ok(Snapshot { observed_ms: now, generation: 0, covered_from_ms, source_through_ms, recapture_at_ms,
            recipients,
            #[cfg(test)]
            source_work,
            samples, held, unknown, source_cut, error: None })
}

#[cfg(test)]
#[path = "mail_delivery_objectives_tests.rs"]
mod tests;
