//! The part of `store::conversation_reads` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_missions::store::conversation_reads::*;
pub use fractal_missions::store::conversation_reads::{Owner, Owners};

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_missions::store::conversation_reads_shared_seats::*;
    use fractal_missions::store::conversation_reads_shared_seats::{BEFORE_MESSAGE_DECODE, BEFORE_OWNER_PUBLISH};
    use fractal_missions::store::conversation_reads_tests_shared_seats::*;
    use fractal_missions::store::conversation_reads_tests_shared_seats::{append};

    fn oracle(store: &Store, at: u64) -> Owners {
        store
            .status_for_subject_prefix_at("agent/", Some(at), true, crate::store::ClaimsReduction::Full)
            .unwrap()
            .subjects
            .into_iter()
            .map(|subject| {
                let fields = subject
                    .actual
                    .as_ref()
                    .map(|v| v.get("fields").unwrap_or(v));
                let incarnation = fields
                    .and_then(|v| v["incarnation_id"].as_str())
                    .or(subject.projection.runtime_incarnation.as_deref())
                    .map(str::to_owned);
                let runtime = fields
                    .and_then(|v| v["runtime_id"].as_str())
                    .map(str::to_owned);
                (
                    subject.subject.clone(),
                    Owner {
                        subject: subject.subject,
                        incarnation,
                        runtime,
                        origin: subject.actual_origin,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn conversation_owner_frontiers_match_full_status_and_refresh_only_changed_agents() {
        let store = Store::open_memory("node").unwrap();
        for agent in ["agent/alder", "agent/birch"] {
            append(
                &store,
                agent,
                "runtime.observed",
                json!({"status":"running","incarnation_id":"first","runtime_id":"pty"}),
            );
        }
        let first = store.index().unwrap();
        let owners = store.conversation_owners_at(first).unwrap();
        assert_eq!(*owners, oracle(&store, first));
        append(
            &store,
            "message/unrelated",
            "message.sent",
            json!({"from":"person/receiver","to":"agent/other","content":"Unrelated", "status":"sent"}),
        );
        let unrelated = store.index().unwrap();
        let before = STATEMENTS_RUN.with(std::cell::Cell::get);
        let unchanged = store.conversation_owners_at(unrelated).unwrap();
        let queries = STATEMENTS_RUN.with(std::cell::Cell::get) - before;
        assert_eq!(*unchanged, *owners);
        assert!(
            queries <= 3,
            "an unrelated write must not fold agents: {queries} queries"
        );
        append(
            &store,
            "agent/alder",
            "runtime.observed",
            json!({"status":"idle"}),
        );
        let inherited = store.index().unwrap();
        assert_eq!(
            *store.conversation_owners_at(inherited).unwrap(),
            oracle(&store, inherited)
        );
        append(
            &store,
            "agent/alder",
            "runtime.observed",
            json!({"status":"running","incarnation_id":null,"runtime_id":"second"}),
        );
        let nulled = store.index().unwrap();
        assert_eq!(
            *store.conversation_owners_at(nulled).unwrap(),
            oracle(&store, nulled)
        );
        assert_eq!(
            *store.conversation_owners_at(first).unwrap(),
            oracle(&store, first)
        );
        // A historical rebuild must exclude later fields even after eviction.
        store.forget_current_views();
        assert_eq!(
            *store.conversation_owners_at(first).unwrap(),
            oracle(&store, first)
        );
        assert_eq!(
            *store.conversation_owners_at(nulled).unwrap(),
            oracle(&store, nulled)
        );
    }

    #[test]
    fn conversation_owner_new_declarations_and_harness_only_agents_match_cold_reads() {
        let store = Store::open_memory("node").unwrap();
        append(
            &store,
            "agent/alder",
            "runtime.observed",
            json!({"status":"running","incarnation_id":"first","runtime_id":"pty"}),
        );
        store
            .conversation_owners_at(store.index().unwrap())
            .unwrap();
        store
            .connection
            .batched(|tx| -> Result<()> {
                append_claim_tx(
                    tx,
                    "node",
                    "agent/birch",
                    "intent.desired",
                    None,
                    &json!({"subject":"agent/birch","kind":"agent","desired":{}}),
                    &[],
                    None,
                )?;
                append_claim_tx(
                    tx,
                    "node",
                    "agent/cedar",
                    "harness.observed",
                    None,
                    &json!({"fields":{"state":"idle"}}),
                    &[],
                    None,
                )?;
                Ok(())
            })
            .unwrap()
            .unwrap();
        let frontier = store.index().unwrap();
        let warm = store.conversation_owners_at(frontier).unwrap();
        assert!(warm.contains_key("agent/birch"));
        assert!(warm.contains_key("agent/cedar"));
        assert_eq!(*warm, oracle(&store, frontier));
        store.forget_current_views();
        let cold = store.conversation_owners_at(frontier).unwrap();
        assert_eq!(*warm, *cold);
    }

    #[test]
    fn conversation_owner_build_before_repair_cannot_repopulate_the_cleared_cache() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        append(
            &store,
            "agent/alder",
            "runtime.observed",
            json!({"status":"running","incarnation_id":"first","runtime_id":"pty"}),
        );
        let frontier = store.index().unwrap();
        let (ready_send, ready) = std::sync::mpsc::channel();
        let (resume_send, resume) = std::sync::mpsc::channel();
        let reader = store.clone();
        let worker = std::thread::spawn(move || {
            BEFORE_OWNER_PUBLISH.with(|pause| {
                *pause.borrow_mut() = Some(Box::new(move || {
                    ready_send.send(()).unwrap();
                    resume
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                }));
            });
            reader.conversation_owners_at(frontier).unwrap()
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        store.forget_current_views();
        resume_send.send(()).unwrap();
        let result = worker.join().unwrap();
        assert!(
            store
                .smalltalk
                .conversation_owners
                .lock()
                .unwrap()
                .is_empty(),
            "an in-flight build republished a pre-repair cache entry"
        );
        assert_eq!(*result, oracle(&store, frontier));
        assert_eq!(*store.conversation_owners_at(frontier).unwrap(), *result);
        assert_eq!(store.smalltalk.conversation_owners.lock().unwrap().len(), 1);
    }

    #[test]
    fn conversation_owner_rivals_and_other_actual_kinds_keep_canonical_authority() {
        let older = Store::open_memory("older").unwrap();
        append(
            &older,
            "agent/alder",
            "runtime.observed",
            json!({"status":"running","incarnation_id":"first","runtime_id":"pty"}),
        );
        older.connection.batched(|tx| append_claim_tx(
            tx, "newer", "agent/alder", "runtime.observed", None,
            &json!({"fields":{"status":"running","incarnation_id":"rival","runtime_id":"other"}}), &[], None,
        )).unwrap().unwrap();
        let rival = older.index().unwrap();
        assert_eq!(
            *older.conversation_owners_at(rival).unwrap(),
            oracle(&older, rival)
        );
        append(
            &older,
            "agent/alder",
            "runtime.reconcile-decision",
            json!({"decision":"hold","reachability":"unreachable"}),
        );
        let other = older.index().unwrap();
        assert_eq!(
            *older.conversation_owners_at(other).unwrap(),
            oracle(&older, other)
        );
    }
}
