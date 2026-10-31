//! The attention list's refresher. One task folds every person's attention rows off the request
//! path and publishes them; collection windows serve the newest publication under its own cut
//! and never fold attention while it runs. See `store::attention_list`.

use crate::AttentionListStore;
use crate::OwnerListsStore;
use crate::PublishedViewsAttentionStore;
use crate::SummaryListStore;
use crate::SeatsStore;
use super::*;
use crate::store::attention_list::{AttentionDelta, AttentionFleet, AttentionPublication};
use crate::store::owner_lists::OwnerView;
use std::collections::HashMap;

/// The shortest pause between two refreshes. A refresh also pauses as long as it took, so the
/// refresher never takes more than about half a core. Windows never wait for it: a summary
/// selection a window waits for is computed during the pause, once.
const ATTENTION_LIST_REFRESH_PAUSE: Duration = Duration::from_secs(1);

/// How often the rows are evaluated again with no claim: eligibility and grace periods read the
/// clock. The same period collection windows used to reread attention on.
const ATTENTION_LIST_CLOCK: Duration = Duration::from_secs(30);

/// How long after a window last read attention or a summary the refresher keeps folding
/// attention for new commits. Their windows reread at least every clock period while they are
/// open, so this passes only when none is. A window that opens later gets the newest list at
/// once and wakes the refresher.
const ATTENTION_LIST_IDLE: Duration = Duration::from_secs(600);

