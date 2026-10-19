#![allow(unused_imports)]
use crate::store::terminal_gate_evidence::native_source_shared::*;
use crate::graph::{parse_execution_intent, parse_intent};
use crate::store::{Store, append_claim_tx, project_mission_run_created, select_replicated_desired, select_replicated_mission};
use serde_json::json;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use crate::store::terminal_gate_evidence_shared::*;
use crate::store::terminal_gate_evidence::*;
use crate::model::{DesiredSubject, GateSpec, MemberKind, MissionSpec};
use smallclaims::ivm::install::{Installer, Limits, Outcome, ScanPage};
use std::collections::{BTreeMap, BTreeSet};
use std::collections::btree_map::Entry;
use crate::store::terminal_gate_evidence::native_source_shared::*;

pub struct Fixture {
    pub source: NativeSource,
    pub store: Store,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
        let source = {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            let source = NativeSource::attach(
                &tx,
                Binding {
                    origin: "node".into(),
                    mission: "one".into(),
                    run: "mission-run/one".into(),
                    generation: "run-generation/one".into(),
                    step: "step-run/one/prepare".into(),
                    path: "prepare".into(),
                    gate: "prepared".into(),
                    exec: "exec/one/probe".into(),
                },
            )
            .unwrap();
            tx.commit().unwrap();
            source
        };
        Self { source, store, dir }
    }
    pub fn seeded() -> Self {
        let fixture = Self::empty();
        let mut writer = fixture.store.connection.write();
        let tx = writer.transaction().unwrap();
        seed_native(&tx, &fixture.source.binding);
        fixture.source.finish(&tx).unwrap();
        assert_eq!(fixture.source.doctor_line(&tx)["status"], "warn");
        tx.commit().unwrap();
        drop(writer);
        fixture
    }
    pub fn edit(&self, change: impl FnOnce(&Transaction<'_>)) {
        let mut writer = self.store.connection.write();
        let tx = writer.transaction().unwrap();
        change(&tx);
        self.source.finish(&tx).unwrap();
        tx.commit().unwrap();
    }
    pub fn line(&self) -> Value {
        let mut writer = self.store.connection.write();
        let tx = writer.transaction().unwrap();
        self.source.doctor_line(&tx)
    }
    pub fn measured(&self, change: impl FnOnce(&Transaction<'_>)) -> SqliteWork {
        let mut writer = self.store.connection.write();
        let tx = writer.transaction().unwrap();
        let work = SqliteWorkScope::start();
        change(&tx);
        self.source.finish(&tx).unwrap();
        let measured = work.finish();
        tx.commit().unwrap();
        measured
    }
    pub fn populate_foreign(&self, n: usize) {
        let mut writer = self.store.connection.write();
        let tx = writer.transaction().unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h7bf4e39a6bfb owner=st3_store::store::terminal_gate_evidence::native_source_tests_shared TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES('foreign','node',999,'fixture-foreign','1')",[]).unwrap();
        // Keep the raw legacy volume fixture in bounded batches. Replica-record writes
        // intentionally invalidate this private source adapter; a single 100000-row legacy
        // batch would instead multiply position-recovery work while settling publication.
        tx.execute(/* sqlite-large-value: allow-for-now lv-h063850ef4d73 owner=st3_store::store::terminal_gate_evidence::native_source_tests_shared TODO=chunk-or-reference-exact-wire-values */ "WITH RECURSIVE pages(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM pages WHERE n<(?1-1)/64)
            INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
            SELECT 'foreign-page/'||n,'node',1000+n,'fixture-page/'||n,'1' FROM pages", [n]).unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h14b8736a45b7 owner=st3_store::store::terminal_gate_evidence::native_source_tests_shared TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE rows(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM rows WHERE n<?1)
            INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
            SELECT 'foreign/'||n,'foreign-page/'||((n-1)/64),'exec/unrelated/'||n,'runtime.observed','node','{}','[]','1' FROM rows WHERE n<=?1",
            [n]).unwrap();
        tx.commit().unwrap();
    }
}

// Shared only by the two private fixture modules; the native writers are unchanged.
pub fn seed_native(tx: &Transaction<'_>, binding: &Binding) -> String {
    let intent = parse_intent(
        r#"
version 2
mission "one" state="ready" {
 goal "Private source fixture."
 step "prepare" {
   exec "probe" { workspace "/tmp"; command "true"; restart "never" }
   gate "prepared" { field "exit_code" "exec/one/probe" is 0 }
 }
}
"#,
        "node",
    )
    .unwrap();
    let mission = intent.missions.values().next().unwrap();
    let execution = parse_execution_intent(
        mission.steps["prepare"]
            .declarations_kdl
            .as_deref()
            .unwrap(),
        "node",
        "one",
    )
    .unwrap();
    let mut desired = execution.subjects["exec/one/probe"].clone();
    assert_eq!(desired.subject, binding.exec);
    assert_eq!(desired.owner_run.as_deref(), Some("mission-run/one"));
    desired.owner_run = Some(binding.run.clone());
    desired.owner_generation = Some(binding.generation.clone());
    desired.owner_step = Some(binding.step.clone());
    let publication = append_claim_tx(
        tx,
        "node",
        &mission.subject,
        "mission.published",
        Some("person/test"),
        &serde_json::to_value(mission).unwrap(),
        &[],
        None,
    )
    .unwrap();
    select_replicated_mission(tx, &publication, publication.store_index).unwrap();
    let created = append_claim_tx(
        tx,
        "node",
        "mission-run/one",
        "mission-run.created",
        None,
        &json!({"fields":{"mission":"mission/one","revision":mission.revision,
                "current_generation":"run-generation/one","workspace":"/tmp",
                "requester":"person/test","mode":"run","root_revision":mission.revision,
                "root_mission_run":"mission-run/one"},"evidence":[]}),
        &[],
        None,
    )
    .unwrap();
    project_mission_run_created(tx, &created).unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h4f41218f5393 owner=st3_store::store::terminal_gate_evidence::native_source_tests_shared TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='working' WHERE subject='step-run/one/prepare'",
        [],
    )
    .unwrap();
    let declaration = append_claim_tx(
        tx,
        "node",
        "exec/one/probe",
        "intent.desired",
        Some("person/test"),
        &serde_json::to_value(&desired).unwrap(),
        &[],
        None,
    )
    .unwrap();
    select_replicated_desired(tx, &declaration, &desired).unwrap();
    append_claim_tx(
        tx,
        "node",
        "exec/one/probe",
        "runtime.observed",
        None,
        &json!({"fields":{"runtime_id":"fixture-probe","host":"node","terminal":false,
                "status":"exited","exit_code":2,"incarnation_id":"fixture-incarnation"},
                "evidence":[declaration.id]}),
        std::slice::from_ref(&declaration.id),
        None,
    )
    .unwrap();
    declaration.id
}
