//! Registry compatibility through normal reconciliation/replication; not an old-binary claim.
use super::*;
use serde_json::json;
use crate::schedule_standby::tests::{advertise, pair, sync};

#[tokio::test]
async fn old_registry_unknown_takeover_detail_cannot_replay_a_primary_occurrence() {
    let (primary, standby, standby_spec) = pair();
    let primary = Arc::new(primary);
    let root = tempfile::tempdir().unwrap();
    let at = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let source = format!(r#"version 2
mission "harvest" state="ready" {{
 completion {{ when "all-steps-exhausted" }}
 step "done" {{ agentless }}
}}
schedule "orchard" {{ host "ivory"; standby {{ host "jade"; after "50ms" }}
 at {at:?}; work {{ mission "harvest"; workspace {:?} }}
}}"#, root.path().to_str().unwrap());
    apply_source(&primary, &source, "standby-compat-source");
    sync(&primary, &standby);
    let reconciler = Reconciler::new(primary.clone(), Arc::new(FakeRuntime::default()), "ivory".into(), Arc::new(Notify::new()));
    sync(&standby, &primary);
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let before = primary.claims_for("schedule/orchard", Some("schedule.work-started")).unwrap();
    assert_eq!(before.len(), 1, "normal primary path did not start the occurrence");
    let run = Store::scheduled_mission_run_subject("schedule/orchard", 0);
    assert!(primary.mission_run(&run).unwrap().is_some());

    let mut old_registry = st3_schema::registry().clone();
    for kind in ["schedule.occurrence-scheduled", "schedule.occurrence-reached"] {
        old_registry.claims.get_mut(kind).unwrap().fields.remove("standby");
        old_registry.claims.get_mut(kind).unwrap().fields.remove("reason");
    }
    primary.set_claim_registry(old_registry);
    advertise(&primary, 0);
    sync(&primary, &standby);
    assert!(standby.schedule_standby_adoption_epoch(&standby_spec).is_err(), "missing old-member advertisement enabled standby");
    // A retained future detail may be received even though our own standby admission holds.
    // Feed it through normal export/admission, not a direct table mutation.
    standby.append_claim(&ClaimInput {
        subject: "schedule/orchard".into(), kind: "schedule.occurrence-reached".into(), actor: None,
        fields: BTreeMap::from([
            ("occurrence".into(), Value::from(0)), ("revision".into(), json!("future-detail")),
            ("scheduled_at_unix_ms".into(), json!("1")),
            ("standby".into(), json!({"host":"jade","primary":"ivory","reason":"invented compatibility input"})),
        ]), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let exchange = standby.export_replication_exchange_answering(crate::schedule_standby::tests::FLEET,
        &primary.replication_inventory().unwrap(), &primary.replication_signature_requests().unwrap()).unwrap();
    primary.receive_replication_exchange("jade", crate::schedule_standby::tests::FLEET, &exchange).unwrap();
    let admission = primary.validate_replication_backlog().unwrap();
    assert!(admission.unknown > 0, "old registry did not retain unknown takeover detail");
    primary.project_replication_backlog().unwrap();
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); tokio::task::yield_now().await; }
    let after = primary.claims_for("schedule/orchard", Some("schedule.work-started")).unwrap();
    assert_eq!(after.len(), before.len(), "unknown future detail replayed known primary work");
    assert_eq!(after[0].body["fields"]["mission_run"], run);
}