/// Keep attention, every person's glasses and arrangements, and the summary rows windows read
/// published off the request path. As the daemon starts it folds them once; after that it
/// refreshes, at most once a second and attention only while windows read it, when a commit lands, a roster is published, the clock
/// passes attention's period, or a window asks. A refresh folds attention again only when
/// something attention reads changed (see `Store::attention_list_delta`) and reads again only
/// the people whose glasses or arrangements changed; otherwise each view keeps its rows.
pub fn start_attention_list(state: &AppState) {
    // An escape hatch while the published list is new: windows then fold attention as before.
    if std::env::var_os("ST3_ATTENTION_LIST").is_some_and(|value| value == "off") {
        return;
    }
    let Some(wake) = state.store.start_attention_list_refresher() else {
        return;
    };
    let store = state.store.clone();
    let state = state.clone();
    let mut changed = state.event_notify.subscribe();
    let mut roster = state.store.subscribe_agent_roster();
    let prompts = Arc::new(std::sync::Mutex::new(super::prompt_card::PromptWatch::new(Arc::clone(&wake))));
    tokio::spawn(crate::task_cpu::account("publication.attention", async move {
        // However this task ends, its views stop being served: windows read for themselves.
        struct Stopped(Arc<Store>);
        impl Drop for Stopped {
            fn drop(&mut self) {
                self.0.stop_attention_list_refresher();
            }
        }
        let _stopped = Stopped(store.clone());
        let idle = ATTENTION_LIST_IDLE.as_millis() as u64;
        loop {
            let started = tokio::time::Instant::now();
            let reader = store.clone();
            let summary_state = state.clone();
            // Attention folds only while windows read it (they reread it every clock period),
            // apart from the first fold as the daemon starts. Once views are forgotten (a
            // checkpoint, each chunk of a long catch-up) a node nobody reads does not fold it
            // again; the next window to read it wakes this task.
            let attention = store.attention_list_read_within(idle) || store.attention_list_starting();
            // A prompt card's conversation changed: present the prompts again.
            let prompts_changed = watching(&prompts).take_changed();
            let watch = Arc::clone(&prompts);
            // One view at a time: attention, every person's glasses and arrangements, then the
            // summary rows windows read. One view's failure leaves the others refreshing. A
            // summary selection a window waits for is computed before each view, so the window
            // waits for at most one other view's refresh.
            let refreshed = tokio::task::spawn_blocking(move || crate::task_cpu::blocking("publication.attention", || {
                // A pairing completed or revoked since the last pass changes which windows a
                // session may hold; their windows reread to recheck it, as before.
                if let Err(error) = reader.recheck_pairings() {
                    eprintln!("st3: pairing recheck for published views failed: {error:#}");
                }
                let mut results = Vec::new();
                let waiting = |results: &mut Vec<_>, starting: bool| {
                    if let Some(result) = refresh_waiting_summary(&summary_state, starting) {
                        results.push(("summary", result.map(|_| ())));
                    }
                };
                waiting(&mut results, false);
                results.push(("attention", crate::performance::task("attention/refresh", || {
                    if !attention {
                        return Ok(());
                    }
                    // A prompt card reads its seat's conversation on this node, following it
                    // from before the read; one another member keeps, its follower read last.
                    let pending = |seat: &str| {
                        super::prompt_card::seat_pending_calls(&summary_state, seat, |conversation| {
                            watching(&watch).follow_conversation(conversation)
                        })
                    };
                    watching(&watch).begin();
                    if refresh_attention_list_reading(
                        &reader,
                        |previous| attention_fleet(&reader, previous),
                        Some(&pending),
                        prompts_changed,
                    )? {
                        watching(&watch).finish();
                    }
                    Ok(())
                })));
                // As the daemon starts, the counts every summary shares, once attention is
                // published and before any window asks for a summary.
                waiting(&mut results, true);
                results.push(("glasses", crate::performance::task("person-views/refresh", || {
                    refresh_owner_list(&reader, OwnerView::Glasses)
                })));
                waiting(&mut results, false);
                results.push(("arrangements", crate::performance::task("person-views/refresh", || {
                    refresh_owner_list(&reader, OwnerView::Arrangements)
                })));
                results.push(("summary", crate::performance::task("summary/refresh", || {
                    super::client_v0::summary::refresh_published(
                        &summary_state,
                        ATTENTION_LIST_IDLE.as_millis() as u64,
                        false,
                    )
                    .map(|_| ())
                })));
                results
            }))
            .await;
            match refreshed {
                Ok(results) => {
                    if prompts_changed
                        && results.iter().any(|(view, result)| *view == "attention" && result.is_err())
                    {
                        watching(&prompts).keep_changed();
                    }
                    for (view, result) in results {
                        if let Err(error) = &result {
                            eprintln!("st3: {view} view refresh failed: {error:#}");
                        }
                        store.note_view_refreshed(view, result.is_ok());
                    }
                }
                Err(error) => {
                    // The refresh panicked: what woke it for the prompts is still unread.
                    if prompts_changed {
                        watching(&prompts).keep_changed();
                    }
                    eprintln!("st3: published view refresh stopped: {error}");
                    for view in crate::store::attention_list::PUBLISHED_VIEWS {
                        store.note_view_refreshed(view, false);
                    }
                }
            }
            // The pause bounds this task's work, but a window waiting for a summary selection
            // does not wait through it: each wake computes the selections waiting then.
            let pause = tokio::time::Instant::now() + started.elapsed().max(ATTENTION_LIST_REFRESH_PAUSE);
            let mut woken = false;
            let mut roster_arrived = false;
            loop {
                let summary_state = state.clone();
                let refreshed = if roster_arrived {
                    // A complete roster arrived while no summary could be counted: count every
                    // selection now, not after the pause.
                    roster_arrived = false;
                    tokio::task::spawn_blocking(move || crate::task_cpu::blocking("publication.summary", || {
                        Some(crate::performance::task("summary/refresh", || {
                            super::client_v0::summary::refresh_published(
                                &summary_state,
                                ATTENTION_LIST_IDLE.as_millis() as u64,
                                false,
                            )
                        }))
                    }))
                    .await
                } else if store.summary_selection_waiting() {
                    tokio::task::spawn_blocking(move || crate::task_cpu::blocking("publication.summary", || refresh_waiting_summary(&summary_state, false))).await
                } else {
                    Ok(None)
                };
                match refreshed {
                    Ok(None) => {}
                    Ok(Some(result)) => {
                        if let Err(error) = &result {
                            eprintln!("st3: summary view refresh failed: {error:#}");
                        }
                        store.note_view_refreshed("summary", result.is_ok());
                    }
                    Err(error) => {
                        eprintln!("st3: summary view refresh stopped: {error}");
                        store.note_view_refreshed("summary", false);
                    }
                }
                tokio::select! {
                    () = tokio::time::sleep_until(pause) => break,
                    () = wake.notified() => woken = true,
                    // With no shared counts (as the daemon starts, or after views are
                    // forgotten) every summary waits for the complete roster they count agents
                    // from. Its publication is all they wait for, so it ends their wait.
                    result = roster.changed(), if store.summary_shared().is_none() => {
                        if result.is_err() {
                            return;
                        }
                        roster_arrived = true;
                    }
                }
            }
            // A window asked during the pause: refresh again at once.
            if woken {
                continue;
            }
            // Attention is evaluated again when its clock period ends or a claim it read
            // becomes eligible, counted from when it was evaluated, not from this wait.
            let due = store
                .attention_list_due_at(ATTENTION_LIST_CLOCK.as_millis())
                .map_or(ATTENTION_LIST_CLOCK, |due| {
                    Duration::from_millis(due.saturating_sub(client_now_ms()) as u64)
                });
            // A glasses or arrangements window rereads only when its view publishes, so every
            // commit is weighed; with nothing changed that is a few indexed reads.
            tokio::select! {
                () = wake.notified() => {}
                result = changed.changed() => {
                    if result.is_err() {
                        return;
                    }
                }
                // The summary counts working agents from the published roster.
                result = roster.changed() => {
                    if result.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(due) => {}
            }
        }
    }));
}

/// The refresher's prompt watch, which its blocking refresh and its wait share.
fn watching(
    prompts: &std::sync::Mutex<super::prompt_card::PromptWatch>,
) -> std::sync::MutexGuard<'_, super::prompt_card::PromptWatch> {
    prompts.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Publish the summary selections windows are waiting for, if any: a window that opens a new
/// selection gets its row without waiting for a whole refresh. While `starting`, also the
/// shared counts the daemon has not computed yet.
fn refresh_waiting_summary(state: &AppState, starting: bool) -> Option<anyhow::Result<bool>> {
    (state.store.summary_selection_waiting()
        || starting && state.store.summary_shared_starting())
    .then(|| {
        crate::performance::task("summary/refresh", || {
            super::client_v0::summary::refresh_published(
                state,
                ATTENTION_LIST_IDLE.as_millis() as u64,
                true,
            )
        })
    })
}

/// Publish the list at the current cut in one short snapshot: the same rows when nothing
/// attention reads changed since the newest publication, else a fold of every person's rows.
/// A fold that comes out equal keeps the published rows, so windows have nothing to reread.
/// The clock is read inside the snapshot: every local claim in it was accepted before then.
/// Each row carries its presentation, presented again when the agents it reads change.
pub fn refresh_attention_list(store: &Store) -> anyhow::Result<()> {
    refresh_attention_list_with(store, |previous| attention_fleet(store, previous))
}

/// `refresh_attention_list`, presenting with the fleet `fleet` gives for the previous
/// publication's fleet.
pub fn refresh_attention_list_with(
    store: &Store,
    fleet: impl FnOnce(Option<&Arc<AttentionFleet>>) -> Option<Arc<AttentionFleet>>,
) -> anyhow::Result<()> {
    refresh_attention_list_reading(store, fleet, None, false).map(|_| ())
}

/// `refresh_attention_list_with`, giving each prompt card the calls `pending` reads for its
/// seat, and presenting them again when `prompts_changed` (a conversation they read changed)
/// even with nothing else changed. Without `pending` a prompt card carries no prompt. Returns
/// whether it presented the prompts.
fn refresh_attention_list_reading(
    store: &Store,
    fleet: impl FnOnce(Option<&Arc<AttentionFleet>>) -> Option<Arc<AttentionFleet>>,
    pending: Option<super::prompt_card::PendingCalls>,
    prompts_changed: bool,
) -> anyhow::Result<bool> {
    let prompts_changed = prompts_changed && pending.is_some();
    let (generation, previous) = store.newest_attention_list();
    let clock = ATTENTION_LIST_CLOCK.as_millis();
    // Read before the snapshot: a roster published in between presents again at the next wake.
    let fleet = fleet(
        previous
            .as_ref()
            .and_then(|previous| previous.fleet.as_ref()),
    );
    let fleet_changed = previous
        .as_ref()
        .is_none_or(|previous| previous.fleet.as_deref() != fleet.as_deref());
    store.read_snapshot(|index| {
        let now = client_now_ms();
        let mut presented = false;
        let (cause, due) = match &previous {
            None => (Some("first".to_owned()), None),
            Some(previous)
                if now < previous.evaluated_at_unix_ms
                    || now - previous.evaluated_at_unix_ms >= clock =>
            {
                (Some("clock".to_owned()), None)
            }
            Some(previous) if previous.due_at_unix_ms.is_some_and(|due| now >= due) => {
                (Some("due".to_owned()), None)
            }
            Some(previous) => match store.attention_list_delta(previous, index, now)? {
                AttentionDelta::Unchanged(None)
                    if previous.cut == index && !fleet_changed && !prompts_changed =>
                {
                    return Ok(false);
                }
                AttentionDelta::Unchanged(due) => (None, due),
                AttentionDelta::Refold(cause) => (Some(cause), None),
            },
        };
        let projection = store.attention_projection_frontier()?;
        let ask_deadline_policy =
            crate::store::person_work::ask_deadline_policy(&store.readers.get())?;
        let (rows, evaluated_at_unix_ms, due_at_unix_ms, future_after) = match (cause, &previous) {
            (None, Some(previous)) => {
                // Still pending future acceptance times stand beside the new ones.
                let due = [previous.due_at_unix_ms, due].into_iter().flatten().min();
                let future_after = due.and(previous.future_after.or(Some(previous.cut)));
                let rows = if fleet_changed || prompts_changed {
                    let mut rows = (*previous.rows).clone();
                    if fleet_changed {
                        present_attention(store, &mut rows, fleet.as_deref())?;
                    }
                    presented = present_prompts(&mut rows, pending);
                    if rows == *previous.rows {
                        Arc::clone(&previous.rows)
                    } else {
                        Arc::new(rows)
                    }
                } else {
                    Arc::clone(&previous.rows)
                };
                (rows, previous.evaluated_at_unix_ms, due, future_after)
            }
            (cause, _) => {
                store.note_attention_list_fold(cause.as_deref().unwrap_or("first"));
                let mut rows = client_attention_resources_at(store, None, false, now)?;
                present_attention(store, &mut rows, fleet.as_deref())?;
                presented = present_prompts(&mut rows, pending);
                let rows = match &previous {
                    Some(previous) if *previous.rows == rows => Arc::clone(&previous.rows),
                    _ => Arc::new(rows),
                };
                // Claims dated after `now` were folded as not yet eligible: those since the
                // previous cut, and earlier ones still pending. Evaluate again when the first
                // of them is.
                let start = previous
                    .as_ref()
                    .map(|previous| previous.future_after.unwrap_or(previous.cut).min(previous.cut));
                let due = match start {
                    Some(start) if start < index => store.attention_future_since(start, index, now)?,
                    _ => None,
                };
                (rows, now, due, due.and(start))
            }
        };
        let changed = previous
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(&previous.rows, &rows));
        let owners = if let Some(previous) = previous.as_ref().filter(|_| !changed) {
            previous.owners.clone()
        } else {
            AttentionPublication::index_owners(&rows)
        };
        let published_at_unix_ms = client_now_ms();
        store.publish_attention_list(
            generation,
            AttentionPublication {
                cut: index,
                projection,
                evaluated_at_unix_ms,
                due_at_unix_ms,
                future_after,
                published_at_unix_ms,
                rows,
                fleet,
                owners,
                ask_deadline_policy,
            },
            changed,
        );
        // While windows read attention it publishes at least once a clock period. A previous
        // publication older than two was from before the refresher followed commits again: its
        // commits waited for a reader, not for a fold, and are not a freshness sample.
        if let Some(previous) = previous.as_ref().filter(|previous| {
            published_at_unix_ms.saturating_sub(previous.published_at_unix_ms) <= 2 * clock
        }) {
            super::record_publish_latency(
                store,
                "attention",
                previous.cut,
                index,
                published_at_unix_ms,
            );
        }
        Ok(presented)
    })
}

