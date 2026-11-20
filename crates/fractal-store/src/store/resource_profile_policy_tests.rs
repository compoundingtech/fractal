use super::resource_membership::{
    Admission, Coverage, Fit, Grant, LaneRequest, Owner, RequestClass,
};
use super::Store;
use fractal_runtime::{
    Capacity, ExecRuntime, ResourceLearningCoverage, ResourceProfileEstimate, ResourceProfileSamples,
};

const HOST: &str = "host-a";
const GIB: u64 = 1 << 30;

fn request(id: &str, memory: u64, revision: u64) -> LaneRequest {
    LaneRequest {
        id: id.into(),
        name: "executor/check".into(),
        host: HOST.into(),
        max_members: 3,
        capacity: Some(Capacity { cores: 1, memory }),
        coverage: Coverage::Logical,
        admission_timeout_ms: None,
        owner: Owner {
            run: format!("mission-run/{id}"),
            step: format!("step-run/{id}"),
            attempt: 1,
            invocation: Some(format!("invocation/{id}")),
        },
        class: RequestClass::Check,
        disk: None,
        io: None,
        profile: Some("check-native".into()),
        profile_estimate: Some(ResourceProfileEstimate {
            seed_revision: Some(format!("seed-{revision}")),
            learned_revision: Some(revision),
            samples: ResourceProfileSamples { memory: revision, disk: 0 },
            memory_high_bytes: Some(memory / 2),
            source: Some("declared-provisional".into()),
            reason: Some(format!("synthetic estimate revision {revision}")),
            learning_coverage: ResourceLearningCoverage::ClientOnly,
        }),
        disk_path: None,
    }
}

fn fit() -> Fit {
    Fit { room: Some(Capacity { cores: 32, memory: 64 * GIB }), ..Fit::default() }
}

fn fixture() -> Store {
    let store = Store::open_memory(HOST).unwrap();
    store.start_resource_authority(HOST, "epoch-a").unwrap();
    store
}

fn admit(store: &Store, request: &LaneRequest, now: u64) -> Grant {
    match store.request_resource_membership(request, fit(), now).unwrap() {
        Admission::Admitted(grant) => grant,
        other => panic!("expected independent membership admission: {other:?}"),
    }
}

#[test]
fn keyed_lane_policy_excludes_estimate_bytes_revisions_and_provenance() {
    let store = fixture();
    let root = tempfile::tempdir().unwrap();
    let runtime = ExecRuntime::new(root.path().join("runtime"), root.path().join("logs"));
    let initial = request("initial", GIB, 1);
    let current = admit(&store, &initial, 1);
    assert!(store.retire_resource_membership(&current, &initial.owner, &runtime, 2).unwrap());

    let mut changed = request("changed", 4 * GIB, 2);
    changed.capacity.as_mut().unwrap().cores = 2;
    let estimate = changed.profile_estimate.as_mut().unwrap();
    estimate.samples.disk = 7;
    estimate.source = Some("learned".into());
    estimate.reason = Some("new measurements supersede the provisional estimate".into());
    estimate.learning_coverage = ResourceLearningCoverage::Full;
    let next = admit(&store, &changed, 3);

    assert_eq!(store.resource_membership_for_grant(&current).unwrap().request, initial);
    assert_eq!(store.resource_membership_for_grant(&next).unwrap().request, changed);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), 4 * GIB);
}

#[test]
fn same_key_memberships_keep_distinct_snapshots_and_independent_custody() {
    let store = fixture();
    let first = request("first", GIB, 1);
    let second = request("second", 3 * GIB, 2);
    let first_grant = admit(&store, &first, 1);
    let second_grant = admit(&store, &second, 2);

    assert_ne!(first_grant.membership_id, second_grant.membership_id);
    assert_ne!(first_grant.generation, second_grant.generation);
    assert_eq!(store.resource_membership_for_grant(&first_grant).unwrap().request, first);
    assert_eq!(store.resource_membership_for_grant(&second_grant).unwrap().request, second);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), 4 * GIB);
    assert_eq!(store.resource_lane_aggregate(HOST, &first.name).unwrap(),
        Some(Capacity { cores: 2, memory: 4 * GIB }));
    assert_eq!(store.held_resource_membership_bytes(HOST, &first.id).unwrap(), 3 * GIB);
    assert_eq!(store.held_resource_membership_bytes(HOST, &second.id).unwrap(), GIB);

    store.cancel_resource_membership(&first_grant, &first.owner, 3).unwrap();
    assert_eq!(store.resource_membership_for_grant(&first_grant).unwrap().state, "retiring");
    assert!(matches!(store.resource_membership(&second.id, &second.owner, 3).unwrap(),
        Admission::Admitted(grant) if grant == second_grant));
    assert_eq!(store.resource_membership_for_grant(&second_grant).unwrap().request, second);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), 4 * GIB);
}

