use super::*;

#[test]
fn relevant_admission_discovers_a_return_after_a_replicated_change_despite_a_stale_mirror() {
    for (field, changed) in [
        ("body_digest", json!("body-b")),
        ("base_branch", json!("release")),
        ("base_sha", json!("base-b")),
        ("checks", json!([{"name": "build", "conclusion": "failure"}])),
        ("body_digest", Value::Null),
        ("base_branch", Value::Null),
        ("base_sha", Value::Null),
        ("checks", Value::Null),
    ] {
        let local = Store::open_memory("local").unwrap();
        let remote = Store::open_memory("remote").unwrap();
        let subscriptions = publish_admission_watch(&local, &[
            ("relevant", r#"admission "relevant-changes""#), ("default", ""),
        ]);
        observe_admission_pulls(&local, &subscriptions, json!([]));
        let original = admission_pull();
        observe_admission_pulls(&local, &subscriptions, json!([original.clone()]));
        let mirror = crate::model::MirroredPrior {
            compact: true,
            items: HashMap::from([(
                "resource/admission-repo/pull-request/4".into(),
                Value::Object(item_facts("resource/admission-repo", "pull_requests", 4, None, &original)),
            )]),
        };
        replicate_fully(&local, &remote);
        let revision = local.selected_desired_revision("observer/admission-repo").unwrap().unwrap();
        let next_check = now_ms() + 120_000;
        let observe = |store: &Store, pull: &Value, next_check| {
            store.record_resource_observation_attempt(
                "observer/admission-repo", &revision, None, "resource/admission-repo", None,
                &json!({"repository_id": 17, "pull_requests": [pull]}),
                next_check, &subscriptions, None, None, Some(&mirror),
            ).unwrap();
        };
        let mut changed_pull = original.clone();
        changed_pull[field] = changed;
        observe(&remote, &changed_pull, next_check);
        assert_eq!(admission_requests(&remote, "relevant").len(), 2, "{field}");
        replicate_fully(&remote, &local);
        // Replication updates the graph, not this poller's private mirror, which still holds A.
        assert_eq!(mirror.items["resource/admission-repo/pull-request/4"][field], original[field]);
        observe(&local, &original, next_check + 1);
        observe(&local, &original, next_check + 2);
        let requests = admission_requests(&local, "relevant");
        assert_eq!(requests.len(), 3, "{field}: A -> replicated B -> A, then a duplicate poll");
        let keys = requests.iter().map(|request| {
            request.body["fields"]["delivery_key"].as_str().unwrap()
        }).collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), 3, "{field}: each transition has its own admission");
        let identities = requests.iter().map(|request| {
            request.body["fields"]["change_identity"].as_str().unwrap()
        }).collect::<BTreeSet<_>>();
        assert_eq!(identities.len(), 2, "{field}: the return restores A's snapshot");
        let defaults = admission_requests(&local, "default");
        assert_eq!(defaults.len(), 1, "{field}: unchanged heads retain legacy admission");
        assert_eq!(defaults[0].body["fields"]["delivery_key"], json!(canonical_hash(&(
            "admission-review", "default", json!(17), Some("vcs.pull-request"),
            Some(4_u64), Some("head-a"),
        )).unwrap()));
        assert!(defaults[0].body["fields"].get("admission").is_none());
        assert!(defaults[0].body["fields"].get("authors").is_none());
    }
}

#[test]
fn ready_head_only_preparation_keeps_the_mirror_baseline_for_noncompact_facts() {
    let local = Store::open_memory("local").unwrap();
    let remote = Store::open_memory("remote").unwrap();
    let subscriptions = publish_admission_watch(&local, &[("default", "")]);
    observe_admission_pulls(&local, &subscriptions, json!([]));
    let original = admission_pull();
    observe_admission_pulls(&local, &subscriptions, json!([original.clone()]));
    let mirror = crate::model::MirroredPrior {
        compact: true,
        items: HashMap::from([(
            "resource/admission-repo/pull-request/4".into(),
            Value::Object(item_facts("resource/admission-repo", "pull_requests", 4, None, &original)),
        )]),
    };
    replicate_fully(&local, &remote);
    let mut changed = original.clone();
    changed["body_digest"] = json!("body-b");
    // A noncompact observer records B even though default admission ignores body changes.
    observe_admission_pulls(&remote, &subscriptions, json!([changed]));
    replicate_fully(&remote, &local);
    let revision = local.selected_desired_revision("observer/admission-repo").unwrap().unwrap();
    let facts = json!({"repository_id": 17, "pull_requests": [original]});
    let prepared = resource_preparation::read_inputs(&local.readers.get(), resource_preparation::Observation {
        observer: "observer/admission-repo", desired_revision: &revision, attempt: None,
        resource: "resource/admission-repo", facts: &facts, subscriptions: &subscriptions,
        mirrored: Some(&mirror),
    }).unwrap();
    assert_eq!(prepared.recorded_items["resource/admission-repo/pull-request/4"].as_ref().unwrap().0["body_digest"], "body-a");
    assert!(prepared.discovered.is_empty());
    assert_eq!(admission_requests(&local, "default").len(), 1);
}
