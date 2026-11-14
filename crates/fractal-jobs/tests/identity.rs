use fractal_jobs::identity::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn fixture() -> Value {
    json!({
        "source": {"kind": "git", "repo": "repo/é", "commit": {
            "algorithm": "sha1", "digest": vec![1; 20]
        }},
        "environment": {"kind": "exact", "value": {"Z": "最後", "A": "é"}},
        "command": {"argv": ["printf", "é", ""], "cwd": "."},
        "resources": {
            "cpu_millicores": 1, "memory_bytes": 2, "io_weight": 3,
            "tasks_max": 4, "expected_duration_ms": 5, "run_timeout_ms": null,
            "priority": 0, "requested_class": "configured",
            "storage_request": null, "delegated_reservations": null
        }
    })
}
fn request(value: Value) -> ExecutionRequest {
    serde_json::from_value(value).unwrap()
}

#[test]
fn canonical_fixture_is_stable() {
    let original = request(fixture());
    let reordered = serde_json::from_str::<ExecutionRequest>(
        r#"{"command":{"cwd":".","argv":["printf","\u00e9",""]},"resources":{"delegated_reservations":null,"storage_request":null,"requested_class":"configured","priority":0,"run_timeout_ms":null,"expected_duration_ms":5,"tasks_max":4,"io_weight":3,"memory_bytes":2,"cpu_millicores":1},"environment":{"value":{"A":"\u00e9","Z":"最後"},"kind":"exact"},"source":{"commit":{"digest":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"algorithm":"sha1"},"repo":"repo/é","kind":"git"}}"#,
    ).unwrap();
    let expected = concat!(
        "fractal-jobs-request-v1\n",
        r#"{"source":{"kind":"git","repo":"repo/é","commit":{"algorithm":"sha1","digest":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]}},"environment":{"kind":"exact","value":{"A":"é","Z":"最後"}},"command":{"argv":["printf","é",""],"cwd":"."},"resources":{"cpu_millicores":1,"memory_bytes":2,"io_weight":3,"tasks_max":4,"expected_duration_ms":5,"run_timeout_ms":null,"priority":0,"requested_class":"configured","storage_request":null,"delegated_reservations":null}}"#
    );
    assert_eq!(original.canonical_bytes(), expected.as_bytes());
    assert_eq!(original.canonical_bytes(), reordered.canonical_bytes());
    assert_eq!(original.digest(), reordered.digest());
    assert_eq!(&original.digest().0[..], &Sha256::digest(expected)[..]);
}

#[test]
fn every_execution_and_resource_input_changes_identity() {
    let base = fixture();
    let digest = request(base.clone()).digest();
    for (pointer, replacement) in [
        ("/source/repo", json!("other")),
        ("/source/commit/digest/0", json!(2)),
        ("/source", json!({"kind":"content","sha256":vec![0;32]})),
        (
            "/source/commit",
            json!({"algorithm":"sha256","digest":vec![1;32]}),
        ),
        ("/environment/value/A", json!("e\u{301}")),
        ("/environment/value", json!({"Z":"最後","B":"é"})),
        ("/environment", json!({"kind":"sha256","value":vec![0;32]})),
        ("/command/argv", json!(["printf", "", "é"])),
        ("/command/argv/1", json!("other")),
        ("/command/cwd", json!("subdir")),
        ("/resources/cpu_millicores", json!(2)),
        ("/resources/memory_bytes", json!(3)),
        ("/resources/io_weight", json!(4)),
        ("/resources/tasks_max", json!(5)),
        ("/resources/expected_duration_ms", json!(6)),
        ("/resources/run_timeout_ms", json!(7)),
        ("/resources/priority", json!(1)),
        ("/resources/requested_class", json!("other")),
        (
            "/resources/storage_request",
            json!({"bytes":1,"purpose":"scratch"}),
        ),
        ("/resources/delegated_reservations", json!("opaque")),
    ] {
        let mut changed = base.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert_ne!(digest, request(changed).digest(), "{pointer}");
    }
}

#[test]
fn cwd_cannot_escape_the_source() {
    for path in ["", "/absolute", "..", "a/../b", "a\0b"] {
        let mut value = fixture();
        value["command"]["cwd"] = json!(path);
        assert!(serde_json::from_value::<ExecutionRequest>(value).is_err());
    }
}

#[test]
fn idempotency_reuses_refuses_or_creates() {
    let key = IdempotencyKey {
        requester: "requester".into(),
        key: "key".into(),
        request_digest: request(fixture()).digest(),
    };
    assert_eq!(
        decide_idempotency::<&str>(&key, None),
        Ok(IdempotencyDecision::NewJob)
    );
    assert_eq!(
        decide_idempotency(&key, Some((key.request_digest, "job/1"))),
        Ok(IdempotencyDecision::ExistingJob("job/1"))
    );
    let refusal = decide_idempotency(&key, Some((RequestDigest([0; 32]), "job/1"))).unwrap_err();
    assert_eq!(refusal, IdempotencyRefusal::IdempotencyDigestMismatch);
    assert_eq!(
        serde_json::to_value(refusal).unwrap(),
        json!({"reason":"idempotency-digest-mismatch"})
    );
}

#[test]
fn command_requires_an_executable_but_allows_empty_arguments() {
    let mut command = request(fixture()).command;
    assert_eq!(command.validate(), Ok(()));
    command.argv.clear();
    assert_eq!(command.validate(), Err(CommandError::EmptyArgv));
    command.argv.push(String::new());
    assert_eq!(command.validate(), Err(CommandError::EmptyExecutable));
    command.argv[0] = "printf".into();
    command.argv.push(String::new());
    assert_eq!(command.validate(), Ok(()));
}
