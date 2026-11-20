use super::*;

fn roots(revision: u64) -> NormalizedIntent {
    parse_intent(&format!(r#"version 2
objective "garden/throughput" {{
    about "Published work revision {revision}"
    indicator "count" {{ event "custom.garden.done" at="done_at"; subject-prefix "custom/garden/" }}
    window "1h" min-samples=1
    target "per-hour" min=1
}}
github-source "garden" {{
    host "github.com"
    generation {revision}
    repository "example/public"
    enabled #false
    checkpoint-every-ms 60000
    replicate-private #false
}}
github-scope "garden" {{
    revision "authored-{revision}"
    repository "R_example" from-ms=100
}}
"#), "amber").unwrap()
}

const SUBJECTS: [&str; 3] = ["objective/garden/throughput", "github-source/garden", "github-scope/garden"];

#[test]
fn native_roots_create_update_prune_reintroduce_and_replay() {
    let amber = Store::open_memory("amber").unwrap();
    let cobalt = Store::open_memory("cobalt").unwrap();
    apply(&amber, &roots(1), 10);
    let initial = amber.owned_sets().unwrap().remove(0);
    assert_eq!(initial.receipt.members.len(), 3);
    for subject in SUBJECTS {
        assert!(initial.receipt.members.contains_key(subject));
    }
    apply(&amber, &roots(2), 11);
    let updated = amber.owned_sets().unwrap().remove(0);
    for subject in SUBJECTS {
        assert_ne!(initial.receipt.members[subject].revision, updated.receipt.members[subject].revision);
    }
    let empty = parse_intent("version 2\n", "amber").unwrap();
    let mut opts = options(&amber, 12);
    opts.allow_empty = true;
    let preview = amber.owned_set_preview(&empty, &opts).unwrap();
    assert!(preview.mass_retirement);
    for subject in SUBJECTS {
        assert_eq!(preview.changes[subject], "retiring");
        assert!(preview.effects[subject].contains("retain historical facts and acquisition authority"));
    }
    opts.expected_subjects = preview.expected_subjects;
    opts.confirm_retire = Some(preview.digest);
    amber.apply_owned_set(&empty, &opts, "prune-native-roots", "person/operator").unwrap();
    share(&amber, &cobalt);
    for store in [&amber, &cobalt] {
        let view = store.owned_sets().unwrap().remove(0);
        assert!(view.blockers.is_empty(), "{:?}", view.blockers);
        assert!(view.receipt.members.is_empty());
        assert_eq!(view.receipt.retired.len(), 3);
        for subject in SUBJECTS {
            let desired = store.desired_subjects().unwrap().into_iter().find(|d| d.subject == subject).unwrap();
            assert_eq!(desired.kind, "stop");
            assert_eq!(desired.desired["stop"], subject);
            assert_eq!(store.claims_for(subject, Some("intent.desired")).unwrap().len(), 3);
        }
        store.connection.batched(replay_graph_from_nothing_tx).unwrap().unwrap();
        let view = store.owned_sets().unwrap().remove(0);
        assert_eq!(view.receipt.retired.len(), 3);
        for subject in SUBJECTS {
            assert_eq!(store.desired_subjects().unwrap().into_iter().find(|d| d.subject == subject).unwrap().kind, "stop");
        }
    }
    apply(&amber, &roots(2), 13);
    let view = amber.owned_sets().unwrap().remove(0);
    assert_eq!(view.receipt.members.len(), 3);
    assert!(view.receipt.retired.is_empty());
}

#[test]
fn native_root_membership_requires_every_members_capability() {
    let stores = signed_fleet();
    let amber = &stores[0];
    let opts = options(amber, 10);
    let preview = amber.owned_set_preview(&roots(1), &opts).unwrap();
    for host in ["amber", "cobalt", "ivory"] {
        assert!(preview.blockers.iter().any(|b| b.contains(&format!("host/{host}")) && b.contains("native root support")));
    }
    for store in &stores {
        store.append_claim(&ClaimInput {
            subject: format!("daemon/{}", store.origin), kind: "daemon.started".into(), actor: None,
            fields: serde_json::from_value(json!({"status":"running","features":{"owned_sets":1,"owned_set_native_roots":1}})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    for from in &stores {
        for to in &stores {
            if from.origin != to.origin { signed_share(from, to); }
        }
    }
    assert!(amber.owned_set_preview(&roots(1), &opts).unwrap().blockers.is_empty());
    apply(amber, &roots(1), 10);
    signed_share(amber, &stores[1]);
    assert_eq!(stores[1].owned_sets().unwrap().remove(0).receipt.members.len(), 3);
}

#[test]
fn native_root_stops_are_root_only_and_other_kinds_still_refused() {
    for (subject, error) in [
        ("objective/garden/throughput", "objective-inside-mission"),
        ("github-source/garden", "github-configuration-inside-mission"),
        ("github-scope/garden", "github-configuration-inside-mission"),
    ] {
        let kdl = format!("version 2\nstop {subject:?}\n");
        assert!(parse_intent(&kdl, "amber").is_ok());
        assert_eq!(fractal_attention::graph::parse_execution_intent(&kdl, "amber", "run-one").unwrap_err().code, error);
    }
    let store = Store::open_memory("amber").unwrap();
    for kdl in [
        "version 2\nperson \"operator\" {}\n",
        "version 2\nexec \"standalone\" { command \"true\" }\n",
    ] {
        let input = parse_intent(kdl, "amber").unwrap();
        assert_eq!(store.owned_set_preview(&input, &options(&store, 10)).unwrap_err().code, "unsupported-set-member");
    }
    assert_eq!(parse_intent("version 2\nstop \"person/operator\"", "amber").unwrap_err().code, "invalid-stop-subject");
}
