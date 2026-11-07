use super::*;

#[test]
fn vanished_runtime_keeps_only_its_local_incarnation() {
    for (prior_runtime, status, observed, expected) in [
        (Some("example.worker"), "vanished", None, Some("source-incarnation")),
        (Some("older-runtime"), "vanished", None, None),
        (None, "vanished", None, None),
        (Some("example.worker"), "unknown", None, None),
        (Some("example.worker"), "vanished", Some("observed-incarnation"), Some("observed-incarnation")),
    ] {
        let store = Arc::new(Store::open_memory("source").unwrap());
        apply_source(&store, "version 2\nagent \"example/worker\" { host \"source\"; workspace \"/tmp\"; command \"true\" }", "seat");
        let subject = store.desired_subjects().unwrap().remove(0);
        let member = subject.member.as_ref().unwrap();
        assert_eq!(member.runtime_id, "example.worker");
        let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()),
            "source".into(), Arc::new(Notify::new()));
        if let Some(runtime) = prior_runtime {
            let mut old = member.clone();
            old.runtime_id = runtime.into();
            reconciler.record_once(&subject.subject, "runtime.observed",
                member_fields(&old, "running", Some("source-incarnation"), false)).unwrap();
        }
        // A destination's newer running observation must never become the source's identity.
        let peer_store = Arc::new(Store::open_memory("destination").unwrap());
        peer_store.import_replication("source", &store.export_replication(0).unwrap()).unwrap();
        let peer = Reconciler::new(peer_store.clone(), Arc::new(FakeRuntime::default()),
            "destination".into(), Arc::new(Notify::new()));
        let mut destination = member.clone();
        destination.host = "destination".into();
        peer.record_once(&subject.subject, "runtime.observed",
            member_fields(&destination, "running", Some("destination-incarnation"), false)).unwrap();
        store.import_replication("destination", &peer_store.export_replication(0).unwrap()).unwrap();
        assert_eq!(store.latest_observation(&subject.subject, "runtime.observed").unwrap().unwrap().origin,
            "destination");
        let missing = RuntimeObservation {
            runtime_id: member.runtime_id.clone(), terminal: true, status: status.into(),
            exit_code: None, incarnation_id: observed.map(str::to_owned),
        };
        reconciler.record_member_as(&subject, member, &missing, false).unwrap();
        let local = crate::placement::latest_by_origin(&store, &subject.subject, u64::MAX)
            .unwrap().remove("source").unwrap();
        assert_eq!(crate::placement::field(&local, "incarnation_id"), expected,
            "prior={prior_runtime:?} status={status} observed={observed:?}");
        assert_eq!(crate::placement::field(&local, "status"), Some(status));
        let cut = store.index().unwrap();
        reconciler.record_member_as(&subject, member, &missing, false).unwrap();
        assert_eq!(store.index().unwrap(), cut, "an unchanged retirement must deduplicate");
    }
}
