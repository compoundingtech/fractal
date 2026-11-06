use super::*;
use crate::store::custom::RegistrationRequest;

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const SNAPSHOT: &str = "custom/attention/snapshot/01900000-0000-7000-8000-000000000001";
const CONTROL: &str = "custom/attention/control/01900000-0000-7000-8000-000000000003";
const RUNTIME: &str = "agent/example/runtime";

fn designate(store: &Store, name: &str) -> Result<ApplyResponse, FractalError> {
    let intent = crate::parse_intent(&format!("version 2\nagent \"{name}\" {{ command \"true\"; notice-runtime; }}"), "example").unwrap();
    let expected = intent.subjects.keys().map(|s| (s.clone(), Vec::new())).collect();
    store.apply_as(&intent, &expected, name, Some("person/example"))
}

fn setup() -> (Store, ClaimRecord) {
    let store = Store::open_memory("example").unwrap();
    designate(&store, "example/runtime").unwrap();
    for manifest in notice::manifests() {
        store.register_custom_kind(&RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
    }
    let fixture: Value = serde_json::from_str(include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&input(SOURCE, "custom.attention.notice-created", "agent/example/producer", fixture["snapshot"]["input"].clone())).unwrap();
    (store, source)
}

fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput { subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(), ..custom::tests_shared_delivery::empty_input() }
}

fn linked(source: &ClaimRecord, control: bool) -> Value {
    let mut fields = json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example","runtime":RUNTIME});
    if control {
        for field in ["seen_at", "dismissed_at", "snoozed_until"] { fields[field] = Value::Null; }
    } else { fields["policy"] = json!({"disposition":"deliver"}); fields["delivery"] = json!([]); }
    fields
}

#[test]
fn notice_first_creator_forgery_and_forged_runtime_are_refused() {
    let (store, source) = setup();
    for actor in ["agent/example/producer", "person/example"] {
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", actor, linked(&source, false))).is_err());
    }
    let mut fields = linked(&source, false); fields["runtime"] = json!("agent/example/forged");
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err());
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false))).unwrap();
}

#[test]
fn notice_agent_and_runtime_cannot_write_person_controls() {
    let (store, source) = setup();
    for actor in [RUNTIME, "agent/example/producer", "person/other"] {
        assert!(store.append_claim(&input(CONTROL, "custom.attention.control-created", actor, linked(&source, true))).is_err());
    }
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&source, true))).unwrap();
    assert!(store.append_claim(&input(CONTROL, "custom.attention.control-updated", RUNTIME, linked(&source, true))).is_err());
}

#[test]
fn notice_altered_links_recipient_producer_and_manifest_are_refused() {
    let (store, source) = setup();
    for (field, value) in [("source_id", json!("custom/attention/source/other")), ("creation_claim", json!("0".repeat(64))),
        ("recipient", json!("person/other")), ("producer", json!(RUNTIME)), ("_registration", json!("0".repeat(64)))] {
        let mut fields = linked(&source, false); fields[field] = value;
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err(), "{field}");
    }
    let mut fields = custom::fields(&source); fields.remove("_registration"); fields.insert("links".into(), json!([]));
    assert!(store.append_claim(&ClaimInput { fields, ..input(SOURCE, &source.kind, "agent/example/producer", json!({})) }).is_err());
}

#[test]
fn notice_second_designated_holder_is_refused_and_missing_binding_fails_closed() {
    let (store, _) = setup();
    assert_eq!(designate(&store, "example/other").unwrap_err().code, "notices-unavailable");
    let empty = Store::open_memory("example").unwrap();
    assert!(runtime(&empty.readers.get()).is_err());
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("DELETE FROM desired WHERE subject=?1", [RUNTIME])?;
        custom::rebuild(tx)?;
        let row = custom::source(tx, SOURCE)?.unwrap();
        assert_ne!(row["state"], "ready");
        assert!(row["error"].as_str().unwrap().contains("missing notice-runtime"));
        Ok(())
    }).unwrap().unwrap();
}

#[test]
fn notice_reserved_prefix_squat_and_alternate_claim_prefix_are_refused() {
    let store = Store::open_memory("example").unwrap();
    let mut manifest = notice::manifests()[0].clone(); manifest.kind = "attention.squat".into();
    assert!(store.register_custom_kind(&RegistrationRequest { manifest, actor: "person/example".into() }).is_err());
    assert!(store.append_claim(&input("custom/other/fact", "custom.attention.snapshot-created", RUNTIME, json!({}))).is_err());
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, json!({}))).is_err());
}

#[test]
fn notice_generic_reads_never_expose_reserved_raw_rows() {
    let (store, source) = setup();
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false))).unwrap();
    for subject in [SOURCE, SNAPSHOT, CONTROL] { assert!(store.custom_subject(subject).is_err()); }
    for kind in [None, Some("attention.notice-source"), Some("attention.notice-snapshot")] {
        assert!(store.custom_subjects(kind, None, None, 10).unwrap().is_empty());
    }
}

#[test]
fn notice_source_payload_cannot_forge_authenticated_producer() {
    let (store, source) = setup();
    let mut fields = custom::fields(&source); fields.insert("producer".into(), json!("agent/example/forged"));
    assert!(store.append_claim(&ClaimInput { subject: format!("{}other", notice::SOURCE_PREFIX), fields,
        ..input(SOURCE, &source.kind, "agent/example/producer", json!({})) }).is_err());
}

#[test]
fn notice_later_replicated_source_conflict_invalidates_links_like_rebuild() {
    let (store, source) = setup();
    for (subject, kind, actor, control) in [
        (SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, false),
        (CONTROL, "custom.attention.control-created", "person/example", true),
    ] {
        store.append_claim(&input(subject, kind, actor, linked(&source, control))).unwrap();
        let connection = store.readers.get();
        assert_eq!(custom::source(&connection, subject).unwrap().unwrap()["state"], "ready");
        let edges: i64 = connection.query_row(
            "SELECT count(*) FROM custom_dependencies WHERE source=?1 AND dependency IN (?2,?3)",
            params![subject, SOURCE, source.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(edges, 2);
    }
    let remote = Store::open_memory("remote").unwrap();
    custom::tests_shared_delivery::legacy_sync(&store, &remote);
    // Model a mixed-version member admitting a competing creation in a later batch.
    let mut fields = custom::fields(&source); fields.insert("summary".into(), json!("Competing input"));
    remote.connection.batched(|tx| append_claim_tx(tx, &remote.origin, SOURCE,
        &source.kind, source.actor.as_deref(), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
    custom::tests_shared_delivery::legacy_sync(&remote, &store);
    store.connection.batched(|tx| -> Result<()> {
        let incremental = [SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap().unwrap());
        for row in &incremental {
            assert_eq!(row["state"], "invalid");
            assert!(row["error"].as_str().unwrap().contains("conflicting or ineligible source creation"));
        }
        custom::rebuild(tx)?;
        for (subject, before) in [SNAPSHOT, CONTROL].iter().zip(incremental) {
            assert_eq!(custom::source(tx, subject)?.unwrap(), before);
        }
        Ok(())
    }).unwrap().unwrap();
}