/// The newest published agents roster as the attention presentation reads it, reusing
/// `previous` while no roster has been published since, and while none is available (its cache
/// cleared, or no roster refresher), so rows keep their agents until the next roster. None
/// until a roster is published.
pub fn attention_fleet(
    store: &Store,
    previous: Option<&Arc<AttentionFleet>>,
) -> Option<Arc<AttentionFleet>> {
    let revision = store.agent_roster_revision();
    if let Some(previous) = previous.filter(|previous| previous.revision == revision) {
        return Some(Arc::clone(previous));
    }
    let Some((_, cards, _)) = store.published_agent_roster(u64::MAX, false) else {
        return previous.cloned();
    };
    Some(Arc::new(AttentionFleet::from_cards(revision, &cards)))
}

/// What the attention presentation reads beside a row: the fleet, and the paths of the steps
/// the rows hold.
struct Known<'a> {
    fleet: &'a AttentionFleet,
    paths: &'a HashMap<String, String>,
}

impl fractal_ui_model::attention::Fleet for Known<'_> {
    fn agent_label(&self, agent: &str) -> Option<String> {
        self.fleet.agent_label(agent)
    }
    fn claimant(&self, step: &str) -> Option<String> {
        self.fleet.claimant(step)
    }
    fn working_in(&self, mission: &str) -> Option<String> {
        self.fleet.working_in(mission)
    }
    fn step_path(&self, step: &str) -> Option<String> {
        self.paths.get(step).cloned()
    }
}

