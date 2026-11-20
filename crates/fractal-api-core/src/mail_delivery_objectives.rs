//! Owner-host mail latency from transaction-fed, retained source facts. Reads never sample mail.
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::sync::atomic::{AtomicBool, Ordering};

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
    source_cut: u64,
    error: Option<String>,
    #[cfg(test)]
    source_work: [usize; 3],
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

/// One subscription owns source capture. Timers renew the producer heartbeat and wake the
/// objective evaluator at actual lifecycle/budget boundaries; they never poll source tables.
pub fn start(state: &crate::api::AppState) {
    let publication = state.store.extension::<Publication>();
    if publication.started.swap(true, Ordering::AcqRel) { return; }
    *publication.incarnation.lock().unwrap_or_else(PoisonError::into_inner) = Some(crate::model::new_request_id());
    let state = state.clone();
    let mut changed = state.store.subscribe_mail_objective_inputs();
    *publication.changed.lock().unwrap_or_else(PoisonError::into_inner) = Some(changed.clone());
    tokio::spawn(async move {
        loop {
            let generation = *changed.borrow_and_update();
            #[cfg(test)]
            publication.captures.fetch_add(1, Ordering::AcqRel);
            let store = state.store.clone();
            let captured = tokio::task::spawn_blocking(move || capture(&store, now())).await;
            let mut snapshot = match captured {
                Ok(Ok(snapshot)) => snapshot,
                error => Snapshot { observed_ms: now(), generation, samples: Vec::new(), held: 0, unknown: 1,
                    covered_from_ms: now(), source_through_ms: 0, recapture_at_ms: None,
                    #[cfg(test)]
                    source_work: [0; 3],
                    source_cut: 0, error: Some(format!("mail source capture unavailable: {error:?}")) },
            };
            snapshot.generation = generation;
            *publication.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
            let store = state.store.clone();
            let state_dir = state.state_dir.clone();
            let reported = tokio::task::spawn_blocking(move || {
                let rows = crate::api::operations::current_slo_rows(&store, &state_dir)?;
                crate::declared_objectives::check_and_report(&store, &rows);
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
    row["explain"] = json!(format!("{explain} One sample per message and canonical agent recipient on its current declared host. Generic delivered/read assertions never certify acceptance. Native receipts were checked against the physical owner and recipient incarnation when committed; later restarts do not erase historical acceptance. Explicit next-turn holds remain visible separately until their first durable eligibility edge; after release latency still begins at the ORIGINAL durable send, without subtracting hold time. Terminal close/expiry without certified input acceptance is failure, not a successful stop. Unknown ownership, malformed source clocks or historical delivered/read without certified acceptance prevents recovery. Budget evaluation stops at the minimum current native producer frontier, not the daemon cache heartbeat; a missing/stale frontier is unknown. Source capture is reactive and request reads never sample mail."));
    Ok(vec![row])
}

fn canonical(connection: &rusqlite::Connection, recipient: &str) -> anyhow::Result<String> {
    Ok(crate::store::identities::resolve_reference_conn(connection,
        &crate::store::normalize_message_party(recipient))?)
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

fn capture(store: &Store, now: u64) -> anyhow::Result<Snapshot> {
    capture_with_progress(store, now, || {})
}

const SOURCE_PAGE: usize = 128;
type SourceRow = (String, String, u64, String);

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

fn capture_with_progress(store: &Store, now: u64, mut progress: impl FnMut()) -> anyhow::Result<Snapshot> {
        let changed = store.subscribe_mail_objective_inputs();
        let generation = *changed.borrow();
        let source_cut = store.index()?;
        let owners = store.desired_subjects()?.into_iter().filter(|subject| subject.kind == "agent")
            .map(|subject| (subject.subject, subject.member.map(|member| member.host)))
            .collect::<BTreeMap<_, _>>();
        let connection = store.readers.get();
        let raw = source_rows(store,
            now.saturating_sub(HORIZON.saturating_add(DEADLINE).saturating_add(FRESHNESS)), &mut progress)?;
        #[cfg(test)]
        let mut source_work = [raw.len(), 0, 0];
        let mut source = BTreeMap::<(String, String), (u64, Value)>::new();
        let mut unknown = 0;
        let mut covered_from_ms = now;
        let mut recapture_at_ms: Option<u64> = None;
        let mut source_through_ms = now;
        let mut checked_recipients = std::collections::BTreeSet::new();
        let mut uncovered_recipients = std::collections::BTreeSet::new();
        for (message, recipient, sent, fields) in raw {
            let recipient = canonical(&connection, &recipient)?;
            if !recipient.starts_with("agent/") { continue; }
            covered_from_ms = covered_from_ms.min(sent);
            let expiry = sent.saturating_add(DEADLINE).saturating_add(HORIZON).saturating_add(FRESHNESS).saturating_add(1);
            if expiry > now { recapture_at_ms = Some(recapture_at_ms.map_or(expiry, |at| at.min(expiry))); }
            if sent > now { unknown += 1; continue; }
            let Some(Some(host)) = owners.get(&recipient) else { unknown += 1; continue; };
            if host != store.origin() { continue; }
            if checked_recipients.insert(recipient.clone()) {
                match native_frontier(store, &connection, &recipient)? {
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
            let fields: Value = serde_json::from_str(&fields)?;
            match source.entry((message, recipient)) {
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
            let raw = acceptance_query.query_map([message], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            #[cfg(test)]
            { source_work[1] += raw.len(); }
            for (recipient, at) in raw {
                let key = (message.clone(), canonical(&connection, &recipient)?);
                accepted.entry(key).and_modify(|first| *first = (*first).min(at)).or_insert(at);
            }
            let raw = step_query.query_map([message], |row| Ok((row.get::<_, String>(0)?,
                row.get::<_, String>(1)?, row.get::<_, u64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            #[cfg(test)]
            { source_work[2] += raw.len(); }
            for (consumer, kind, at) in raw {
                // Terminal lifecycle state closes the message, including a runtime refusal
                // before any harness handoff; its actor is not the target input consumer.
                let consumer = if consumer.is_empty() || matches!(kind.as_str(), "message.closed" | "message.expired") {
                    String::new()
                } else { canonical(&connection, &consumer)? };
                steps.entry((message.clone(), consumer)).or_default().entry(kind)
                    .and_modify(|first| *first = (*first).min(at)).or_insert(at);
            }
            progress();
        }
        // The mirror's own cut must include every stream item visible to its shard.
        let shard = crate::store::messages_shard::mailbox_reads::position(store)?;
        let mirrored = crate::store::stream_mirror::mail_source_sent_through(&connection)?;
        let lifecycle: Option<String> = connection.query_row(
            "SELECT value FROM meta WHERE key='mail_delivery_source_v1_stream_through'", [], |row| row.get(0)).optional()?;
        if mirrored < shard.admitted || lifecycle.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0) < shard.lifecycle {
            unknown += 1;
        }
        let mut samples = Vec::with_capacity(source.len());
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
        if let Some(shard) = crate::store::messages_shard::shard(store) {
            drop(shard.connection.write());
        }
        drop(store.connection.write());
        if *changed.borrow() != generation || store.mail_objective_input_coverage_failed() {
            unknown += 1;
        }
        Ok(Snapshot { observed_ms: now, generation: 0, covered_from_ms, source_through_ms, recapture_at_ms,
            #[cfg(test)]
            source_work,
            samples, held, unknown, source_cut, error: None })
}

#[cfg(test)]
#[path = "mail_delivery_objectives_tests.rs"]
mod tests;
