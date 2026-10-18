//! The part of `store::operations` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::operations::*;

#[cfg(test)]
mod tests {
    use super::*;
    use st3_store::store::operations_tests_shared_seats::*;
    use st3_store::store::operations_tests_shared_seats::{clock_run};

    fn publish_overview_mission(store: &Store) {
        let source = "version 2\nmission \"example/overview\" state=\"ready\" { goal \"Build.\"; step \"build\" { goal \"Build.\"; }; }\n";
        let intent = crate::graph::parse_intent(source, store.origin()).unwrap();
        store.apply_internal(&intent, "overview-mission").unwrap();
    }

    #[test]
    fn mission_clock_fence_expires_leases_at_equality_without_signed_clock_truncation() {
        let store = Store::open_memory("fixture").unwrap();
        publish_overview_mission(&store);
        let run = clock_run(&store, "clock-run");
        let expiry = u128::from(u64::MAX) + 100;
        for status in ["claimed", "working", "verifying", "blocked"] {
            store.connection.lock().unwrap().execute(
                "UPDATE step_runs SET status=?2,lease_owner='agent/fixture/worker',
                     lease_incarnation='worker-one',lease_expires_at_unix_ms=?3,blocked_reason=NULL
                 WHERE subject=?1",
                params![run.steps[0].subject, status, expiry.to_string()],
            ).unwrap();
            assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], expiry - 1).unwrap(), Some(expiry));
            assert_eq!(store.mission_collection_valid_until(&[], expiry - 1).unwrap(), None);
            assert_eq!(store.mission_collection_valid_until(&["mission/example/unselected".into()], expiry - 1).unwrap(), None);
            assert_eq!(store.mission_collection_valid_until(&["example/overview".into()], expiry - 1).unwrap(), Some(expiry));
            let before = store.mission_step_preview_at(&run.subject, expiry - 1).unwrap().2;
            let at = store.mission_step_preview_at(&run.subject, expiry).unwrap().2;
            assert_eq!(before[0].status, status);
            assert_eq!(at[0].status, "ready");
            assert_eq!(at[0].claimant, None);
            assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], expiry).unwrap(), None);
            assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], expiry + 1).unwrap(), None);
        }
    }

    #[test]
    fn mission_clock_fence_ignores_graph_driven_deadlines_and_terminal_owners() {
        let store = Store::open_memory("fixture").unwrap();
        publish_overview_mission(&store);
        let run = clock_run(&store, "clock-run");
        {
            let connection = store.connection.lock().unwrap();
            connection.execute(
                "UPDATE step_runs SET status='pending',not_before_unix_ms='2000' WHERE subject=?1",
                [&run.steps[0].subject],
            ).unwrap();
            connection.execute(
                "INSERT INTO mission_run_deadlines(run_id,timeout_ms,deadline_at_unix_ms)
                 VALUES (?1,1000,'1500')",
                [&run.id],
            ).unwrap();
        }
        assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], 1000).unwrap(), None);
        assert_eq!(
            store.mission_step_preview_at(&run.subject, 2500).unwrap().2[0].status,
            "pending"
        );
        for terminal_owner in [
            "UPDATE mission_runs SET status='completed',phase='terminal'",
            "UPDATE mission_runs SET status='running',phase='normal';
             UPDATE run_generations SET status='superseded'",
        ] {
            let connection = store.connection.lock().unwrap();
            connection.execute_batch(terminal_owner).unwrap();
            connection.execute(
                "UPDATE step_runs SET status='claimed',lease_owner='agent/fixture/worker',
                 lease_expires_at_unix_ms='3000' WHERE subject=?1",
                [&run.steps[0].subject],
            ).unwrap();
            drop(connection);
            assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], 1000).unwrap(), None);
        }
    }

    #[test]
    fn mission_clock_fence_only_observes_three_run_headers_and_twenty_preview_steps() {
        let store = Store::open_memory("fixture").unwrap();
        let source = "version 2\nmission \"example/overview\" state=\"ready\" { concurrent-runs max=4; goal \"Build.\"; step \"build\" { goal \"Build.\"; }; }\n";
        let intent = crate::graph::parse_intent(source, store.origin()).unwrap();
        store.apply_internal(&intent, "preview-clock-mission").unwrap();
        let mut runs = Vec::new();
        for i in 0..4 {
            let run = clock_run(&store, &format!("clock-run-{i}"));
            store.connection.lock().unwrap().execute(
                "UPDATE mission_runs SET created_at_unix_ms=?2 WHERE id=?1",
                params![run.id, format!("100{i}")],
            ).unwrap();
            runs.push(run);
        }
        {
            let connection = store.connection.lock().unwrap();
            connection.execute(
                "UPDATE step_runs SET status='claimed',lease_expires_at_unix_ms='1500'
                 WHERE run_id=?1",
                [&runs[0].id],
            ).unwrap();
            // The newest run has twenty earlier rows before the leased twenty-first row.
            connection.execute(
                "UPDATE step_runs SET created_at_unix_ms='1' WHERE run_id=?1",
                [&runs[3].id],
            ).unwrap();
            for i in 1..=20 {
                connection.execute(
                    "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                         status,attempt,available_to,agentless,goals,created_at_unix_ms,updated_at_unix_ms)
                     SELECT ?2,run_id,generation_id,?3,definition_hash,?4,attempt,available_to,
                            agentless,goals,?5,updated_at_unix_ms FROM step_runs WHERE subject=?1",
                    params![runs[3].steps[0].subject,
                        format!("step-run/{}/extra-{i:02}", runs[3].generation.trim_start_matches("run-generation/")),
                        format!("extra-{i:02}"), if i == 20 { "claimed" } else { "pending" },
                        format!("{:02}", i + 1)],
                ).unwrap();
            }
            connection.execute(
                "UPDATE step_runs SET lease_expires_at_unix_ms='1600' WHERE step_path='extra-20'",
                [],
            ).unwrap();
        }
        assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], 1000).unwrap(), None);
        store.connection.lock().unwrap().execute(
            "UPDATE step_runs SET status='claimed',lease_expires_at_unix_ms='1700'
             WHERE subject=?1",
            [&runs[3].steps[0].subject],
        ).unwrap();
        assert_eq!(store.mission_collection_valid_until(&["mission/example/overview".into()], 1000).unwrap(), Some(1700));
    }

    #[test]
    fn mission_clock_fence_tracks_future_person_ask_and_blocker_acceptance() {
        let (store, origin, input) = person_work::tests::fixture();
        let ask = store.ask_person(&input).unwrap();
        let at = now_ms() + 10_000;
        store.connection.lock().unwrap().execute(
            "UPDATE claims SET accepted_at_unix_ms=?2 WHERE subject=?1 AND kind='work.person-asked'",
            params![ask.subject, at.to_string()],
        ).unwrap();
        assert_eq!(store.mission_collection_valid_until(&["mission/person-work".into()], at - 1).unwrap(), Some(at));
        assert_eq!(store.mission_collection_valid_until(&["mission/example/unselected".into()], at - 1).unwrap(), None);
        let before = store.mission_step_preview_at(&origin.run, at - 1).unwrap().2;
        let after = store.mission_step_preview_at(&origin.run, at).unwrap().2;
        let before = before.iter().find(|step| step.subject == origin.subject).unwrap();
        let after = after.iter().find(|step| step.subject == origin.subject).unwrap();
        assert_eq!(before.status, "ready");
        assert!(before.blockers.is_empty());
        assert_eq!(after.status, "waiting-person");
        assert_eq!(after.blockers, vec![ask.subject.clone()]);
        assert_eq!(store.mission_collection_valid_until(&["mission/person-work".into()], at).unwrap(), None);
        store.connection.lock().unwrap().execute(
            "UPDATE step_runs SET status='completed' WHERE subject=?1",
            [&ask.subject],
        ).unwrap();
        assert_eq!(store.mission_collection_valid_until(&["mission/person-work".into()], at - 1).unwrap(), None);
    }

    #[test]
    fn mission_clock_fence_tracks_person_attention_outside_the_step_preview() {
        let (store, origin, input) = person_work::tests::fixture();
        let ask = store.ask_person(&input).unwrap();
        let at = now_ms() + 10_000;
        {
            let connection = store.connection.lock().unwrap();
            connection.execute(
                "UPDATE claims SET accepted_at_unix_ms=?2 WHERE subject=?1 AND kind='work.person-asked'",
                params![ask.subject, at.to_string()],
            ).unwrap();
            // Keep the real ask live but place all person steps beyond the card preview.
            connection.execute(
                "UPDATE step_runs SET created_at_unix_ms='9' WHERE assignee LIKE 'person/%'",
                [],
            ).unwrap();
            connection.execute(
                "UPDATE step_runs SET status='pending' WHERE step_path='review'",
                [],
            ).unwrap();
            connection.execute(
                "UPDATE step_runs SET created_at_unix_ms='1' WHERE subject=?1",
                [&origin.subject],
            ).unwrap();
            for i in 1..20 {
                connection.execute(
                    "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                         status,attempt,available_to,agentless,goals,created_at_unix_ms,updated_at_unix_ms)
                     SELECT ?2,run_id,generation_id,?3,definition_hash,'pending',attempt,available_to,
                            agentless,goals,'1',updated_at_unix_ms FROM step_runs WHERE subject=?1",
                    params![origin.subject,
                        format!("step-run/{}/extra-{i:02}", origin.generation.trim_start_matches("run-generation/")),
                        format!("extra-{i:02}")],
                ).unwrap();
            }
            // The originating blocker is not previewed either.
            connection.execute(
                "UPDATE step_runs SET created_at_unix_ms='9' WHERE subject=?1",
                [&origin.subject],
            ).unwrap();
            connection.execute(
                "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                     status,attempt,available_to,agentless,goals,created_at_unix_ms,updated_at_unix_ms)
                 SELECT ?2,run_id,generation_id,'extra-20',definition_hash,'pending',attempt,
                        available_to,agentless,goals,'1',updated_at_unix_ms FROM step_runs WHERE subject=?1",
                params![origin.subject,
                    format!("step-run/{}/extra-20", origin.generation.trim_start_matches("run-generation/"))],
            ).unwrap();
        }
        assert!(!store.human_attention_runs_at(at - 1).unwrap().contains(&origin.run));
        assert!(store.human_attention_runs_at(at).unwrap().contains(&origin.run));
        assert_eq!(store.mission_collection_valid_until(&["mission/person-work".into()], at - 1).unwrap(), Some(at));
        assert_eq!(store.mission_collection_valid_until(&["mission/person-work".into()], at).unwrap(), None);
    }

    #[test]
    fn mission_overview_published_mission_with_zero_runs_is_shown() {
        let store = Store::open_memory("fixture").unwrap();
        publish_overview_mission(&store);
        for mission in ["mission/example/overview", "example/overview"] {
            let overview = store.mission_overview(mission, 10).unwrap();
            assert_eq!(overview["mission"], "mission/example/overview");
            assert_eq!(overview["total_runs"], 0);
            assert_eq!(overview["counts"], json!({}));
            assert_eq!(overview["newest"], json!([]));
            assert_eq!(overview["failed"], json!([]));
        }
    }

    #[test]
    fn mission_overview_runs_without_a_current_definition_are_shown() {
        let store = Store::open_memory("fixture").unwrap();
        publish_overview_mission(&store);
        let run = store
            .create_mission_run(&MissionRunRequest {
                mission: "example/overview".into(),
                revision: None,
                workspace: "/example".into(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "overview-run".into(),
            })
            .unwrap();
        // Keep the historical run and revision after the current definition disappears.
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM mission_definitions WHERE mission_id='example/overview'",
                [],
            )
            .unwrap();
        assert!(
            store
                .mission_spec("example/overview", None)
                .unwrap()
                .is_none()
        );
        let overview = store
            .mission_overview("mission/example/overview", 10)
            .unwrap();
        assert_eq!(overview["total_runs"], 1);
        assert_eq!(overview["newest"][0]["id"], run.subject);
    }
}