/// Give each row the presentation st serves for it (docs/clients/ui-contract.md, Attention
/// row), reading `fleet`, or no agents before the first roster. A row it cannot read goes
/// without one, and a step whose path it cannot read is named on no card; neither fails the
/// list for every client.
pub fn present_attention(
    store: &Store,
    rows: &mut [Value],
    fleet: Option<&AttentionFleet>,
) -> anyhow::Result<()> {
    crate::performance::task("attention/present", || present_rows(store, rows, fleet))
}

/// Give each prompt card's presentation its prompt: the call it asks about, while `pending` reads
/// it without a result in its seat's conversation. Returns whether it read any: without
/// `pending` a card carries no prompt.
fn present_prompts(rows: &mut [Value], pending: Option<super::prompt_card::PendingCalls>) -> bool {
    let Some(pending) = pending else {
        return false;
    };
    for row in rows
        .iter_mut()
        .filter(|row| row["attention_kind"] == "harness-prompt" && row["presentation"].is_object())
    {
        let seat = row["source_id"].as_str().map(str::to_owned);
        let calls = || match seat {
            Some(seat) => pending(&seat),
            None => Err("st does not know which seat shows this prompt".to_owned()),
        };
        row["presentation"]["prompt"] = super::prompt_card::prompt_card(row, calls);
    }
    true
}

