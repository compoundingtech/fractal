//! The part of `store::evidence` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::SeatsStore;
use super::*;
pub use fractal_missions::store::evidence::*;

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
    /// Revalidate native connector custody under the graph writer before any mutation.
    fn append_harness_event_publication_checked(
            &self,
            publication: &crate::harness_events::Publication,
            validate: &(dyn Fn() -> Result<(), St3Error> + Send + Sync),
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
        let cache = self.fractal.agent_resources_cache.lock()
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
        self.append_harness_event_publication_checked(publication, &|| Ok(()))
    }

    fn append_harness_event_publication_checked(
            &self,
            publication: &crate::harness_events::Publication,
            validate: &(dyn Fn() -> Result<(), St3Error> + Send + Sync),
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
        fractal_schema::registry()
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
            append_latest_observation_publication_checked(
                &self.graph, &input, now_ms(), Some(&publication.runtime_incarnation), validate,
            )
        } else if input.kind == "harness.limits" {
            // Every seat on an account reports its limits; only a report that changes the
            // account's answer replicates.
            validate_claim_input(&input)?;
            append_limits_publication_checked(
                &self.graph, &input, now_ms(), Some(&publication.runtime_incarnation), validate,
            )
            .map(|(record, appended)| (record, appended, false))
        } else {
            append_claim_with_commit_context(
                &self.graph, &input, None, Some(&publication.runtime_incarnation), None, None,
                ClaimCommitContext { writer_validate: Some(validate), ..Default::default() },
            ).map(|(record, appended)| (record, appended, false))
        }
    }
}


#[cfg(test)]
mod checked_publication_tests {
    use super::*;
    use fractal_missions::store::harness_event_tests_shared_seats::{SEAT, event, runtime};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const KINDS: [&str; 6] = [
        "harness.observed", "harness.usage", "harness.limits", "harness.timeline",
        "harness.todo.observed", "harness.inventory.observed",
    ];

    fn publication(kind: &str) -> crate::harness_events::Publication {
        if !matches!(kind, "harness.todo.observed" | "harness.inventory.observed") {
            return event(kind);
        }
        let mut publication = event("harness.observed");
        publication.claim.kind = kind.into();
        let fields = if kind == "harness.todo.observed" {
            json!({
                "harness":"omp", "session_id":"native-a", "incarnation_id":"runtime-a",
                "observed_at":"2026-10-29T00:00:00Z", "source_op":"clear",
                "phases":[], "totals":{"pending":0,"in_progress":0,"completed":0,"blocked":0},
                "truncated":false
            })
        } else {
            json!({
                "harness":"omp", "session_id":"native-a", "incarnation_id":"runtime-a",
                "observed_at":"2026-10-29T00:00:00Z",
                "dynamic_commands":"supported", "commands":[]
            })
        };
        publication.claim.fields = serde_json::from_value(fields).unwrap();
        publication
    }

    fn log_sizes(store: &Store) -> (u64, u64) {
        store.readers.get().query_row(
            "SELECT (SELECT COUNT(*) FROM claims), (SELECT COUNT(*) FROM local_observations)",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap()
    }

    #[test]
    fn custody_refusal_leaves_every_native_event_retention_unchanged() {
        for kind in KINDS {
            let store = Store::open_memory("amber").unwrap();
            runtime(&store, "runtime-a", "running");
            let before_index = store.index().unwrap();
            let before_logs = log_sizes(&store);
            let calls = AtomicUsize::new(0);
            let caller = std::thread::current().id();
            let validate = || {
                assert_ne!(std::thread::current().id(), caller, "{kind}");
                calls.fetch_add(1, Ordering::SeqCst);
                Err(St3Error::new("native-custody-retired", "connector custody changed"))
            };
            let error = store.append_harness_event_publication_checked(
                &publication(kind), &validate,
            ).unwrap_err();
            assert_eq!(error.code, "native-custody-retired", "{kind}");
            assert_eq!(calls.load(Ordering::SeqCst), 1, "{kind}");
            assert_eq!(store.index().unwrap(), before_index, "{kind}");
            assert_eq!(log_sizes(&store), before_logs, "{kind}");
            assert!(store.latest_observation(SEAT, kind).unwrap().is_none(), "{kind}");
        }
    }

    #[test]
    fn checked_native_events_revalidate_in_writer_even_on_replay() {
        for kind in KINDS {
            let store = Store::open_memory("amber").unwrap();
            runtime(&store, "runtime-a", "running");
            let input = publication(kind);
            let calls = AtomicUsize::new(0);
            let caller = std::thread::current().id();
            let validate = || {
                assert_ne!(std::thread::current().id(), caller, "{kind}");
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            };
            let (_, appended, _) = store.append_harness_event_publication_checked(
                &input, &validate,
            ).unwrap();
            assert!(appended, "{kind}");
            let before_logs = log_sizes(&store);
            let before_index = store.index().unwrap();
            let error = store.append_harness_event_publication_checked(&input, &|| {
                assert_ne!(std::thread::current().id(), caller, "{kind}");
                calls.fetch_add(1, Ordering::SeqCst);
                Err(St3Error::new("native-custody-retired", "connector custody changed"))
            }).unwrap_err();
            assert_eq!(error.code, "native-custody-retired", "{kind}");
            let (_, appended, _) = store.append_harness_event_publication_checked(
                &input, &validate,
            ).unwrap();
            assert!(!appended, "{kind}");
            assert_eq!(calls.load(Ordering::SeqCst), 3, "{kind}");
            assert_eq!(log_sizes(&store), before_logs, "{kind}");
            assert_eq!(store.index().unwrap(), before_index, "{kind}");
            runtime(&store, "runtime-b", "running");
            let error = store.append_harness_event_publication_checked(
                &input, &validate,
            ).unwrap_err();
            assert_eq!(error.code, "stale-harness-event-session", "{kind}");
            assert_eq!(calls.load(Ordering::SeqCst), 4, "{kind}");
        }
    }
}
