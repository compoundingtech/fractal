//! The part of `api::client_v0::summary` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AttentionListStore;
use crate::ClientSummaryStore;
use crate::PublishedViewsAttentionStore;
use crate::SummaryListStore;
use crate::MissionsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::store::summary_list::{SummaryInputs, SummaryMachines, SummaryShared};
use st3_ui_model::missions::Display;
pub use st3_api_core::api::client_v0::summary::*;
pub use st3_api_core::api::client_v0::summary::{active, machine_counts, published_attention_alerts, published_machines, refresh_published, working};
#[cfg(test)]
#[path = "summary/tests.rs"]
mod tests;

pub fn native(
    state: &AppState,
    session: &ClientSession,
    request: &CollectionSubscribe,
    snapshot: &ClientSnapshot,
    now: u128,
    windows: Option<&collection_windows::Windows>,
    commits: Option<u64>,
) -> anyhow::Result<Vec<Value>> {
    let person = person_filter(session, request.person.as_deref())
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let attention = state.store.attention_snapshot(person.as_deref(), now)?;
    // What needs the person is what waits on their answer: an update asks nothing.
    let alerts = attention.iter().filter(|item| item.is_alert()).count();
    let shared = shared(state, session, request, snapshot, now, windows, commits, None)?;
    Ok(vec![row(person.as_deref(), alerts, &shared)?])
}

/// The row of `person` from the newest shared counts and attention publication, with the
/// shared counts' cut and publication time: what the refresher publishes for that selection.
/// `None` until both are published. Memory only: a window that opens a selection is served at
/// once, before the refresher publishes it.
pub fn published_row(
    store: &Store,
    person: Option<&str>,
) -> Option<(u64, u128, Value)> {
    let (shared, _) = store.summary_shared()?;
    let attention = store.newest_attention_list().1?;
    let alerts = published_attention_alerts(&attention.rows, person);
    let row = row(person, alerts, &shared).ok()?;
    Some((shared.cut, shared.published_at_unix_ms, row))
}

pub fn retain_timestamp(items: &mut [Value], previous: &BTreeMap<String, Value>) {
    for item in items {
        if let Some(old) = item["id"].as_str().and_then(|id| previous.get(id))
            && old["revision"] == item["revision"]
            && old["person_id"] == item["person_id"]
        {
            item["updated_at"] = old["updated_at"].clone();
            if !old["agents_as_of"].is_null() {
                item["agents_as_of"] = old["agents_as_of"].clone();
            }
        }
    }
}