fn present_rows(
    store: &Store,
    rows: &mut [Value],
    fleet: Option<&AttentionFleet>,
) -> anyhow::Result<()> {
    let none = AttentionFleet::default();
    let mut paths = HashMap::new();
    for step in rows.iter().filter_map(|row| row["step_run_id"].as_str()) {
        if paths.contains_key(step) {
            continue;
        }
        match store.step_path(step) {
            Ok(Some(path)) => {
                paths.insert(step.to_owned(), path);
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("st3: WARN attention cannot read the path of {step}: {error:#}")
            }
        }
    }
    let known = Known {
        fleet: fleet.unwrap_or(&none),
        paths: &paths,
    };
    for row in rows {
        match fractal_ui_model::attention::Facts::deserialize(&*row) {
            Ok(facts) => {
                let mut presentation =
                    serde_json::to_value(fractal_ui_model::attention::present(&facts, &known))?;
                // A revision card names what the candidate changes. One st cannot read names
                // nothing; it never fails the list.
                if facts.attention_kind == "revision-approval" {
                    match super::revision_changes::revision_card(store, row) {
                        Ok(Some(revision)) => presentation["revision"] = revision,
                        Ok(None) => {}
                        Err(error) => eprintln!(
                            "st3: WARN attention {} names no revision changes: {error:#}",
                            row["id"]
                        ),
                    }
                }
                // A launch card previews its newest variant's mission, or says why it cannot.
                if facts.attention_kind == "launch-approval" {
                    match super::launch_card::launch_card(store, row, |agent| {
                        known.fleet.agent_label(agent)
                    }) {
                        Ok(Some(launch)) => presentation["launch"] = launch,
                        Ok(None) => {}
                        Err(error) => eprintln!(
                            "st3: WARN attention {} has no launch preview: {error:#}",
                            row["id"]
                        ),
                    }
                }
                row["presentation"] = presentation;
            }
            Err(error) => eprintln!(
                "st3: WARN attention {} has no presentation: {error}",
                row["id"]
            ),
        }
    }
    Ok(())
}

/// Copy each item's presentation from the newest published list, which presents every row;
/// a read computes none. An item the list has not published yet goes without one until it is.
pub(crate) fn published_attention_presentations(store: &Store, items: &mut [Value]) {
    let Some(publication) = store.published_attention_list() else {
        return;
    };
    let presentations = publication
        .rows
        .iter()
        .filter_map(|row| Some((row["id"].as_str()?, row.get("presentation")?)))
        .collect::<HashMap<_, _>>();
    for item in items {
        if let Some(presentation) = item["id"].as_str().and_then(|id| presentations.get(id)) {
            item["presentation"] = (*presentation).clone();
        }
    }
}

