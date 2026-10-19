//! The part of `store::conversation_reads` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_missions::store::conversation_reads::*;
pub use st3_missions::store::conversation_reads::{Owner, Owners};

#[cfg(test)]
mod tests {
    use super::*;
    use st3_missions::store::missions::MissionsStore;
    use st3_missions::store::conversation_reads_shared_seats::*;
    use st3_missions::store::conversation_reads_shared_seats::{BEFORE_MESSAGE_DECODE, BEFORE_OWNER_PUBLISH};
    use st3_missions::store::conversation_reads_tests_shared_seats::*;
    use st3_missions::store::conversation_reads_tests_shared_seats::{append};

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
    fn assistant_owner_pages_match_historical_fold_without_building_whole_map() {
        let store = Store::open_memory("node").unwrap();
        for number in 0..32 {
            append(&store, &format!("agent/{number:03}"), "runtime.observed",
                json!({"runtime_id":"pty","incarnation_id":"original"}));
        }
        let through = store.index().unwrap();
        append(&store, "agent/000", "runtime.observed",
            json!({"runtime_id":null,"incarnation_id":"new"}));
        append(&store, "agent/future", "runtime.observed", json!({"runtime_id":"future"}));
        let expected = oracle(&store, through);
        let mut actual = Owners::new();
        let mut after = None;
        loop {
            let page = store.assistant_conversation_owners_page(after.as_deref(), 3, through).unwrap();
            assert!(page.len() <= 4);
            let more = page.len() > 3;
            for (id, owner) in page.into_iter().take(3) {
                assert!(after.as_ref().is_none_or(|after| after < &id));
                after = Some(id.clone());
                actual.insert(id, owner);
            }
            if !more { break; }
        }
        assert_eq!(actual, expected);
        assert_eq!(store.assistant_conversation_owners_page(None, 0, through).unwrap().len(), 1);
        assert!(store.assistant_conversation_owners_page(Some("agent/zzz"), 3, through).unwrap().is_empty());
        assert!(store.assistant_conversation_owners_page(None, 3, store.index().unwrap() + 1).is_err());
        assert!(store.smalltalk.conversation_owners.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    }

    #[test]
    fn assistant_owner_id_selection_is_bounded_before_identity_projection() {
        let store = Store::open_memory("node").unwrap();
        let mut costs = Vec::new();
        store.connection.batched(|tx| -> Result<()> {
            for number in 0..1000 {
                append_claim_tx(tx, "node", &format!("agent/{number:05}"), "runtime.observed",
                    None, &json!({"fields":{"runtime_id":"pty"}}), &[], None)?;
                if number == 9 || number == 999 {
                    let mut query = tx.prepare_cached(ASSISTANT_OWNER_IDS_PAGE)?;
                    query.reset_status(rusqlite::StatementStatus::VmStep);
                    let ids = query.query_map(params![i64::MAX, Option::<&str>::None, 4],
                        |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    assert_eq!(ids.len(), 4);
                    costs.push(query.get_status(rusqlite::StatementStatus::VmStep));
                }
            }
            Ok(())
        }).unwrap().unwrap();
        assert!(costs[0] > 0 && costs[1] <= costs[0] * 2,
            "bounded seeks must not scan every owner: {costs:?}");
    }

    #[test]
    fn assistant_work_candidates_are_keyset_limited_before_hydration() {
        let store = Store::open_memory("node").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute_batch(
                "INSERT INTO mission_runs VALUES
                 ('run','mission','revision','current','revision','run',NULL,'workspace',
                  'person/test','{}','automatic','running','running','1','1');
                 INSERT INTO run_generations VALUES
                 ('current','run','revision',NULL,'running','person/test','test','1','1'),
                 ('old','run','revision',NULL,'cancelled','person/test','test','1','1');")?;
            for number in 0..1000 {
                let status = match number {
                    0 => "pending", 1 => "completed", 2 => "failed", 3 => "cancelled",
                    _ => "ready",
                };
                tx.execute(
                    "INSERT INTO step_runs
                     (subject,run_id,generation_id,step_path,definition_hash,status,attempt,
                      agentless,goals,created_at_unix_ms,updated_at_unix_ms)
                     VALUES(?1,'run',?2,?1,'not-hydrated',?3,0,?4,'[]','1','1')",
                    params![format!("step-run/{number:04}"),
                        if number == 4 { "old" } else { "current" }, status, number % 2])?;
            }
            Ok(())
        }).unwrap().unwrap();
        let snapshot = store.index().unwrap();
        assert_eq!(store.assistant_work_candidate_ids_page(None, 2, 1, snapshot).unwrap(),
            ["step-run/0005", "step-run/0006", "step-run/0007"]);
        assert_eq!(store.assistant_work_candidate_ids_page(Some("step-run/0006"), 2, 1, snapshot).unwrap(),
            ["step-run/0007", "step-run/0008", "step-run/0009"]);
        assert_eq!(store.assistant_work_candidate_ids_page(None, 0, 1, snapshot).unwrap(),
            ["step-run/0005"]);
        assert!(store.assistant_work_candidate_ids_page(Some("step-run/0999"), 2, 1, snapshot).unwrap().is_empty());
        append(&store, "agent/new", "runtime.observed", json!({"runtime_id":"pty"}));
        assert!(store.assistant_work_candidate_ids_page(None, 2, 1, snapshot).is_err());
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
