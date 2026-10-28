use super::*;
use serde_json::json;

const SEAT: &str = "agent/node.orchid";

fn setup() -> (
    Arc<Store>,
    Arc<FakeRuntime>,
    Reconciler<FakeRuntime>,
    DesiredSubject,
) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        "version 2\nagent \"orchid\" { workspace \"/tmp\"; restart \"never\"; harness \"claude\" {} }",
        "channel-recovery",
    );
    let desired = store.desired_subjects().unwrap().remove(0);
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    );
    (store, runtime, reconciler, desired)
}

fn attachment(store: &Store, incarnation: &str, attached: bool) -> u128 {
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.observed".into(),
            actor: Some(SEAT.into()),
            fields: BTreeMap::from([
                ("status".into(), json!("running")),
                ("runtime_id".into(), json!("node.orchid")),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "harness.observed".into(),
            actor: Some(SEAT.into()),
            fields: BTreeMap::from([
                ("state".into(), json!("idle")),
                ("driver".into(), json!("claude")),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "harness.diagnostic".into(),
            actor: Some(SEAT.into()),
            fields: BTreeMap::from([
                (
                    "code".into(),
                    json!(if attached {
                        "claude-channel-attached"
                    } else {
                        "claude-channel-unattached"
                    }),
                ),
                (
                    "status".into(),
                    json!(if attached { "recovered" } else { "blocked" }),
                ),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .accepted_at_unix_ms
}

fn recover(
    reconciler: &Reconciler<FakeRuntime>,
    desired: &DesiredSubject,
    incarnation: &str,
    at: u128,
) -> Result<bool> {
    reconciler.reconcile_claude_channel_recovery(
        desired,
        desired.member.as_ref().unwrap(),
        Some(&claude_seat_pty("orchid", "running", incarnation)),
        None,
        at,
    )
}

fn attempts(store: &Store) -> Vec<crate::model::ClaimRecord> {
    store
        .claims_for(SEAT, Some("runtime.action.requested"))
        .unwrap()
        .into_iter()
        .filter(|claim| {
            claim.body["fields"]["operation"] == super::super::channel_recovery::POLICY_OPERATION
        })
        .collect()
}

const POLICY_BANNER: &str = "--dangerously-load-development-channels blocked by org policy (plugin:st-channel@st)\nInbound messages will be silently dropped\nHave an administrator set channelsEnabled: true in managed settings to enable";

fn policy_launch(store: &Store, incarnation: &str) {
    let token = store.selected_desired_token(SEAT).unwrap().unwrap();
    store.append_claim(&ClaimInput {
        subject: SEAT.into(), kind: "runtime.action.succeeded".into(), actor: None,
        fields: BTreeMap::from([
            ("action".into(), json!("start")),
            ("desired_token".into(), json!(token)),
            ("incarnation_id".into(), json!(incarnation)),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[test]
fn channel_policy_refusal_stops_without_spending_retries_and_survives_daemon_restart() {
    let (store, runtime, reconciler, mut desired) = setup();
    desired.member.as_mut().unwrap().restart = RestartType::Always;
    let since = attachment(&store, "orchid-one", false);
    policy_launch(&store, "orchid-one");
    *runtime.screen.lock().unwrap() = format!("Claude Code v2.1.306\n{POLICY_BANNER}\n❯ ");
    let error = recover(&reconciler, &desired, "orchid-one", since).unwrap_err();
    assert!(error.to_string().contains("channelsEnabled: true"), "{error}");
    reconciler.record_member_reconcile_result(SEAT, Err(error)).unwrap();
    assert!(store.member_reconcile_fault(SEAT, None).unwrap().unwrap().contains("channelsEnabled"));
    assert_eq!(*runtime.stops.lock().unwrap(), ["node.orchid"]);
    assert!(attempts(&store).is_empty());
    let fault = store.latest_observation(SEAT, "runtime.action.failed").unwrap().unwrap();
    assert_eq!(fault.body["fields"]["action"], "claude-channel-policy-blocked");
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    // No screen or running process is needed to retain the permanent failure.
    runtime.screen.lock().unwrap().clear();
    assert!(restarted.reconcile_claude_channel_recovery(
        &desired, desired.member.as_ref().unwrap(), None, None, since + 86_400_000,
    ).unwrap_err().to_string().contains("channelsEnabled"));
    assert!(attempts(&store).is_empty());
    store.append_claim(&ClaimInput {
        subject: SEAT.into(), kind: "runtime.action.requested".into(),
        actor: Some(SEAT.into()),
        fields: BTreeMap::from([("action".into(), json!("restart"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!restarted.reconcile_claude_channel_recovery(
        &desired, desired.member.as_ref().unwrap(), None, None, since + 86_400_001,
    ).unwrap());
}

#[test]
fn channel_policy_banner_does_not_stop_an_attached_or_stale_incarnation() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    policy_launch(&store, "orchid-one");
    *runtime.screen.lock().unwrap() = POLICY_BANNER.into();
    assert!(!recover(&reconciler, &desired, "predecessor", since).unwrap());
    attachment(&store, "orchid-one", true);
    assert!(!recover(&reconciler, &desired, "orchid-one", since).unwrap());
    assert!(runtime.stops.lock().unwrap().is_empty());
}

#[test]
fn quoted_channel_policy_banner_is_not_a_provider_refusal() {
    for screen in [
        format!("❯ Please explain this error:\n> {}", POLICY_BANNER.replace('\n', "\n> ")),
        format!("❯ Please explain this error:\n```\n{POLICY_BANNER}\n```"),
        format!("❯ {POLICY_BANNER}"),
        format!("❯ Please explain this error:\n{POLICY_BANNER}"),
        format!("● This is the error you quoted:\n{POLICY_BANNER}"),
        format!("```\n{POLICY_BANNER}\n```"),
    ] {
        let (store, runtime, reconciler, desired) = setup();
        let since = attachment(&store, "orchid-one", false);
        policy_launch(&store, "orchid-one");
        *runtime.screen.lock().unwrap() = screen;
        assert!(!recover(&reconciler, &desired, "orchid-one", since).unwrap());
        assert!(runtime.stops.lock().unwrap().is_empty());
        assert!(store.claims_for(SEAT, Some("runtime.action.failed")).unwrap().is_empty());
    }
}

// The quotation in #3999 is one line and includes a period after "dropped".
const REPORTED_POLICY_BANNER: &str = /* sqlite-large-value: allow-for-now lv-h72967be8c2d9 owner=fractal_reconcile::reconcile::tests::channel_recovery TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-drivers/tests/fixtures/blocking-screens/claude-channel-policy-reported.txt");

#[test]
fn verbatim_and_wrapped_policy_banners_stop_only_in_provider_startup_chrome() {
    for source in [POLICY_BANNER, REPORTED_POLICY_BANNER] {
        for width in [usize::MAX, 80, 60, 40, 20] {
            // Both fixture sources are ASCII. A PTY soft-wrap may split a word at
            // the right edge, whereas Claude can also wrap at a space.
            let wrapped = source.lines()
                .flat_map(|line| line.as_bytes().chunks(width))
                .map(|line| std::str::from_utf8(line).unwrap())
                .collect::<Vec<_>>().join("\n");
            for (screen, refusal) in [
                (format!("Claude Code v2.1.306\n{wrapped}\n❯ "), true),
                (format!("❯ Explain this error:\n{wrapped}"), false),
                (format!("● You quoted:\n{wrapped}"), false),
                (format!("> {}", wrapped.replace('\n', "\n> ")), false),
                (format!("```\n{wrapped}\n```"), false),
            ] {
                let (store, runtime, reconciler, desired) = setup();
                let since = attachment(&store, "orchid-one", false);
                policy_launch(&store, "orchid-one");
                *runtime.screen.lock().unwrap() = screen;
                assert_eq!(
                    recover(&reconciler, &desired, "orchid-one", since).is_err(),
                    refusal, "width={width}, source={source:?}",
                );
                assert_eq!(runtime.stops.lock().unwrap().len(), usize::from(refusal));
                assert!(attempts(&store).is_empty());
            }
        }
    }
}

#[test]
fn predecessor_policy_banner_does_not_fence_the_replacement_declaration() {
    let (store, runtime, reconciler, desired) = setup();
    reconciler.reconcile_once().unwrap();
    let initial_starts = runtime.starts.lock().unwrap().len();
    let since = attachment(&store, "orchid-one", false);
    policy_launch(&store, "orchid-one");
    *runtime.screen.lock().unwrap() = POLICY_BANNER.into();
    apply_source(
        &store,
        "version 2\nagent \"orchid\" { workspace \"/var/tmp\"; restart \"always\"; harness \"claude\" {} }",
        "policy-replacement",
    );
    let replacement = store.desired_subjects().unwrap().remove(0);
    assert_ne!(desired.member.as_ref().unwrap().workspace, replacement.member.as_ref().unwrap().workspace);
    assert!(!recover(&reconciler, &replacement, "orchid-one", since).unwrap());
    assert!(store.claims_for(SEAT, Some("runtime.action.failed")).unwrap().is_empty());
    // The normal launch-change path remains authoritative; no policy stop or fence is minted.
    assert!(reconciler.declared_launch_changes(
        &replacement, &claude_seat_pty("orchid", "running", "orchid-one"),
    ).unwrap().is_some());
    assert!(runtime.stops.lock().unwrap().is_empty());
    *runtime.ptys.lock().unwrap() = vec![claude_seat_pty("orchid", "running", "orchid-one")];
    reconciler.reconcile_once().unwrap();
    assert_eq!(*runtime.stops.lock().unwrap(), ["node.orchid"]);
    *runtime.ptys.lock().unwrap() = vec![claude_seat_pty("orchid", "exited", "orchid-one")];
    reconciler.reconcile_once().unwrap();
    assert_eq!(runtime.starts.lock().unwrap().len(), initial_starts + 1);
}

#[test]
fn channel_recovery_rechecks_then_restarts_once_and_ignores_a_stale_incarnation() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    let (result, deadline) =
        smallclaims::touched::record_due(|| recover(&reconciler, &desired, "orchid-one", since));
    assert!(!result.unwrap());
    assert_eq!(deadline, Some(since + 10_000));
    assert!(attempts(&store).is_empty());
    assert!(!recover(&reconciler, &desired, "predecessor", since + 10_000).unwrap());
    assert!(recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap());
    assert!(!recover(&reconciler, &desired, "orchid-one", since + 20_000).unwrap());
    assert_eq!(attempts(&store).len(), 1);
    let request = &attempts(&store)[0];
    assert!(
        request.body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("attempt 1 of 3")
    );
    assert_eq!(
        request.body["evidence"][0],
        store.selected_desired_token(SEAT).unwrap().unwrap()
    );
    // The regular restart path stops this exact seat even under restart "never".
    assert!(
        reconciler
            .reconcile_requested_restart(
                &desired,
                desired.member.as_ref().unwrap(),
                Some(&claude_seat_pty("orchid", "running", "orchid-one")),
                None
            )
            .unwrap()
    );
    assert_eq!(*runtime.stops.lock().unwrap(), ["node.orchid"]);
    let restarted = Reconciler::new(
        store,
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    );
    assert!(!recover(&restarted, &desired, "orchid-one", since + 30_000).unwrap());
}

#[test]
fn channel_initialized_during_recheck_keeps_its_incarnation() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    assert!(!recover(&reconciler, &desired, "orchid-one", since).unwrap());
    attachment(&store, "orchid-one", true);
    assert!(!recover(&reconciler, &desired, "orchid-one", since + 30_000).unwrap());
    assert!(attempts(&store).is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());
}

#[test]
fn channel_initialization_can_cancel_a_recorded_recovery_before_the_stop() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    assert!(recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap());
    attachment(&store, "orchid-one", true);
    assert!(!recover(&reconciler, &desired, "orchid-one", since + 10_001).unwrap());
    assert!(
        !reconciler
            .reconcile_requested_restart(
                &desired,
                desired.member.as_ref().unwrap(),
                Some(&claude_seat_pty("orchid", "running", "orchid-one")),
                None
            )
            .unwrap()
    );
    assert!(runtime.stops.lock().unwrap().is_empty());
    assert_eq!(attempts(&store).len(), 1);
}

#[test]
fn recovery_success_requires_channel_attachment_even_when_hooks_are_ready() {
    let (store, _, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap();
    let request = attempts(&store).pop().unwrap();
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.action.requested".into(),
            actor: request.actor.clone(),
            fields: BTreeMap::from([("action".into(), json!("start"))]),
            evidence: vec![request.id.clone()],
            expected_subject: None,
            idempotency_key: Some(format!("agent-restart-attempt:{}", request.id)),
        })
        .unwrap();
    for (kind, fields) in [
        (
            "runtime.observed",
            BTreeMap::from([
                ("status".into(), json!("running")),
                ("runtime_id".into(), json!("node.orchid")),
                ("incarnation_id".into(), json!("orchid-two")),
            ]),
        ),
        (
            "harness.observed",
            BTreeMap::from([
                ("state".into(), json!("idle")),
                ("driver".into(), json!("claude")),
                ("incarnation_id".into(), json!("orchid-two")),
            ]),
        ),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: kind.into(),
                actor: Some(SEAT.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let replacement = claude_seat_pty("orchid", "running", "orchid-two");
    assert!(store.current_harness(SEAT).unwrap().unwrap().is_ready());
    assert!(
        !reconciler
            .reconcile_requested_restart(
                &desired,
                desired.member.as_ref().unwrap(),
                Some(&replacement),
                None
            )
            .unwrap()
    );
    let completion = format!("agent-restart-completed:{}", request.id);
    assert!(store.operation_claim(&completion).unwrap().is_none());
    attachment(&store, "orchid-two", true);
    reconciler
        .reconcile_requested_restart(
            &desired,
            desired.member.as_ref().unwrap(),
            Some(&replacement),
            None,
        )
        .unwrap();
    assert_eq!(
        store.operation_claim(&completion).unwrap().unwrap().kind,
        "runtime.action.succeeded"
    );
}

#[test]
fn channel_recovery_budget_and_parking_survive_daemon_restarts() {
    let (store, runtime, reconciler, desired) = setup();
    for (index, backoff) in [10_000, 20_000, 40_000].into_iter().enumerate() {
        let incarnation = format!("orchid-{index}");
        let since = attachment(&store, &incarnation, false);
        let fresh = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        assert!(!recover(&fresh, &desired, &incarnation, since + backoff - 1).unwrap());
        assert!(recover(&fresh, &desired, &incarnation, since + backoff).unwrap());
        assert_eq!(attempts(&store).len(), index + 1);
    }
    let since = attachment(&store, "orchid-last", false);
    assert!(recover(&reconciler, &desired, "orchid-last", since).unwrap());
    let fresh = Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    );
    let failure = recover(&fresh, &desired, "orchid-last", since + 1).unwrap_err();
    assert!(failure.to_string().contains("parked"));
    assert!(failure.to_string().contains("mail remains held"));
    assert_eq!(*runtime.stops.lock().unwrap(), ["node.orchid"]);
    assert_eq!(attempts(&store).len(), 3);
    assert!(
        fresh
            .reconcile_claude_channel_recovery(
                &desired,
                desired.member.as_ref().unwrap(),
                None,
                None,
                since + 100_000
            )
            .is_err()
    );
    assert!(runtime.starts.lock().unwrap().is_empty());

    // An explicit restart reopens a budget without discarding the durable failure evidence.
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.action.requested".into(),
            actor: Some("person/eval".into()),
            fields: BTreeMap::from([
                ("action".into(), json!("restart")),
                ("incarnation_id".into(), json!("orchid-last")),
            ]),
            evidence: vec![store.selected_desired_token(SEAT).unwrap().unwrap()],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let since = attachment(&store, "orchid-explicit", false);
    assert!(recover(&fresh, &desired, "orchid-explicit", since + 10_000).unwrap());
    assert!(
        attempts(&store).last().unwrap().body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("attempt 1 of 3")
    );
}

#[test]
fn a_brief_automatic_recovery_does_not_reset_the_durable_attempt_budget() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap();
    let request = attempts(&store).pop().unwrap();
    attachment(&store, "orchid-two", true);
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.action.succeeded".into(),
            actor: request.actor,
            fields: BTreeMap::from([("action".into(), json!("restart"))]),
            evidence: vec![request.id],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let since = attachment(&store, "orchid-two", false);
    let fresh = Reconciler::new(
        store.clone(),
        runtime,
        "node".into(),
        Arc::new(Notify::new()),
    );
    assert!(!recover(&fresh, &desired, "orchid-two", since + 10_000).unwrap());
    assert!(recover(&fresh, &desired, "orchid-two", since + 20_000).unwrap());
    assert!(
        attempts(&store).last().unwrap().body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("attempt 2 of 3")
    );
}

#[test]
fn channel_recovery_attempts_expire_after_six_hours_without_a_quick_attach_reset() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    assert!(recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap());
    let first = attempts(&store).pop().unwrap();
    // Expiry is based on the retained claim's accepted time, not daemon uptime or attach count.
    let expired = first.accepted_at_unix_ms + 6 * 60 * 60 * 1_000;
    let fresh = Reconciler::new(
        store.clone(),
        runtime,
        "node".into(),
        Arc::new(Notify::new()),
    );
    attachment(&store, "orchid-two", false);
    assert!(recover(&fresh, &desired, "orchid-two", expired).unwrap());
    let requests = attempts(&store);
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("attempt 1 of 3")
    );
    assert_ne!(requests[0].id, requests[1].id);
}

#[test]
fn automatic_replacement_success_cannot_reopen_a_parked_budget() {
    let (store, runtime, reconciler, desired) = setup();
    for (index, backoff) in [10_000, 20_000, 40_000].into_iter().enumerate() {
        let incarnation = format!("orchid-{index}");
        let since = attachment(&store, &incarnation, false);
        assert!(recover(&reconciler, &desired, &incarnation, since + backoff).unwrap());
        let request = attempts(&store).pop().unwrap();
        attachment(&store, &format!("orchid-{}", index + 1), true);
        store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "runtime.action.succeeded".into(),
                actor: request.actor,
                fields: BTreeMap::from([("action".into(), json!("restart"))]),
                evidence: vec![request.id],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let since = attachment(&store, "orchid-last", false);
    assert!(recover(&reconciler, &desired, "orchid-last", since).unwrap());
    let fresh = Reconciler::new(
        store.clone(),
        runtime,
        "node".into(),
        Arc::new(Notify::new()),
    );
    assert!(
        recover(&fresh, &desired, "orchid-last", since + 1)
            .unwrap_err()
            .to_string()
            .contains("parked")
    );
    assert_eq!(attempts(&store).len(), 3);
}

#[test]
fn a_parked_recovery_never_stops_an_unrelated_new_incarnation() {
    let (store, runtime, reconciler, desired) = setup();
    for (i, backoff) in [10_000, 20_000, 40_000].into_iter().enumerate() {
        let incarnation = format!("orchid-{i}");
        let since = attachment(&store, &incarnation, false);
        assert!(recover(&reconciler, &desired, &incarnation, since + backoff).unwrap());
    }
    let since = attachment(&store, "orchid-parked", false);
    assert!(recover(&reconciler, &desired, "orchid-parked", since).unwrap());
    attachment(&store, "orchid-independent", true);
    assert!(recover(&reconciler, &desired, "orchid-independent", since + 1).is_err());
    assert!(runtime.stops.lock().unwrap().is_empty());
}

#[test]
fn same_episode_keys_dedupe_across_two_daemon_recreations() {
    let (store, runtime, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    assert!(recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap());
    let first = attempts(&store).pop().unwrap();
    for _ in 0..2 {
        let fresh = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        assert!(!recover(&fresh, &desired, "orchid-one", since + 10_001).unwrap());
        assert_eq!(attempts(&store).len(), 1);
        assert_eq!(attempts(&store)[0].id, first.id);
    }
    for (index, backoff) in [20_000, 40_000].into_iter().enumerate() {
        let incarnation = format!("orchid-{}", index + 2);
        let since = attachment(&store, &incarnation, false);
        assert!(recover(&reconciler, &desired, &incarnation, since + backoff).unwrap());
    }
    let since = attachment(&store, "orchid-parked", false);
    assert!(recover(&reconciler, &desired, "orchid-parked", since).unwrap());
    let failures = || {
        store
            .claims_for(SEAT, Some("runtime.action.failed"))
            .unwrap()
    };
    let park = failures().pop().unwrap();
    for _ in 0..2 {
        let fresh = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        assert!(
            recover(&fresh, &desired, "orchid-parked", since + 1)
                .unwrap_err()
                .to_string()
                .contains("parked")
        );
        let recorded = failures();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, park.id);
        assert_eq!(attempts(&store).len(), 3);
    }
}

#[test]
fn automatic_and_person_requested_restart_failures_preserve_their_initiator() {
    let (store, _, reconciler, desired) = setup();
    let since = attachment(&store, "orchid-one", false);
    assert!(recover(&reconciler, &desired, "orchid-one", since + 10_000).unwrap());
    let automatic = attempts(&store).pop().unwrap();
    let reason = reconciler
        .record_requested_restart_failure(
            &desired,
            desired.member.as_ref().unwrap(),
            &automatic,
            "fixture refusal",
        )
        .unwrap();
    assert!(reason.starts_with("the automatic Claude channel recovery failed"));
    assert!(!reason.contains("explicit"));
    let manual = store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.action.requested".into(),
            actor: Some("person/eval".into()),
            fields: BTreeMap::from([("action".into(), json!("restart"))]),
            evidence: vec![store.selected_desired_token(SEAT).unwrap().unwrap()],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let reason = reconciler
        .record_requested_restart_failure(
            &desired,
            desired.member.as_ref().unwrap(),
            &manual,
            "fixture refusal",
        )
        .unwrap();
    assert!(reason.starts_with("the explicit restart failed"));
    let failures = store
        .claims_for(SEAT, Some("runtime.action.failed"))
        .unwrap();
    for request in [&automatic, &manual] {
        let failure = failures
            .iter()
            .find(|claim| claim.body["evidence"][0] == request.id)
            .unwrap();
        assert_eq!(failure.actor, request.actor);
        assert_eq!(failure.body["fields"]["action"], "restart");
    }
}

#[test]
fn rolling_expiry_and_explicit_reset_reopen_zero_through_three_attempts() {
    for spent in 0..=3 {
        for explicit in [false, true] {
            let (store, _, reconciler, desired) = setup();
            let mut latest = 0;
            for index in 0..spent {
                let incarnation = format!("before-{index}");
                let since = attachment(&store, &incarnation, false);
                assert!(
                    recover(
                        &reconciler,
                        &desired,
                        &incarnation,
                        since + [10_000, 20_000, 40_000][index]
                    )
                    .unwrap()
                );
                latest = attempts(&store).last().unwrap().accepted_at_unix_ms;
            }
            let incarnation = "after-boundary";
            let since = attachment(&store, incarnation, false);
            if explicit {
                store
                    .append_claim(&ClaimInput {
                        subject: SEAT.into(),
                        kind: "runtime.action.requested".into(),
                        actor: Some("person/eval".into()),
                        fields: BTreeMap::from([("action".into(), json!("restart"))]),
                        evidence: vec![store.selected_desired_token(SEAT).unwrap().unwrap()],
                        expected_subject: None,
                        idempotency_key: None,
                    })
                    .unwrap();
            }
            let at = if explicit || spent == 0 {
                since + 10_000
            } else {
                latest + 6 * 60 * 60 * 1_000
            };
            assert!(
                recover(&reconciler, &desired, incarnation, at).unwrap(),
                "spent={spent} explicit={explicit}"
            );
            let claims = attempts(&store);
            assert_eq!(claims.len(), spent + 1);
            assert!(
                claims.last().unwrap().body["fields"]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("attempt 1 of 3")
            );
        }
    }
}

#[test]
fn adopting_the_window_never_spends_historical_requests_or_uses_their_park_fault() {
    for same_incarnation in [false, true] {
        let (store, runtime, _, desired) = setup();
        let token = store.selected_desired_token(SEAT).unwrap().unwrap();
        let mut old_ids = Vec::new();
        for index in 0..3 {
            let old = store
                .append_claim(&ClaimInput {
                    subject: SEAT.into(),
                    kind: "runtime.action.requested".into(),
                    actor: Some(RECONCILER_ACTOR.into()),
                    fields: BTreeMap::from([
                        ("action".into(), json!("restart")),
                        ("operation".into(), json!("claude-channel-recovery")),
                        (
                            "incarnation_id".into(),
                            json!(if same_incarnation {
                                "new-loss".into()
                            } else {
                                format!("old-{index}")
                            }),
                        ),
                        (
                            "reason".into(),
                            json!("Automatic Claude channel recovery attempt 1 of 3: old policy"),
                        ),
                    ]),
                    evidence: vec![token.clone()],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            old_ids.push(old.id);
        }
        store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "runtime.action.failed".into(),
                actor: Some(RECONCILER_ACTOR.into()),
                fields: BTreeMap::from([
                    ("action".into(), json!("claude-channel-recovery")),
                    ("desired_token".into(), json!(token)),
                    ("incarnation_id".into(), json!("new-loss")),
                    ("reason".into(), json!("historical parked budget")),
                ]),
                evidence: old_ids.clone(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let fresh = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        let since = attachment(&store, "new-loss", false);
        // Full budget before the first due recheck; historical same-token/same-incarnation
        // requests and their park fault do not pre-park this adopted policy.
        assert!(!recover(&fresh, &desired, "new-loss", since).unwrap());
        assert!(attempts(&store).is_empty());
        assert!(runtime.stops.lock().unwrap().is_empty());
        assert!(recover(&fresh, &desired, "new-loss", since + 10_000).unwrap());
        let new = attempts(&store);
        assert_eq!(new.len(), 1);
        assert!(
            new[0].body["fields"]["reason"]
                .as_str()
                .unwrap()
                .contains("attempt 1 of 3")
        );
        assert!(super::super::channel_recovery::automatic_request(&new[0]));
        let history = store
            .claims_for(SEAT, Some("runtime.action.requested"))
            .unwrap();
        assert_eq!(history.len(), 4);
        for id in old_ids {
            let old = history.iter().find(|claim| claim.id == id).unwrap();
            assert!(super::super::channel_recovery::automatic_request(old));
            assert_ne!(
                old.body["fields"]["operation"],
                super::super::channel_recovery::POLICY_OPERATION
            );
        }
        assert_eq!(
            store
                .claims_for(SEAT, Some("runtime.action.failed"))
                .unwrap()
                .len(),
            1,
            "old fault retained, no new park fault"
        );
        // A late old-policy request must not masquerade as an explicit person reset.
        store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "runtime.action.requested".into(),
                actor: Some(RECONCILER_ACTOR.into()),
                fields: BTreeMap::from([
                    ("action".into(), json!("restart")),
                    ("operation".into(), json!("claude-channel-recovery")),
                    ("incarnation_id".into(), json!("old-late")),
                ]),
                evidence: vec![store.selected_desired_token(SEAT).unwrap().unwrap()],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let since = attachment(&store, "next-loss", false);
        assert!(recover(&fresh, &desired, "next-loss", since + 20_000).unwrap());
        let new = attempts(&store);
        assert_eq!(new.len(), 2);
        assert!(
            new[1].body["fields"]["reason"]
                .as_str()
                .unwrap()
                .contains("attempt 2 of 3")
        );
        // A selected declaration change supplies a distinct fence; even stamped requests
        // from the previous token must not consume the replacement declaration's budget.
        let old_token = store.selected_desired_token(SEAT).unwrap().unwrap();
        apply_source(
            &store,
            "version 2\nagent \"orchid\" { workspace \"/tmp/replacement\"; restart \"never\"; harness \"claude\" {} }",
            "channel-recovery-replacement",
        );
        assert_ne!(
            store.selected_desired_token(SEAT).unwrap().unwrap(),
            old_token
        );
        let replacement = store.desired_subjects().unwrap().remove(0);
        let since = attachment(&store, "replacement-loss", false);
        assert!(recover(&fresh, &replacement, "replacement-loss", since + 10_000).unwrap());
        let new = attempts(&store);
        assert_eq!(new.len(), 3);
        assert!(
            new[2].body["fields"]["reason"]
                .as_str()
                .unwrap()
                .contains("attempt 1 of 3")
        );
    }
}

#[test]
fn ownership_launch_choice_comes_only_from_the_install_and_defaults_off() {
    for configured in [None, Some(false), Some(true)] {
        let (_, _, mut reconciler, _) = setup();
        if let Some(enabled) = configured {
            reconciler = reconciler.with_claude_primary_ownership(enabled);
        }
        let key = fractal_drivers::claude_process::ENFORCEMENT_ENV;
        for declared in ["0", "1"] {
            let mut member = BTreeMap::from([(key.into(), declared.into())]);
            overlay_runtime_environment(SEAT, &mut member, &reconciler.runtime_environment);
            assert_eq!(
                member[key],
                if configured == Some(true) { "1" } else { "0" }
            );
        }
    }
}
