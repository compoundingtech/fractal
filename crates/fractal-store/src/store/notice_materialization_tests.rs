use super::*;
use crate::store::notice_materialization::{NoticeCollectionState, NoticeMaterializationState};

fn notice_materialization_rebuild_parity(store: &Store) {
    let before = serde_json::to_value(store.notice_materializations("person/example").unwrap()).unwrap();
    store.connection.batched(custom::rebuild).unwrap().unwrap();
    assert_eq!(serde_json::to_value(store.notice_materializations("person/example").unwrap()).unwrap(), before);
    for subject in [SOURCE, CONTROL] {
        assert_eq!(custom::source(&store.readers.get(), subject).unwrap().unwrap()["state"], "ready");
        assert!(store.custom_subject(subject).is_err());
    }
}

fn notice_materialization_role(store: &Store, name: &str, designated: bool) {
    let subject = format!("agent/{name}");
    let token = store.selected_desired_token(&subject).unwrap();
    let key = format!("notice-role:{subject}:{designated}:{}", token.as_deref().unwrap_or("absent"));
    let role = if designated { "notice-runtime;" } else { "" };
    let intent = crate::parse_intent(&format!("version 2\nagent \"{name}\" {{ command \"true\"; {role} }}"), "example").unwrap();
    store.apply_as(&intent, &BTreeMap::from([(subject, token.into_iter().collect())]), &key, Some("person/example")).unwrap();
}

#[test]
fn notice_materialization_ready_latest_canonical_and_private() {
    let store = linked_store();
    let initial = store.newest_claim_for_subject_kind(SNAPSHOT, "custom.attention.snapshot-created").unwrap().unwrap();
    let binding = store.selected_desired_token(RUNTIME).unwrap().unwrap();
    let ready = store.notice_materializations("person/example").unwrap();
    assert_eq!(ready.state, NoticeCollectionState::Ready);
    assert_eq!(ready.notices.len(), 1);
    assert_eq!(ready.notices[0].id, notice::id_for_source(SOURCE).unwrap());
    assert_eq!(ready.notices[0].source_id, SOURCE);
    assert_eq!(ready.notices[0].state, NoticeMaterializationState::Ready { snapshot_claim: initial.id, binding_claim: binding.clone() });
    let source = custom::claims(&store.readers.get(), SOURCE).unwrap().remove(0);
    let remote = Store::open_memory("aardvark").unwrap();
    custom::tests_shared_delivery::legacy_sync(&store, &remote);
    let alternate = "custom/attention/snapshot/01900000-0000-7000-8000-000000000004";
    remote.set_write_clock_at(initial.accepted_at_unix_ms + 10).unwrap();
    remote.append_claim(&input(alternate, "custom.attention.snapshot-created", RUNTIME, linked(&remote, &source, false))).unwrap();
    store.set_write_clock_at(initial.accepted_at_unix_ms + 20).unwrap();
    let fields = linked(&store, &source, false);
    let latest = store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-updated", RUNTIME, fields)).unwrap();
    custom::tests_shared_delivery::legacy_sync(&remote, &store);
    let canonical: String = store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-ha3f772f6c338 owner=fractal_store::store::notice_materialization_tests TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id FROM claims WHERE kind IN ('custom.attention.snapshot-created','custom.attention.snapshot-updated') ORDER BY CANONICAL_DESC(claims) LIMIT 1"), [], |row| row.get(0)).unwrap();
    assert_eq!(canonical, latest.id);
    assert_eq!(store.notice_materializations("person/example").unwrap().notices[0].state,
        NoticeMaterializationState::Ready { snapshot_claim: latest.id, binding_claim: binding });
    assert!(store.notice_materializations("person/other").unwrap().notices.is_empty());
    for subject in [SOURCE, SNAPSHOT, CONTROL, alternate] { assert!(store.custom_subject(subject).is_err()); }
    assert!(store.custom_subjects(None, None, None, 10).unwrap().is_empty());
    notice_materialization_rebuild_parity(&store);
}

