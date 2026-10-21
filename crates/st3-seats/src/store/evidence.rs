//! The part of `store::evidence` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::SeatsStore;
use super::*;
pub use st3_missions::store::evidence::*;

pub trait EvidenceSeatsStore {
    /// [`Self::agent_resources_cached_at`], with when the rows were published.
    fn agent_resources_published_at(
            &self,
            index: u64,
            history: bool,
            selected: Option<&BTreeSet<String>>,
        ) -> Result<Option<(Arc<Vec<Value>>, u128)>>;
    /// Admission's transition decision, from the same transaction as its native runtime fence.
    /// The echoed producer record stays unchanged; it need not carry derived history metadata.
    fn append_harness_event_publication(
            &self,
            publication: &crate::harness_events::Publication,
        ) -> Result<(ClaimRecord, bool, bool), St3Error>;
}

impl EvidenceSeatsStore for Store {

    fn agent_resources_published_at(
            &self,
            index: u64,
            history: bool,
            selected: Option<&BTreeSet<String>>,
        ) -> Result<Option<(Arc<Vec<Value>>, u128)>> {
        let fault_retries = self.agent_roster_fault_retries(history);
        if selected.map_or(!fault_retries.is_empty(), |names| !names.is_disjoint(&fault_retries)) {
            return Ok(None);
        }
        if index < current_index(&self.readers.get())? {
            return Ok(None);
        }
        let now = now_ms();
        let local = crate::performance::task("roster/frontier-read", || {
            roster_local_frontier(&self.readers.get(), index)
        })?;
        let cache = self.smalltalk.agent_resources_cache.lock()
            .expect("agent resources cache poisoned");
        let hit = cache.iter().rev()
            .find(|entry| entry.index == index && entry.local == local && entry.history == history)
            .filter(|entry| agent_resources_entry_hits(entry, now, index, local, history, selected))
            .map(|entry| (Arc::clone(&entry.items), entry.published_at_unix_ms));
        drop(cache);
        Ok(hit.map(|hit| crate::performance::task("roster/cache-hit", || {
            crate::otel::record_roster("hit", hit.0.len());
            hit
        })))
    }

    fn append_harness_event_publication(
            &self,
            publication: &crate::harness_events::Publication,
        ) -> Result<(ClaimRecord, bool, bool), St3Error> {
        let mut input = publication.claim.clone();
        if publication.sequence == 0
            || publication.runtime_incarnation.is_empty()
            || !matches!(
                input.kind.as_str(),
                "harness.observed" | "harness.usage" | "harness.limits" | "harness.timeline"
                    | "harness.todo.observed" | "harness.inventory.observed"
            )
            || input.actor.as_deref() != Some(input.subject.as_str())
            || input.idempotency_key.is_none()
        {
            return Err(St3Error::new(
                "invalid-harness-event",
                "invalid native observation envelope",
            ));
        }
        st3_schema::registry()
            .validate_public_claim(
                &input.subject,
                &input.kind,
                &input.fields,
                input.actor.as_deref(),
            )
            .map_err(|error| St3Error::new(error.code, error.message))?;
        // A context event can yield two independently acknowledged usage semantics. Its slot
        // is stable even if a caller mistakenly changes its body on retry.
        let source_runtime = input
            .fields
            .get("incarnation_id")
            .and_then(Value::as_str)
            .filter(|runtime| !runtime.is_empty())
            .ok_or_else(|| {
                St3Error::new(
                    "invalid-harness-event",
                    "event has no source runtime provenance",
                )
            })?;
        let slot = input
            .fields
            .get("semantics")
            .and_then(Value::as_str)
            .unwrap_or("");
        input.idempotency_key = Some(format!(
            "harness-event:{}:{}:{}:{}:{}",
            input.subject, source_runtime, publication.sequence, input.kind, slot
        ));
        if input.kind == "harness.observed" {
            validate_claim_input(&input)?;
            append_latest_observation_publication(
                &self.graph, &input, now_ms(), Some(&publication.runtime_incarnation),
            )
        } else {
            append_claim_with_fences(
                &self.graph, &input, None, Some(&publication.runtime_incarnation),
            ).map(|(record, appended)| (record, appended, false))
        }
    }
}

