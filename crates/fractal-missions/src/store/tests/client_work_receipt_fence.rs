use super::*;
use fractal_store::pairing::ClientWorkClaimFence;

// A real persisted worker store, native WorkRequest claim, and independently replicated
// controller. No fabricated claim/lease rows or pending projection metadata.
struct Fixture {
    _directory: tempfile::TempDir,
    worker: Store,
    controller: Store,
    owner: StepRunView,
    claim: ClaimRecord,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let worker = Store::open(&directory.path().join("worker.sqlite"), "node").unwrap();
        publish_mission(&worker, r#"version 2
agent "worker" { workspace "/tmp"; command "true" }
mission "client-receipt" state="ready" {
 goal "Admit only effects owned by the current native worker."
 step "work" { assigned-to "agent/node.worker" }
}
"#, "client-receipt-definition");
        let run = worker.create_mission_run(&MissionRunRequest {
            mission: "client-receipt".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/requester".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "client-receipt-run".into(),
        }).unwrap();
        worker.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
        let owner = worker.work_action(&run.steps[0].subject, "claim",
            &carry_request("original", "client-receipt-claim")).unwrap();
        let claim = worker.claims_for(&owner.subject, Some("work.claimed")).unwrap()
            .into_iter().max_by_key(|claim| claim.store_index).unwrap();
        assert_eq!(claim.origin, "node");
        let controller = Store::open_memory("controller").unwrap();
        loop {
            let exchange = exchange_from(&worker, &controller.replication_inventory().unwrap());
            if exchange.envelopes.is_empty() { break; }
            receive_and_project(&controller, "node", &exchange);
        }
        assert_eq!(worker.replication_inventory().unwrap().digest,
            controller.replication_inventory().unwrap().digest);
        Self { _directory: directory, worker, controller, owner, claim }
    }

    fn append(&self, kind: &str) -> Result<ClaimRecord, FractalError> {
        self.worker.append_client_operation_claim_for_work(&ClaimInput {
            subject: "custom/client/control-grant/native-fence".into(),
            kind: kind.into(), actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([("command_id".into(), json!("command/native-fence"))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }, &ClientWorkClaimFence {
            run: &self.owner.run, step: &self.owner.subject, generation: &self.owner.generation,
            attempt: self.owner.attempt, actor: "agent/node.worker", incarnation: Some("original"),
            claim_id: &self.claim.id, expires_at: now_ms() + 60_000,
        })
    }

    fn assert_no_receipt(&self) {
        assert!(self.worker.claims_for("custom/client/control-grant/native-fence", None).unwrap().is_empty());
        let receipts: u64 = self.worker.readers.get().query_row(
            "SELECT count(*) FROM claims WHERE kind='client.operation-authorized'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(receipts, 0, "refusal must roll back the operation and its trusted receipt");
    }
}

#[test]
fn client_work_receipt_current_native_claim_commits_trusted_receipt() {
    let fixture = Fixture::new();
    for kind in ["custom.client.control-granted", "custom.client.control-consumed"] {
        let claim = fixture.append(kind).unwrap();
        assert!(fixture.worker.client_operation_is_trusted(&claim).unwrap());
        let reloaded = fixture.worker.claims_for(&claim.subject, Some(kind)).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert!(fixture.worker.client_operation_is_trusted(&reloaded[0]).unwrap());
    }
}

#[test]
fn client_work_receipt_native_cancel_refuses_both_effect_kinds() {
    let fixture = Fixture::new();
    fixture.worker.set_mission_run_state(&fixture.owner.run, "cancelled", "terminal",
        Some("native cancellation wins")).unwrap();
    assert_eq!(fixture.worker.mission_run(&fixture.owner.run).unwrap().unwrap().status, "cancelled");
    for kind in ["custom.client.control-granted", "custom.client.control-consumed"] {
        let error = fixture.append(kind).unwrap_err();
        assert_eq!(error.code, "grant-expired", "{error:?}");
    }
    fixture.assert_no_receipt();
}

#[test]
fn client_work_receipt_native_reclaim_refuses_the_old_claim() {
    let fixture = Fixture::new();
    fixture.worker.work_action(&fixture.owner.subject, "release",
        &carry_request("original", "native-fence-release")).unwrap();
    let replacement = fixture.worker.work_action(&fixture.owner.subject, "claim",
        &carry_request("replacement", "native-fence-reclaim")).unwrap();
    assert_eq!(replacement.claim_incarnation.as_deref(), Some("replacement"));
    assert_ne!(fixture.worker.claims_for(&fixture.owner.subject, Some("work.claimed")).unwrap()
        .into_iter().max_by_key(|claim| claim.store_index).unwrap().id, fixture.claim.id);
    let error = fixture.append("custom.client.control-consumed").unwrap_err();
    assert_eq!(error.code, "grant-expired", "{error:?}");
    fixture.assert_no_receipt();
}

#[test]
fn client_work_receipt_pending_native_cancel_drains_actual_owner_before_admission() {
    let fixture = Fixture::new();
    fixture.controller.set_mission_run_state(&fixture.owner.run, "cancelled", "terminal",
        Some("replicated native cancellation wins")).unwrap();
    assert_eq!(fixture.controller.mission_run(&fixture.owner.run).unwrap().unwrap().status, "cancelled");
    // Existing replication yield hook interrupts AFTER a real dirty marker commits and
    // BEFORE its deferred run fold. This is not a synthetic update of projected rows.
    receive_until_run_projection_is_pending(&fixture.controller, &fixture.worker);
    assert!(fixture.worker.run_projection_pending(&fixture.owner.subject).unwrap());
    {
        let connection = fixture.worker.readers.get();
        let stale: (String, String) = connection.query_row(
            "SELECT run.status, step.status FROM mission_runs run JOIN step_runs step ON step.run_id=run.id WHERE step.subject=?1",
            [&fixture.owner.subject], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(stale, ("running".into(), "claimed".into()),
            "reach the admission oracle with actual deferred, still-live owner rows");
        let cancelled: u64 = connection.query_row(
            "SELECT count(*) FROM claims WHERE subject=?1 AND kind='mission-run.state' AND json_extract(body,'$.fields.status')='cancelled'",
            [&fixture.owner.run], |row| row.get(0),
        ).unwrap();
        assert_eq!(cancelled, 1, "native cancellation is already durably accepted");
    }
    let error = fixture.append("custom.client.control-consumed").unwrap_err();
    assert_eq!(error.code, "grant-expired", "{error:?}");
    assert!(!fixture.worker.run_projection_pending(&fixture.owner.subject).unwrap());
    assert_eq!(fixture.worker.mission_run(&fixture.owner.run).unwrap().unwrap().status, "cancelled");
    fixture.assert_no_receipt();
}
