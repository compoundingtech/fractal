//! The part of `store::operations` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_delivery::store::operations::*;

/// The generated fleet fixture, a `Store` method before the cut.
#[cfg(any(test, feature = "test-support"))]
pub trait OperationsFixtureStore {
    fn seed_operational_fleet(&self);
}

#[cfg(any(test, feature = "test-support"))]
impl OperationsFixtureStore for Store {
    /// Generated read fixture: 2,401 runs and steps, 250,000 unrelated claims, and terminal
    /// transitions. Projection rows are bulk-loaded to keep fleet-scale tests inexpensive.
    fn seed_operational_fleet(&self) {
        let source = "version 2\nmission \"example/fleet\" state=\"ready\" {\n goal \"Build artifacts.\"\n step \"build\" { goal \"Build the example.\" }\n}\n";
        let intent = crate::graph::parse_intent(source, &self.origin).unwrap();
        let planned = self
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        self.apply(&intent, &planned.subject_tokens, "fixture-mission")
            .unwrap();
        let run = self
            .create_mission_run(&MissionRunRequest {
                mission: "example/fleet".into(),
                revision: None,
                workspace: "/example".into(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "fixture-run".into(),
            })
            .unwrap();
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let batch: String = transaction
            .query_row("SELECT batch_id FROM claims LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let at = now_ms().to_string();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h58850df68114 owner=fractal_missions::store::operations TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE seq(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM seq WHERE n<249999)
             INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
             SELECT 'fixture-heartbeat-' || n,?1,'agent/example/observer','harness.observed',
                    'fixture',?2,'[]',?3 FROM seq",
            params![batch,r#"{"fields":{"state":"idle","incarnation_id":"fixture"}}"#,at],
        ).unwrap();
        for i in 0..2400 {
            let id = format!("example/fleet/seed-{i:05}");
            let generation = format!("fixture-generation-{i}");
            let status = ["running", "completed", "failed", "cancelled"][i % 4];
            let phase = if status == "running" {
                "normal"
            } else {
                "terminal"
            };
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hcb67b6580e72 owner=fractal_missions::store::operations TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs SELECT ?1,mission_id,initial_revision,?2,root_revision,?1,NULL,workspace,requester,inputs,mode,?3,?4,?5,?5 FROM mission_runs WHERE id=?6",
                params![id,generation,status,phase,at,run.id]).unwrap();
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h5574f8672a86 owner=fractal_missions::store::operations TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations SELECT ?1,?2,revision,NULL,?3,actor,reason,?4,?4 FROM run_generations WHERE id=?5",
                params![generation,id,status,at,run.generation.trim_start_matches("run-generation/")]).unwrap();
            let step = format!("step-run/{generation}/build");
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h3566d793378e owner=fractal_missions::store::operations TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,assignee,available_to,agentless,title,goals,created_at_unix_ms,updated_at_unix_ms)
                SELECT ?1,?2,?3,step_path,definition_hash,?4,attempt,'agent/example/builder',available_to,agentless,title,goals,?5,?5 FROM step_runs WHERE subject=?6",
                params![step,id,generation,status,at,run.steps[0].subject]).unwrap();
            let reason = if i % 8 == 2 {
                "the active execution timeout expired"
            } else {
                "the example check failed"
            };
            for (subject, kind) in [
                (format!("mission-run/{id}"), "mission-run.state"),
                (step, "step-run.state"),
            ] {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hbf4b8385d9a3 owner=fractal_missions::store::operations TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,?2,?3,?4,'fixture',?5,'[]',?6)",
                    params![format!("fixture-{kind}-{i}"),batch,subject,kind,json!({"fields":{"status":status,"phase":phase,"reason":reason}}).to_string(),at]).unwrap();
            }
        }
        transaction.commit().unwrap();
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::operations_tests_shared_seats::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::operations_tests_shared_seats::{clock_run};
    use super::*;

    #[test]
    fn fleet_overview_counts_all_runs_but_bounds_previews() {
        let store = Store::open_memory("fixture").unwrap();
        store.seed_operational_fleet();
        let overview = store.mission_overview("mission/example/fleet", 10).unwrap();
        assert_eq!(overview["total_runs"], 2401);
        assert_eq!(overview["counts"]["running"], 601);
        assert_eq!(overview["counts"]["failed"], 600);
        assert_eq!(overview["newest"].as_array().unwrap().len(), 10);
        assert_eq!(overview["failed"].as_array().unwrap().len(), 10);
        assert!(serde_json::to_vec(&overview).unwrap().len() < 20_000);
    }

    #[test]
    fn fleet_outcomes_page_stably_keep_reasons_and_filter_time_status_and_actor() {
        let store = Store::open_memory("fixture").unwrap();
        store.seed_operational_fleet();
        for collection in ["missions", "work"] {
            let first = store
                .outcome_history(collection, 0, now_ms(), Some("failed"), None, u64::MAX, 50)
                .unwrap();
            assert_eq!(first["items"].as_array().unwrap().len(), 50);
            assert_eq!(first["has_more"], true);
            assert!(
                first["items"][0]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("failed")
            );
            let before = first["next_before"].as_u64().unwrap();
            let second = store
                .outcome_history(collection, 0, now_ms(), Some("failed"), None, before, 50)
                .unwrap();
            assert!(second["items"][0]["index"].as_u64().unwrap() < before);
            let timed = store
                .outcome_history(
                    collection,
                    0,
                    now_ms(),
                    Some("timed-out"),
                    None,
                    u64::MAX,
                    50,
                )
                .unwrap();
            assert!(
                timed["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["reason"].as_str().unwrap().contains("timeout"))
            );
            let cancelled = store
                .outcome_history(
                    collection,
                    0,
                    now_ms(),
                    Some("cancelled"),
                    None,
                    u64::MAX,
                    50,
                )
                .unwrap();
            assert_eq!(cancelled["items"].as_array().unwrap().len(), 50);
            let old = store
                .outcome_history(collection, 0, 1, None, None, u64::MAX, 50)
                .unwrap();
            assert!(old["items"].as_array().unwrap().is_empty());
        }
        let actor = store
            .outcome_history(
                "work",
                0,
                now_ms(),
                Some("failed"),
                Some("agent/example/builder"),
                u64::MAX,
                50,
            )
            .unwrap();
        assert_eq!(actor["items"].as_array().unwrap().len(), 50);
        let other = store
            .outcome_history(
                "work",
                0,
                now_ms(),
                Some("failed"),
                Some("agent/example/other"),
                u64::MAX,
                50,
            )
            .unwrap();
        assert!(other["items"].as_array().unwrap().is_empty());
        // A retry changes the current row, while the earlier terminal transition remains.
        store.connection.lock().unwrap().execute("UPDATE step_runs SET status='ready' WHERE subject='step-run/fixture-generation-2398/build'",[]).unwrap();
        let history = store
            .outcome_history("work", 0, now_ms(), Some("failed"), None, u64::MAX, 1)
            .unwrap();
        assert_eq!(
            history["items"][0]["subject"],
            "step-run/fixture-generation-2398/build"
        );
        assert_eq!(history["items"][0]["status"], "failed");
    }
}