#[test]
fn aggregate_bytes_sum_live_snapshots_including_retiring_and_uncertain() {
    let store = fixture();
    let root = tempfile::tempdir().unwrap();
    let runtime = ExecRuntime::new(root.path().join("runtime"), root.path().join("logs"));
    let retired = request("retired", 8 * GIB, 1);
    let retired_grant = admit(&store, &retired, 1);
    assert!(store.retire_resource_membership(&retired_grant, &retired.owner, &runtime, 2).unwrap());

    let admitted = request("admitted", GIB, 2);
    let retiring = request("retiring", 2 * GIB, 3);
    let uncertain = request("uncertain", 4 * GIB, 4);
    admit(&store, &admitted, 3);
    let retiring_grant = admit(&store, &retiring, 4);
    let uncertain_grant = admit(&store, &uncertain, 5);
    store.cancel_resource_membership(&retiring_grant, &retiring.owner, 6).unwrap();
    store.prepare_resource_membership(&uncertain_grant, "unknown-preparation").unwrap();
    assert!(!store.retire_resource_membership(&uncertain_grant, &uncertain.owner, &runtime, 7).unwrap());

    let queued = request("queued", 16 * GIB, 5);
    assert!(matches!(store.request_resource_membership(&queued, fit(), 8).unwrap(),
        Admission::Queued { .. }));
    let members = store.resource_memberships_for_host(HOST).unwrap();
    for (id, state) in [
        (admitted.id.as_str(), "admitted"),
        (retiring.id.as_str(), "retiring"),
        (uncertain.id.as_str(), "uncertain"),
        (queued.id.as_str(), "queued"),
        (retired.id.as_str(), "retired"),
    ] {
        let member = members.iter().find(|member| member.request.id == id).unwrap();
        assert_eq!(member.state, state);
    }
    let live_sum: u64 = members.iter()
        .filter(|member| matches!(member.state.as_str(), "admitted" | "launched" | "retiring" | "uncertain"))
        .map(|member| member.request.capacity.unwrap().memory)
        .sum();
    assert_eq!(live_sum, 7 * GIB);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), live_sum);
    assert_eq!(store.resource_lane_aggregate(HOST, &admitted.name).unwrap(),
        Some(Capacity { cores: 3, memory: live_sum }));
    assert_eq!(store.resource_lane_aggregate(HOST, "executor/other").unwrap(), None);
    assert_eq!(store.held_resource_membership_bytes(HOST, &uncertain.id).unwrap(), 3 * GIB);
    assert_eq!(store.resource_membership_for_grant(&uncertain_grant).unwrap().request, uncertain);
}

#[test]
fn different_profile_key_conflicts_even_when_estimates_are_identical() {
    let store = fixture();
    let first = request("first", GIB, 1);
    let first_grant = admit(&store, &first, 1);
    let mut conflicting = request("conflicting", GIB, 1);
    conflicting.profile = Some("check-other".into());
    let error = store.request_resource_membership(&conflicting, fit(), 2).unwrap_err();
    assert!(error.to_string().contains("resource lane policy differs"), "{error:#}");
    assert!(store.resource_membership_record(&conflicting.id, &conflicting.owner).is_err());
    assert_eq!(store.resource_membership_for_grant(&first_grant).unwrap().request, first);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), GIB);
}

#[test]
fn keyed_lane_policy_still_binds_member_limit_and_coverage() {
    let store = fixture();
    let first = request("first", GIB, 1);
    admit(&store, &first, 1);
    let mut changed_limit = request("changed-limit", GIB, 1);
    changed_limit.max_members += 1;
    let mut changed_coverage = request("changed-coverage", GIB, 1);
    changed_coverage.coverage = Coverage::Enforced;
    for changed in [changed_limit, changed_coverage] {
        let error = store.request_resource_membership(&changed, fit(), 2).unwrap_err();
        assert!(error.to_string().contains("resource lane policy differs"), "{error:#}");
    }
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), GIB);
}

#[test]
fn historical_tuple_policy_and_membership_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.sqlite");
    let runtime = ExecRuntime::new(root.path().join("runtime"), root.path().join("logs"));
    let mut historical = request("historical", GIB, 1);
    historical.profile = None;
    historical.profile_estimate = None;
    let historical_grant;
    {
        let store = Store::open(&path, HOST).unwrap();
        store.start_resource_authority(HOST, "epoch-a").unwrap();
        historical_grant = admit(&store, &historical, 1);
    }

    let store = Store::open(&path, HOST).unwrap();
    assert_eq!(store.resource_membership_for_grant(&historical_grant).unwrap().request, historical);
    assert_eq!(admit(&store, &historical, 2), historical_grant);
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), GIB);
    assert!(store.retire_resource_membership(&historical_grant, &historical.owner, &runtime, 3).unwrap());

    let mut changed_tuple = historical.clone();
    changed_tuple.id = "changed-tuple".into();
    changed_tuple.owner = request("changed-tuple", GIB, 1).owner;
    changed_tuple.capacity.as_mut().unwrap().memory = 2 * GIB;
    let error = store.request_resource_membership(&changed_tuple, fit(), 4).unwrap_err();
    assert!(error.to_string().contains("resource lane policy differs"), "{error:#}");

    let mut keyed = request("keyed", GIB, 1);
    keyed.name = historical.name.clone();
    let error = store.request_resource_membership(&keyed, fit(), 5).unwrap_err();
    assert!(error.to_string().contains("resource lane policy differs"), "{error:#}");

    let mut matching_tuple = historical.clone();
    matching_tuple.id = "matching-tuple".into();
    matching_tuple.owner = request("matching-tuple", GIB, 1).owner;
    let next = admit(&store, &matching_tuple, 6);
    assert_eq!(store.resource_membership_for_grant(&next).unwrap().request, matching_tuple);
    assert_eq!(store.resource_membership_for_grant(&historical_grant).unwrap().state, "retired");
    assert_eq!(store.held_resource_membership_bytes(HOST, "").unwrap(), GIB);
}