/// Publish every person's rows of `view` (glasses or arrangements) at the current cut in one
/// short snapshot, reading again only the people whose rows may have changed.
pub fn refresh_owner_list(store: &Store, view: OwnerView) -> anyhow::Result<()> {
    if store.refresh_owner_list(view, client_now_ms())? {
        store.publish_collection_view(view.collection());
    }
    Ok(())
}

/// A window's rows of a person-owned view (glasses, arrangements): `person`'s rows in the newest
/// publication, with its cut and publication time. `None` while a refresher runs but has
/// published nothing yet; the refresher is woken either way when the publication is older than
/// the window's snapshot at `index`.
pub fn published_owner_rows(
    store: &Store,
    view: OwnerView,
    index: u64,
    person: &str,
) -> Option<(u64, u128, Vec<Value>)> {
    let Some(publication) = store.published_owner_list(view) else {
        store.request_attention_list_refresh();
        return None;
    };
    if publication.cut < index {
        store.request_attention_list_refresh();
    }
    let rows = publication
        .owners
        .get(person)
        .map(|rows| (**rows).clone())
        .unwrap_or_default();
    Some((publication.cut, publication.published_at_unix_ms, rows))
}

/// A window's attention rows: the newest publication's rows for `person` (every person's for
/// none), at most `keep` of them, with that publication's cut and publication time. `None`
/// while a refresher runs but has published nothing yet; the refresher is woken either way
/// when the publication is older than `index`.
pub fn published_attention_rows(
    store: &Store,
    index: u64,
    person: Option<&str>,
    keep: usize,
) -> Option<(u64, u128, Vec<Value>)> {
    let Some(publication) = store.published_attention_list() else {
        store.request_attention_list_refresh();
        return None;
    };
    if publication.cut < index {
        store.request_attention_list_refresh();
    }
    let rows = publication
        .rows
        .iter()
        .filter(|row| person.is_none_or(|person| row["person_id"] == person))
        .take(keep)
        .cloned()
        .collect();
    Some((publication.cut, publication.published_at_unix_ms, rows))
}

/// Shared owner-indexed attention rows: no scan or content cloning on the read path.
pub fn published_assistant_attention_rows(
    store: &Store,
    index: u64,
    person: &str,
) -> Option<(u64, u128, Arc<Vec<Value>>)> {
    let Some(publication) = store.published_attention_list() else {
        store.request_attention_list_refresh();
        return None;
    };
    if publication.cut < index {
        store.request_attention_list_refresh();
    }
    let rows = publication.owners.get(person).cloned().unwrap_or_default();
    Some((publication.cut, publication.published_at_unix_ms, rows))
}

