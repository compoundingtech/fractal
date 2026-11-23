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
use fractal_store::store::attention_list::AttentionPresentationDependencies;
use std::cell::RefCell;

/// How long after a window last read attention or a summary the refresher keeps folding
/// attention for new commits. Their windows reread at least every clock period while they are
/// open, so this passes only when none is. A window that opens later gets the newest list at
/// once and wakes the refresher.
const ATTENTION_LIST_IDLE: Duration = Duration::from_secs(600);

/// Keep attention, every person's glasses and arrangements, and the summary rows windows read
/// published off the request path. After the initial publication, demand-gated source changes,
/// roster changes, prompt evidence and exact eligibility deadlines drive maintenance.
/// Only changed sources and the rows whose recorded fleet dependencies changed are rebuilt.
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
    let mut presented_roster = state.store.subscribe_agents_publication_changes();
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
            let reader = store.clone();
            let summary_state = state.clone();
            // Attention folds only while windows read it (they reread it every clock period),
            // apart from the first fold as the daemon starts. Once views are forgotten (a
            // checkpoint, each chunk of a long catch-up) a node nobody reads does not fold it
            // again; the next window to read it wakes this task.
            let attention = store.attention_list_read_within(idle) || store.attention_list_starting();
            // A prompt card's conversation changed: present the prompts again.
            let prompts_changed = watching(&prompts).take_changed();
            let changed_prompt_seats = prompts_changed.then(|| watching(&prompts).take_changed_seats()).flatten();
            let watch = Arc::clone(&prompts);
            // One view at a time: attention, every person's glasses and arrangements, then the
            // summary rows windows read. One view's failure leaves the others refreshing. A
            // summary selection a window waits for is computed before each view, so the window
            // waits for at most one other view's refresh.
            let refreshed = tokio::task::spawn_blocking(move || crate::task_cpu::blocking("publication.attention", || {
                // A pairing completed or revoked since the last pass changes which windows a
                // session may hold; their windows reread to recheck it, as before.
                if let Err(error) = reader.recheck_pairings() {
                    eprintln!("fractal: pairing recheck for published views failed: {error:#}");
                }
                let mut results = Vec::new();
                let waiting = |results: &mut Vec<_>, starting: bool| {
                    if let Some(result) = refresh_waiting_summary(&summary_state, starting) {
                        results.push(("summary", result.map(|_| ())));
                    }
                };
                waiting(&mut results, false);
                results.push(("attention", crate::performance::task("attention/refresh", || {
                    if !attention || !(reader.attention_list_read_within(idle) || reader.attention_list_starting()) {
                        return Ok(());
                    }
                    // A prompt card reads its seat's conversation on this node, following it
                    // from before the read; one another member keeps, its follower read last.
                    let pending = |seat: &str| {
                        reader.note_attention_list_fold("card/prompt");
                        watching(&watch).forget_seat(seat);
                        super::prompt_card::seat_pending_calls(&summary_state, seat, |conversation| {
                            watching(&watch).follow_seat_conversation(seat, conversation)
                        })
                    };
                    watching(&watch).begin();
                    if refresh_attention_list_reading_seats(
                        &reader,
                        |previous| attention_fleet(&reader, previous),
                        Some(&pending),
                        prompts_changed,
                        changed_prompt_seats.as_ref(),
                    )? {
                        if let Some(publication) = reader.newest_attention_list().1 {
                            for row in publication.rows.iter().filter(|row| row["attention_kind"] == "harness-prompt") {
                                if let Some(seat) = row["source_id"].as_str() {
                                    watching(&watch).retain_seat(seat);
                                }
                            }
                        }
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
            let mut attention_clock = false;
            let mut summary_clock = false;
            match refreshed {
                Ok(results) => {
                    if prompts_changed
                        && results.iter().any(|(view, result)| *view == "attention" && result.is_err())
                    {
                        watching(&prompts).keep_changed();
                    }
                    for (view, result) in results {
                        match view {
                            "attention" => attention_clock = result.is_ok(),
                            "summary" => summary_clock = result.is_ok(),
                            _ => {}
                        }
                        if let Err(error) = &result {
                            eprintln!("fractal: {view} view refresh failed: {error:#}");
                        }
                        store.note_view_refreshed(view, result.is_ok());
                    }
                }
                Err(error) => {
                    // The refresh panicked: what woke it for the prompts is still unread.
                    if prompts_changed {
                        watching(&prompts).keep_changed();
                    }
                    eprintln!("fractal: published view refresh stopped: {error}");
                    for view in crate::store::attention_list::PUBLISHED_VIEWS {
                        store.note_view_refreshed(view, false);
                    }
                }
            }
            // A clock wake represents an actual eligibility transition, never a periodic
            // re-evaluation. Without demand even those transitions wait for the next reader.
            let due = store.attention_list_read_within(idle).then(|| {
                [
                    attention_clock.then(|| store.attention_list_due_at()).flatten(),
                    summary_clock.then(|| store.summary_valid_until(idle)).flatten(),
                ]
                    .into_iter()
                    .flatten()
                    .min()
            }).flatten();
            tokio::select! {
                () = wake.notified() => {}
                result = changed.changed() => {
                    if result.is_err() {
                        return;
                    }
                }
                result = roster.changed() => {
                    if result.is_err() {
                        return;
                    }
                }
                result = presented_roster.changed() => {
                    if result.is_err() {
                        return;
                    }
                }
                () = async {
                    match due {
                        Some(due) => tokio::time::sleep(Duration::from_millis(
                            due.saturating_sub(client_now_ms()).min(u64::MAX as u128) as u64,
                        )).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {}
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
    refresh_attention_list_reading_seats(store, fleet, pending, prompts_changed, None)
}

fn refresh_attention_list_reading_seats(
    store: &Store,
    fleet: impl FnOnce(Option<&Arc<AttentionFleet>>) -> Option<Arc<AttentionFleet>>,
    pending: Option<super::prompt_card::PendingCalls>,
    prompts_changed: bool,
    changed_prompt_seats: Option<&BTreeSet<String>>,
) -> anyhow::Result<bool> {
    let prompts_changed = prompts_changed && pending.is_some();
    let (generation, previous) = store.newest_attention_list();
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
        let mut subjects = Some(BTreeSet::new());
        let mut kinds = BTreeSet::new();
        let mut refresh_sources = previous.as_ref().is_none_or(|previous| {
            previous.source_snapshot.is_none()
                || now < previous.evaluated_at_unix_ms
                || previous.due_at_unix_ms.is_some_and(|due| now >= due)
        });
        match &previous {
            None => {
                store.note_attention_list_fold("first");
                subjects = None;
            }
            Some(previous) if now < previous.evaluated_at_unix_ms => {
                store.note_attention_list_fold("clock reversed");
                subjects = None;
            }
            Some(previous) => match store.attention_list_delta(previous, index)? {
                AttentionDelta::Unchanged => {
                    if previous.due_at_unix_ms.is_some_and(|due| now >= due) {
                        store.note_attention_list_fold("due");
                    }
                }
                AttentionDelta::Subjects { subjects: changed, kinds: changed_kinds } => {
                    store.note_attention_list_fold(changed_kinds.iter().next().map(String::as_str).unwrap_or("subjects"));
                    subjects = Some(changed);
                    kinds = changed_kinds;
                    refresh_sources = true;
                }
                AttentionDelta::Refold(cause) => {
                    store.note_attention_list_fold(&cause);
                    subjects = None;
                    refresh_sources = true;
                }
            },
        }
        if !refresh_sources && !fleet_changed && !prompts_changed
            && previous.as_ref().is_some_and(|previous| previous.cut == index) {
            return Ok(false);
        }
        let projection = store.attention_projection_frontier()?;
        let ask_deadline_policy =
            crate::store::person_work::ask_deadline_policy(&store.readers.get())?;
        let mut presentation_dependencies = BTreeMap::new();
        let source_snapshot = if refresh_sources {
            let snapshot = store.attention_source_snapshot(
                previous.as_ref().and_then(|previous| previous.source_snapshot.as_deref()),
                subjects.as_ref(),
                &kinds,
                now,
            )?;
            for _ in 0..snapshot.refolded_subjects {
                store.note_attention_list_fold("source/subject");
            }
            for _ in 0..snapshot.refolded_groups {
                store.note_attention_list_fold("source/group");
            }
            Some(Arc::new(snapshot))
        } else {
            previous.as_ref().and_then(|previous| previous.source_snapshot.clone())
        };
        let presented;
        let rows = if refresh_sources || fleet_changed || prompts_changed {
            let mut rows = if refresh_sources {
                attention_resource_rows(store, source_snapshot.as_deref().unwrap(), previous.as_deref(), subjects.as_ref())?
            } else {
                (*previous.as_ref().unwrap().rows).clone()
            };
            present_changed_rows(store, &mut rows, fleet.as_deref(), previous.as_deref(), subjects.as_ref(), &mut presentation_dependencies)?;
            let unchanged = BTreeSet::new();
            let dirty = if prompts_changed { changed_prompt_seats } else { Some(&unchanged) };
            presented = present_prompts_for_seats(&mut rows, pending, dirty);
            match &previous {
                Some(previous) if *previous.rows == rows => Arc::clone(&previous.rows),
                _ => Arc::new(rows),
            }
        } else {
            presented = false;
            presentation_dependencies = previous.as_ref().unwrap().presentation_dependencies.clone();
            Arc::clone(&previous.as_ref().unwrap().rows)
        };
        let evaluated_at_unix_ms = now;
        let due_at_unix_ms = source_snapshot.as_ref()
            .and_then(|snapshot| snapshot.valid_until)
            .filter(|due| *due > now);
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
                published_at_unix_ms,
                rows,
                fleet,
                owners,
                ask_deadline_policy,
                source_snapshot,
                presentation_dependencies,
            },
            changed,
        );
        // Only active readers define a publication-latency sample; an idle view deliberately
        // waits for demand instead of keeping its cut warm.
        if let Some(previous) = previous.as_ref().filter(|_| store.attention_list_read_within(
            ATTENTION_LIST_IDLE.as_millis() as u64,
        )) {
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

/// Convert only changed source items. Unchanged items retain both resource enrichment and
/// presentation; controls on a changed subject still pass through the normal overlay.
fn attention_resource_rows(
    store: &Store,
    snapshot: &fractal_store::store::attention_snapshot::AttentionSourceSnapshot,
    previous: Option<&AttentionPublication>,
    changed: Option<&BTreeSet<String>>,
) -> anyhow::Result<Vec<Value>> {
    let old_items: HashMap<_, _> = previous
        .and_then(|previous| previous.source_snapshot.as_ref())
        .into_iter()
        .flat_map(|snapshot| snapshot.items.iter())
        .map(|item| ((item.subject.as_str(), item.person.as_str(), item.episode.as_str()), item))
        .collect();
    let old_rows: HashMap<_, _> = previous.into_iter()
        .flat_map(|previous| previous.rows.iter())
        .filter_map(|row| row["id"].as_str().map(|id| (id, row)))
        .collect();
    let mut rows = Vec::with_capacity(snapshot.items.len());
    for item in &snapshot.items {
        let key = (item.subject.as_str(), item.person.as_str(), item.episode.as_str());
        let id = client_attention_id(&item.subject, &item.person, &item.episode)?;
        let reusable = changed.is_some_and(|changed| !changed.contains(&item.subject))
            && old_items.get(&key).is_some_and(|before| *before == item);
        if reusable && let Some(row) = old_rows.get(id.as_str()) {
            rows.push((*row).clone());
        } else {
            store.note_attention_list_fold("card/resource");
            rows.extend(client_attention_resources_from_items(store, vec![item.clone()])?);
        }
    }
    Ok(rows)
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
    if let Some(previous) = previous.filter(|previous| previous.observed_revision() == revision) {
        return Some(Arc::clone(previous));
    }
    let Some((_, cards, _)) = store.published_agent_roster(u64::MAX, false) else {
        return previous.cloned();
    };
    let (fleet, changed) = match previous {
        Some(previous) => AttentionFleet::updated(previous, revision, &cards),
        None => (Arc::new(AttentionFleet::from_cards(revision, &cards)), cards.len()),
    };
    for _ in 0..changed {
        store.note_attention_list_fold("fleet/agent");
    }
    Some(fleet)
}

/// Reuse a row's complete presentation unless its source or an exact fleet lookup changed.
/// Rich launch/revision previews are built only for affected cards.
fn present_changed_rows(
    store: &Store,
    rows: &mut [Value],
    fleet: Option<&AttentionFleet>,
    previous: Option<&AttentionPublication>,
    dirty_subjects: Option<&BTreeSet<String>>,
    dependencies: &mut BTreeMap<String, AttentionPresentationDependencies>,
) -> anyhow::Result<()> {
    let empty = AttentionFleet::default();
    let before_fleet = previous.and_then(|previous| previous.fleet.as_deref()).unwrap_or(&empty);
    let after_fleet = fleet.unwrap_or(&empty);
    let previous_rows: HashMap<_, _> = previous.into_iter()
        .flat_map(|previous| previous.rows.iter())
        .filter_map(|row| row["id"].as_str().map(|id| (id, row)))
        .collect();
    for row in rows {
        let reused = row["id"].as_str().and_then(|id| {
            let dirty = dirty_subjects.is_none_or(|dirty| {
                ["source_id", "launch_id", "variant_id", "mission_id", "mission_run_id", "step_run_id"]
                    .iter()
                    .filter_map(|field| row[*field].as_str())
                    .any(|subject| dirty.contains(subject))
            });
            if dirty {
                return None;
            }
            let before = previous_rows.get(id)?;
            let inputs = previous?.presentation_dependencies.get(id)?;
            let same_source = row.as_object()?.iter()
                .all(|(key, value)| key == "presentation" || before.get(key) == Some(value))
                && before.as_object()?.keys()
                    .all(|key| key == "presentation" || row.get(key).is_some());
            (same_source && !inputs.changed(before_fleet, after_fleet))
                .then(|| ((*before)["presentation"].clone(), inputs.clone()))
        });
        if let Some((presentation, inputs)) = reused {
            let id = row["id"].as_str().unwrap().to_owned();
            row["presentation"] = presentation;
            dependencies.insert(id, inputs);
        } else {
            present_rows(store, std::slice::from_mut(row), fleet, dependencies)?;
            // A roster label change does not invalidate transcript evidence for the same call.
            if let Some(before) = row["id"].as_str().and_then(|id| previous_rows.get(id))
                && row["attention_kind"] == "harness-prompt"
                && row["source_id"] == before["source_id"]
                && row["action_parameters"] == before["action_parameters"]
                && let Some(prompt) = before["presentation"].get("prompt") {
                row["presentation"]["prompt"] = prompt.clone();
            }
        }
    }
    Ok(())
}

/// What the attention presentation reads beside a row: the fleet, and the paths of the steps
/// the rows hold.
struct Known<'a> {
    fleet: &'a AttentionFleet,
    paths: &'a HashMap<String, String>,
    dependencies: RefCell<AttentionPresentationDependencies>,
}

impl fractal_ui_model::attention::Fleet for Known<'_> {
    fn agent_label(&self, agent: &str) -> Option<String> {
        self.dependencies.borrow_mut().labels.insert(agent.to_owned());
        self.fleet.agent_label(agent)
    }
    fn claimant(&self, step: &str) -> Option<String> {
        self.dependencies.borrow_mut().claimants.insert(step.to_owned());
        self.fleet.claimant(step)
    }
    fn working_in(&self, mission: &str) -> Option<String> {
        self.dependencies.borrow_mut().working.insert(mission.to_owned());
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
    present_rows(store, rows, fleet, &mut BTreeMap::new())
}

/// Give each prompt card's presentation its prompt: the call it asks about, while `pending` reads
/// it without a result in its seat's conversation. Returns whether it read any: without
/// `pending` a card carries no prompt.
#[cfg(test)]
fn present_prompts(rows: &mut [Value], pending: Option<super::prompt_card::PendingCalls>) -> bool {
    present_prompts_for_seats(rows, pending, None)
}

fn present_prompts_for_seats(
    rows: &mut [Value],
    pending: Option<super::prompt_card::PendingCalls>,
    dirty: Option<&BTreeSet<String>>,
) -> bool {
    let Some(pending) = pending else {
        return false;
    };
    for row in rows
        .iter_mut()
        .filter(|row| row["attention_kind"] == "harness-prompt" && row["presentation"].is_object())
    {
        let changed = dirty.is_none_or(|dirty| row["source_id"].as_str().is_some_and(|seat| dirty.contains(seat)));
        if !changed && row["presentation"].get("prompt").is_some() {
            continue;
        }
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
    dependencies: &mut BTreeMap<String, AttentionPresentationDependencies>,
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
                eprintln!("fractal: WARN attention cannot read the path of {step}: {error:#}")
            }
        }
    }
    let known = Known {
        fleet: fleet.unwrap_or(&none),
        paths: &paths,
        dependencies: RefCell::default(),
    };
    for row in rows {
        store.note_attention_list_fold("card/present");
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
                            "fractal: WARN attention {} names no revision changes: {error:#}",
                            row["id"]
                        ),
                    }
                }
                // A launch card previews its newest variant's mission, or says why it cannot.
                if facts.attention_kind == "launch-approval" {
                    match super::launch_card::launch_card(store, row, |agent| {
                        fractal_ui_model::attention::Fleet::agent_label(&known, agent)
                    }) {
                        Ok(Some(launch)) => presentation["launch"] = launch,
                        Ok(None) => {}
                        Err(error) => eprintln!(
                            "fractal: WARN attention {} has no launch preview: {error:#}",
                            row["id"]
                        ),
                    }
                }
                row["presentation"] = presentation;
                if let Some(id) = row["id"].as_str() {
                    dependencies.insert(id.to_owned(), known.dependencies.take());
                }
            }
            Err(error) => eprintln!(
                "fractal: WARN attention {} has no presentation: {error}",
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
    fn one_agent_change_presents_one_card_and_unrelated_commit_presents_none() {
        let store = Store::open_memory("node").unwrap();
        store.start_attention_list_refresher().unwrap();
        refresh_attention_list(&store).unwrap();
        let (generation, initial) = store.newest_attention_list();
        let initial = initial.unwrap();
        let cards = vec![
            json!({"id":"agent/example/oak","name":"Oak"}),
            json!({"id":"agent/example/elm","name":"Elm"}),
        ];
        let before = Arc::new(AttentionFleet::from_cards(1, &cards));
        let mut rows = vec![
            json!({"id":"attention/oak","attention_kind":"agent-request","source_id":"ask/oak","requester_id":"agent/example/oak"}),
            json!({"id":"attention/elm","attention_kind":"agent-request","source_id":"ask/elm","requester_id":"agent/example/elm"}),
        ];
        let mut dependencies = BTreeMap::new();
        present_changed_rows(&store, &mut rows, Some(&before), None, None, &mut dependencies).unwrap();
        store.publish_attention_list(generation, AttentionPublication {
            cut: initial.cut,
            projection: initial.projection.clone(),
            evaluated_at_unix_ms: initial.evaluated_at_unix_ms,
            due_at_unix_ms: None,
            published_at_unix_ms: initial.published_at_unix_ms,
            owners: AttentionPublication::index_owners(&rows),
            rows: Arc::new(rows),
            fleet: Some(before),
            source_snapshot: initial.source_snapshot.clone(),
            presentation_dependencies: dependencies,
            ask_deadline_policy: initial.ask_deadline_policy,
        }, true);
        let count = || store.attention_list_folds().get("card/present").copied().unwrap_or(0);
        let baseline = count();
        let mut changed = cards;
        changed[0]["name"] = json!("Oak changed");
        let after = Arc::new(AttentionFleet::from_cards(2, &changed));
        refresh_attention_list_with(&store, |_| Some(Arc::clone(&after))).unwrap();
        assert_eq!(count() - baseline, 1, "only the card reading the changed agent");
        let publication = store.newest_attention_list().1.unwrap();
        assert_eq!(publication.rows[0]["presentation"]["raised_by"]["label"], "Oak changed");
        assert_eq!(publication.rows[1]["presentation"]["raised_by"]["label"], "Elm");
        store.append_claim(&ClaimInput {
            subject: "step-run/unrelated/step".into(),
            kind: "work.renewed".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        let baseline = count();
        refresh_attention_list_with(&store, |_| Some(after)).unwrap();
        assert_eq!(count(), baseline, "an unrelated commit rebuilds no card");
    }

    #[test]
    fn unread_backlog_preserves_complete_attention_and_still_finds_relevant_claims() {
        struct Clock;
        impl Drop for Clock {
            fn drop(&mut self) {
                smallclaims::store::set_thread_clock(None);
            }
        }
        let _clock = Clock;
        smallclaims::store::set_thread_clock(Some(client_now_ms()));
        let store = Store::open_memory("node").unwrap();
        store.start_attention_list_refresher().unwrap();
        refresh_attention_list(&store).unwrap();
        let first = store.newest_attention_list().1.unwrap();
        // More than the delta limit of permanently unread lease-renewal claims.
        // Do not force a fleet fold or hide the relevant claim after the backlog.
        for number in 0..10_001 {
            store.append_claim(&ClaimInput {
                subject: format!("step-run/noise-{number}/step"),
                kind: "work.renewed".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            }).unwrap();
        }
        let index = store.index().unwrap();
        assert_eq!(index - first.cut, 10_001, "exercise the raw-span guard boundary");
        assert_eq!(
            store.attention_list_delta(&first, index).unwrap(),
            AttentionDelta::Unchanged,
            "permanently unread traffic creates no card eligibility deadline",
        );
        refresh_attention_list(&store).unwrap();
        let published = store.newest_attention_list().1.unwrap();
        assert_eq!(published.cut, index);
        assert!(Arc::ptr_eq(&first.rows, &published.rows));

        store.append_claim(&ClaimInput {
            subject: "custom/test/relevant".into(),
            kind: "custom.test.changed".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        assert!(matches!(
            store.attention_list_delta(&first, store.index().unwrap()).unwrap(),
            AttentionDelta::Subjects { .. },
        ));
    }

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

        read.lock().unwrap().clear();
        let dirty = BTreeSet::from(["agent/example/oak".to_owned()]);
        assert!(present_prompts_for_seats(&mut rows, Some(&pending), Some(&dirty)));
        assert_eq!(*read.lock().unwrap(), ["agent/example/oak"], "one changed transcript reads one prompt");
        read.lock().unwrap().clear();
        assert!(present_prompts_for_seats(&mut rows, Some(&pending), Some(&BTreeSet::new())));
        assert!(read.lock().unwrap().is_empty(), "unchanged prompt evidence does no source reads");

        // Without a reader, as in a fold that is not the refresher's, a card carries no prompt.
        let mut rows = vec![json!({"attention_kind": "harness-prompt", "source_id": "agent/example/oak", "presentation": {}})];
        assert!(!present_prompts(&mut rows, None));
        assert!(rows[0]["presentation"].get("prompt").is_none());
    }

    /// A host hold placed by a run that has since ended is still a card on its owner's Now:
    /// the published list presents it as `stopped` · `request`, raised by its owner.
    #[test]
    fn a_published_host_hold_is_a_stopped_card_on_now() {
        use crate::store::host_holds::PlaceHostHold;
        let store = Store::open_memory("node").unwrap();
        store.start_attention_list_refresher().unwrap();
        let hold = store
            .place_host_hold(
                "person/operator",
                &PlaceHostHold {
                    actor: "person/operator".into(),
                    host: "node".into(),
                    reason: "investigate".into(),
                    operations: vec!["activation".into()],
                    originating_run: Some("mission-run/terminal".into()),
                    receipt: Some("receipt/activation".into()),
                },
            )
            .unwrap();
        refresh_attention_list(&store).unwrap();
        let (_, publication) = store.newest_attention_list();
        let publication = publication.unwrap();
        let row = publication
            .rows
            .iter()
            .find(|row| row["attention_kind"] == "host-hold")
            .expect("the hold is published");
        assert_eq!(row["source_id"], hold.id);
        let presentation = &row["presentation"];
        assert_eq!(presentation["tier"], "stopped");
        assert_eq!(presentation["kind"], "request");
        assert_eq!(presentation["raised_by"]["id"], "person/operator");
        assert!(presentation["agent"].is_null(), "chat reaches the chief of staff");
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
                published_at_unix_ms: first.published_at_unix_ms,
                owners: AttentionPublication::index_owners(&rows),
                rows: Arc::new(rows),
                fleet: first.fleet.clone(),
                ask_deadline_policy: first.ask_deadline_policy,
                source_snapshot: first.source_snapshot.clone(),
                presentation_dependencies: first.presentation_dependencies.clone(),
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