#[test]
fn notice_materialization_missing_snapshot_is_explicitly_rebuilding() {
    let (store, source) = setup();
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&store, &source, true))).unwrap();
    let result = store.notice_materializations("person/example").unwrap();
    assert!(matches!(result.state, NoticeCollectionState::Rebuilding { reason } if !reason.is_empty()));
    assert_eq!(result.notices.len(), 1);
    assert_eq!(result.notices[0].source_id, SOURCE);
    assert!(matches!(&result.notices[0].state, NoticeMaterializationState::Rebuilding { reason } if !reason.is_empty()));
    notice_materialization_rebuild_parity(&store);
}

#[test]
fn notice_materialization_stop_or_removed_designation_is_unavailable() {
    for stop in [false, true] {
        let store = linked_store();
        if stop {
            let intent = crate::parse_intent("version 2\nstop \"agent/example/runtime\"", "example").unwrap();
            let token = store.selected_desired_token(RUNTIME).unwrap().unwrap();
            store.apply_as(&intent, &BTreeMap::from([(RUNTIME.into(), vec![token])]), "notice-stop", Some("person/example")).unwrap();
        } else { notice_materialization_role(&store, "example/runtime", false); }
        let result = store.notice_materializations("person/example").unwrap();
        assert!(matches!(result.state, NoticeCollectionState::Unavailable { reason } if !reason.is_empty()));
        assert_eq!(result.notices.len(), 1);
        assert!(matches!(&result.notices[0].state, NoticeMaterializationState::Unavailable { reason } if !reason.is_empty()));
        notice_materialization_rebuild_parity(&store);
    }
}

#[test]
fn notice_materialization_same_actor_return_requires_fresh_binding_snapshot() {
    let store = linked_store();
    let source = custom::claims(&store.readers.get(), SOURCE).unwrap().remove(0);
    let original_binding = store.selected_desired_token(RUNTIME).unwrap().unwrap();
    notice_materialization_role(&store, "example/runtime", false);
    for name in ["example/next-runtime", "example/runtime"] {
        notice_materialization_role(&store, name, true);
        let binding = store.selected_desired_token(&format!("agent/{name}")).unwrap().unwrap();
        assert_ne!(binding, original_binding);
        let result = store.notice_materializations("person/example").unwrap();
        assert!(matches!(result.state, NoticeCollectionState::Rebuilding { .. }));
        assert_eq!(result.notices.len(), 1);
        assert!(matches!(result.notices[0].state, NoticeMaterializationState::Rebuilding { .. }));
        notice_materialization_rebuild_parity(&store);
        let subject = format!("{}01900000-0000-7000-8000-00000000000{}", notice::SNAPSHOT_PREFIX, if name == "example/runtime" { 6 } else { 5 });
        let mut fields = linked(&store, &source, false);
        fields["runtime"] = json!(format!("agent/{name}"));
        fields["runtime_binding"] = json!(binding);
        let snapshot = store.append_claim(&input(&subject, "custom.attention.snapshot-created", &format!("agent/{name}"), fields)).unwrap();
        let result = store.notice_materializations("person/example").unwrap();
        assert_eq!(result.state, NoticeCollectionState::Ready);
        assert_eq!(result.notices[0].state, NoticeMaterializationState::Ready { snapshot_claim: snapshot.id, binding_claim: binding });
        notice_materialization_rebuild_parity(&store);
        if name != "example/runtime" { notice_materialization_role(&store, name, false); }
    }
    assert_eq!(store.claim_by_id(&source.id).unwrap().unwrap().body, source.body);
}

#[test]
fn notice_materialization_empty_store_missing_prerequisites_is_unavailable() {
    for registered in [false, true] {
        let store = Store::open_memory("example").unwrap();
        if registered {
            for manifest in notice::manifests() {
                store.register_custom_kind(&RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
            }
        } else { designate(&store, "example/runtime").unwrap(); }
        let result = store.notice_materializations("person/example").unwrap();
        assert!(matches!(result.state, NoticeCollectionState::Unavailable { reason } if !reason.is_empty()));
        assert!(result.notices.is_empty());
    }
}