/// A window of a published view before its first publication: retry shortly.
pub fn published_view_not_ready(collection: &str) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: format!("{collection}-not-ready"),
        message: format!("the {collection} view is still being prepared; retry shortly"),
        details: Box::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_card_reads_its_seat_and_no_other_card_reads_one() {
        let mut rows = vec![
            json!({
                "attention_kind": "harness-prompt",
                "source_id": "agent/example/oak",
                "action_parameters": {"prompt.respond": {"answers": ["allow", "deny"], "call_id": "call-a"}},
                "presentation": {"kind": "prompt"},
            }),
            json!({"attention_kind": "human-gate", "source_id": "agent/example/elm", "presentation": {"kind": "review"}}),
            json!({
                "attention_kind": "harness-prompt",
                "source_id": "agent/example/ash",
                "action_parameters": {"prompt.respond": {"answers": ["allow", "deny"], "call_id": "call-b"}},
                "presentation": {"kind": "prompt"},
            }),
            // A prompt that names no call reads no conversation.
            json!({"attention_kind": "harness-prompt", "source_id": "agent/example/fir", "presentation": {"kind": "prompt"}}),
        ];
        let read = std::sync::Mutex::new(Vec::new());
        let pending = |seat: &str| {
            read.lock().unwrap().push(seat.to_owned());
            match seat {
                "agent/example/oak" => Ok(vec![json!({"call_id": "call-a", "name": "Bash", "arguments": {"command": "ls"}})]),
                _ => Err("the seat runs elsewhere".to_owned()),
            }
        };
        assert!(present_prompts(&mut rows, Some(&pending)));
        assert_eq!(*read.lock().unwrap(), ["agent/example/oak", "agent/example/ash"]);
        assert_eq!(rows[0]["presentation"]["prompt"]["calls"][0]["tool"], "Bash");
        assert_eq!(rows[0]["presentation"]["prompt"]["allow"], true);
        assert!(rows[1]["presentation"].get("prompt").is_none());
        assert_eq!(rows[2]["presentation"]["prompt"]["unknown_reason"], "the seat runs elsewhere");
        assert_eq!(rows[3]["presentation"]["prompt"], json!({"calls": [], "answers": [], "allow": false}));

        // Without a reader, as in a fold that is not the refresher's, a card carries no prompt.
        let mut rows = vec![json!({"attention_kind": "harness-prompt", "source_id": "agent/example/oak", "presentation": {}})];
        assert!(!present_prompts(&mut rows, None));
        assert!(rows[0]["presentation"].get("prompt").is_none());
    }

    /// A seat's conversation changes no claim attention reads. The refresher follows each
    /// card's transcript from before it reads it, so a call or result written at any moment
    /// after that read, even before the read is published, wakes it; it then presents the
    /// prompts again at the same cut, without a claim or the clock.
    #[tokio::test]
    async fn a_conversation_change_presents_the_prompts_again_without_a_claim() {
        let root = tempfile::tempdir().unwrap();
        let transcript = root.path().join("oak.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let append = |line: Value| {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
            writeln!(file, "{line}").unwrap();
        };
        let (store, wake, cut) = with_prompt_card("call-a");
        let prompts = std::sync::Mutex::new(super::super::prompt_card::PromptWatch::new(Arc::clone(&wake)));
        // The first read races the seat: the call lands just after it.
        let racing = std::sync::atomic::AtomicBool::new(true);
        let pending = |_: &str| {
            watching(&prompts).follow(&transcript)?;
            let lines = std::fs::read_to_string(&transcript).unwrap();
            let entries = lines.lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
            let answered = entries.iter().filter_map(|entry| entry["result"].as_str()).collect::<Vec<_>>();
            let calls = entries
                .iter()
                .filter(|entry| entry["call"].as_str().is_some_and(|call| !answered.contains(&call)))
                .map(|entry| json!({"call_id": entry["call"], "name": "Bash", "arguments": {"command": "make"}, "complete": true}))
                .collect();
            if racing.swap(false, std::sync::atomic::Ordering::AcqRel) {
                append(json!({"call": "call-a"}));
            }
            Ok(calls)
        };
        let prompt = || published_prompt(&store);
        let refresh = |changed: bool| {
            watching(&prompts).begin();
            let presented =
                refresh_attention_list_reading(&store, |previous| previous.cloned(), Some(&pending), changed).unwrap();
            if presented {
                watching(&prompts).finish();
            }
            presented
        };
        let woken = || async {
            let take = || watching(&prompts).take_changed();
            assert!(super::super::prompt_card::tests::changed_by(take, &wake, Duration::from_secs(5)).await);
        };

        // Nothing changed: nothing is read.
        assert!(!refresh(false));
        // The first fold reads the card's conversation before the seat makes its call.
        assert!(refresh(true));
        assert_eq!(prompt(), json!({"calls": [], "answers": ["allow", "deny"], "allow": false}));
        // The call landed after that read: it wakes the refresher, which presents it.
        woken().await;
        assert!(refresh(true));
        assert_eq!(prompt()["calls"][0]["call_id"], "call-a");
        assert_eq!(prompt()["allow"], true);
        // The call has its result.
        append(json!({"result": "call-a"}));
        woken().await;
        refresh(true);
        assert_eq!(prompt(), json!({"calls": [], "answers": ["allow", "deny"], "allow": false}));
        assert_eq!(published_cut(&store), cut, "no claim was read");
    }

    /// OpenCode keeps a conversation in SQLite and commits to its write-ahead log, leaving the
    /// database file as it was until a checkpoint. Its call and then its result, committed there
    /// alone, present the prompt again through the real reader and watch, without a claim or the
    /// clock.
    #[tokio::test]
    async fn an_opencode_call_and_its_result_in_the_write_ahead_log_present_the_prompt_again() {
        use crate::external_sessions::{ExternalDriver, ExternalSession};
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        let db = rusqlite::Connection::open(&database).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        db.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER);\
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);\
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);\
             INSERT INTO session VALUES ('ses_oak', '/work/example', 'Oak', 1, 1);\
             INSERT INTO message VALUES ('msg_1', 'ses_oak', 2, 2, '{\"role\":\"assistant\"}');",
        )
        .unwrap();
        let source = ExternalSession {
            id: "session/external-opencode-ses_oak".into(),
            revision: "revision-test".into(),
            driver: ExternalDriver::OpenCode,
            native_id: "ses_oak".into(),
            transcript: database.clone(),
            codex_home: None,
            cwd: None,
            title: None,
            started_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            process: None,
        };
        let (store, wake, cut) = with_prompt_card("call_oak");
        let prompts = std::sync::Mutex::new(super::super::prompt_card::PromptWatch::new(Arc::clone(&wake)));
        let pending = |_: &str| {
            super::super::prompt_card::source_pending_calls(&source, |transcript| watching(&prompts).follow(transcript))
        };
        let refresh = || {
            watching(&prompts).begin();
            if refresh_attention_list_reading(&store, |previous| previous.cloned(), Some(&pending), true).unwrap() {
                watching(&prompts).finish();
            }
        };
        let woken = || async {
            let take = || watching(&prompts).take_changed();
            assert!(super::super::prompt_card::tests::changed_by(take, &wake, Duration::from_secs(5)).await);
        };
        let part = |state: Value| json!({"type": "tool", "callID": "call_oak", "tool": "bash", "state": state}).to_string();

        refresh();
        assert_eq!(published_prompt(&store)["calls"], json!([]));
        db.execute(
            "INSERT INTO part VALUES ('part_1', 'msg_1', 'ses_oak', 3, 3, ?1)",
            [part(json!({"status": "running", "input": {"command": "make release"}}))],
        )
        .unwrap();
        assert!(super::super::prompt_card::tests::wal_holds_a_commit(&database), "the call is in the log");
        woken().await;
        refresh();
        assert_eq!(published_prompt(&store)["calls"][0]["call_id"], "call_oak");
        assert_eq!(published_prompt(&store)["calls"][0]["input"], r#"{"command":"make release"}"#);
        assert_eq!(published_prompt(&store)["allow"], true);
        db.execute(
            "UPDATE part SET time_updated = 4, data = ?1 WHERE id = 'part_1'",
            [part(json!({"status": "completed", "input": {"command": "make release"}, "output": "done"}))],
        )
        .unwrap();
        woken().await;
        refresh();
        assert_eq!(published_prompt(&store), json!({"calls": [], "answers": ["allow", "deny"], "allow": false}));
        assert_eq!(published_cut(&store), cut, "no claim was read");
    }

    /// A store whose published attention list has one prompt card, for seat `agent/example/oak`,
    /// asking about `call`; the refresher's wake and the publication's cut.
    fn with_prompt_card(call: &str) -> (Store, Arc<tokio::sync::Notify>, u64) {
        let store = Store::open_memory("node").unwrap();
        let wake = store.start_attention_list_refresher().unwrap();
        refresh_attention_list(&store).unwrap();
        let (generation, first) = store.newest_attention_list();
        let first = first.unwrap();
        let mut rows = (*first.rows).clone();
        rows.push(json!({
            "id": "attention/prompt",
            "attention_kind": "harness-prompt",
            "source_id": "agent/example/oak",
            "action_parameters": {"prompt.respond": {"answers": ["allow", "deny"], "call_id": call}},
            "presentation": {"kind": "prompt"},
        }));
        store.publish_attention_list(
            generation,
            AttentionPublication {
                cut: first.cut,
                projection: first.projection.clone(),
                evaluated_at_unix_ms: first.evaluated_at_unix_ms,
                due_at_unix_ms: first.due_at_unix_ms,
                future_after: first.future_after,
                published_at_unix_ms: first.published_at_unix_ms,
                owners: AttentionPublication::index_owners(&rows),
                rows: Arc::new(rows),
                fleet: first.fleet.clone(),
                ask_deadline_policy: first.ask_deadline_policy,
            },
            true,
        );
        (store, wake, first.cut)
    }

    fn published_prompt(store: &Store) -> Value {
        let (_, publication) = store.newest_attention_list();
        let publication = publication.unwrap();
        let row = publication.rows.iter().find(|row| row["id"] == "attention/prompt").unwrap();
        row["presentation"]["prompt"].clone()
    }

    fn published_cut(store: &Store) -> u64 {
        store.newest_attention_list().1.unwrap().cut
    }
}
