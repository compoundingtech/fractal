use super::*;
use super::super::tests::{input, state};

fn config() -> JobsConfig {
    JobsConfig {
        enabled: true,
        allowed_classes: vec!["heavy".into()],
        keeper_cgroups: vec!["/keeper".into()],
    }
}

fn keeper(root: &FsPath) {
    std::fs::create_dir(root.join("keeper")).unwrap();
    std::fs::write(root.join("keeper/cgroup.events"), "populated 1\nfrozen 0\n").unwrap();
}

#[test]
fn kernel_membership_maps_to_the_hosts_canonical_job_generation() {
    let root = tempfile::tempdir().unwrap();
    keeper(root.path());
    let state = state(root.path());
    let config = config();
    let session = client_v0::ClientSession::for_tests_scoped(
        "person/a", "person/a", &["control.work"],
    );
    let response = submit_recording(state.clone(), session, &config, input("outer")).unwrap();
    let id = response.0["job"]["id"].as_str().unwrap();
    let row = own_job(&state, "person/a", id).unwrap();
    let intent = fractal_jobs::launch::LaunchIntent {
        generation: fractal_jobs::launch::ExecutionGeneration(
            std::num::NonZeroU64::new(1).unwrap(),
        ),
        outcome_file: "/fixture/outcome.json".into(),
    };
    let name = fractal_jobs::launch::cgroup_name(id, &intent);
    for record in [
        JobRecord::LaunchIntent { binding: row.binding.clone(), intent },
        JobRecord::Transition { binding: row.binding, event: fractal_jobs::Event::Started },
    ] {
        state.store.append_claim(&ClaimInput {
            subject: id.into(), kind: projection::KIND.into(), actor: Some("person/a".into()),
            fields: BTreeMap::from([("record".into(), serde_json::to_value(record).unwrap())]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    let lookup = |name: &str| read(&state, |tx| projection::job_for_cgroup(tx, "node", name));
    let error = check(&config, &format!("0::/keeper/{name}/child\n"), root.path(), lookup)
        .unwrap_err();
    assert_eq!(error.code, "recursive-heavy-job");
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(error.details["job_id"], id);
    assert!(read(&state, |tx| projection::job_for_cgroup(tx, "other-host", &name))
        .unwrap().is_none());
    let error = check(&config, "0::/keeper/unknown\n", root.path(), lookup).unwrap_err();
    assert_eq!(error.code, "guard-unavailable");
}

#[test]
fn boundaries_require_readable_explicit_roots_and_unambiguous_membership() {
    let root = tempfile::tempdir().unwrap();
    keeper(root.path());
    let config = config();
    for membership in ["0::/keeper/idle\n", "0::/keeper-other/job\n", "0::/\n"] {
        check(&config, membership, root.path(), |_| panic!("outside a job")).unwrap();
    }
    for membership in ["", "0::relative", "0::/keeper\n", "0::/a\n0::/b\n"] {
        assert_eq!(check(&config, membership, root.path(), |_| Ok(None))
            .unwrap_err().code, "guard-unavailable");
    }
    let mut missing = config.clone();
    missing.keeper_cgroups.clear();
    assert_eq!(check(&missing, "0::/", root.path(), |_| Ok(None))
        .unwrap_err().code, "guard-unavailable");
    std::fs::remove_file(root.path().join("keeper/cgroup.events")).unwrap();
    assert_eq!(check(&config, "0::/", root.path(), |_| Ok(None))
        .unwrap_err().code, "guard-unavailable");
}

#[tokio::test]
async fn real_router_never_accepts_request_fields_as_a_peer_witness() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let app = super::super::super::router(state.clone()).layer(Extension(config()));
    let mut value = input("forged-peer");
    value["request"]["command"]["argv"] = json!(["true", "--outside-heavy-job"]);
    value["request"]["environment"]["value"] = json!({"FRACTAL_JOB_ID":"outside"});
    value["request"]["resources"]["priority"] = json!("normal");
    let request = Request::builder().method("POST").uri("/v1/client/jobs")
        .header(client_v0::LOCAL_PERSON_HEADER, "person/a")
        .header("content-type", "application/json")
        .body(Body::from(value.to_string())).unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "guard-unavailable");
    assert!(read(&state, |tx| projection::jobs_for_requester(tx, "person/a"))
        .unwrap().is_empty());
}
