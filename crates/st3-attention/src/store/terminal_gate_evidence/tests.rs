#![allow(unused_imports)]
pub use st3_store::store::terminal_gate_evidence::tests::*;
use st3_store::store::terminal_gate_evidence_shared_attention_tests::*;
use super::*;
use serde_json::json;
use smallclaims::ivm::install::{Installer, Limits, Outcome, ScanPage};
use st3_store::store::terminal_gate_evidence_shared_attention_tests::{MEMBERS};
use st3_store::store::terminal_gate_evidence::tests_shared_attention_tests::{encoded};


#[test]
fn dormant_key_refusal_retracts_only_that_key_and_preserves_another_proven_warning() {
    for (path, value) in [
        (["domain", "origin"], json!("node/rival")),
        (
            ["domain", "kinds"],
            json!(["runtime.observed", "agent.output"]),
        ),
        (
            ["domain", "kinds"],
            json!(["runtime.observed", "runtime.action.new-kind"]),
        ),
        (["domain", "complete"], json!(false)),
        (["policy", "version"], json!("unsupported.v9")),
        (["observed", "evidence"], json!(["claim/old-launch"])),
        (["observed", "exit_code"], Value::Null),
        (["generation", "revision"], json!("rev/old")),
        (["desired", "restart"], json!("always")),
        (["gate", "subject"], json!("exec/${ST_ATTEMPT}")),
        (["gate", "operator"], json!("contains")),
        (["gate", "expected"], json!("not-an-integer")),
    ] {
        let mut input = facts();
        input[path[0]][path[1]] = value;
        assert_eq!(
            witness(&encoded(input.clone())).unwrap(),
            Decision::NoWitness,
            "{path:?}"
        );
        let mut fixture = Fixture::new();
        fixture.record(mutation("gate/one", Some(facts())));
        fixture.record(mutation("gate/other", Some(facts())));
        fixture.record(mutation("gate/one", Some(input)));
        let root = fixture.installer.root(&fixture.db, VIEW).unwrap();
        assert!(fixture.installer.status(&fixture.db, VIEW).unwrap().ready);
        assert_eq!(fixture.member_rows(&root.namespace), 1);
        let key: String = fixture
            .db
            .query_row(
                "SELECT key FROM test_terminal_members WHERE namespace=?1",
                [root.namespace.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(key, "gate/other");
    }
}

#[test]
fn dormant_global_capture_encoding_and_storage_failures_still_fence_the_namespace() {
    let mut incomplete = facts();
    incomplete["policy"]["complete"] = json!(false);
    for input in [
        encoded(incomplete),
        Value::String("{\"policy\":}".into()),
        Value::String(" ".repeat(INPUT_BYTES + 1)),
    ] {
        assert!(witness(&input).is_err());
        let mut fixture = Fixture::new();
        fixture.record(mutation("gate/one", Some(facts())));
        fixture.record(mutation("gate/other", Some(facts())));
        let root = fixture.installer.root(&fixture.db, VIEW).unwrap();
        fixture.record(Mutation {
            key: "gate/one".into(),
            old: None,
            new: Some(input),
        });
        assert!(fixture.installer.root(&fixture.db, VIEW).is_err());
        assert!(!fixture.installer.status(&fixture.db, VIEW).unwrap().ready);
        // Saved outputs remain physically present but may not be presented as current.
        assert_eq!(fixture.member_rows(&root.namespace), 2);
    }
    let mut fixture = Fixture::new();
    fixture.record(mutation("gate/one", Some(facts())));
    fixture
        .db
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h21bcd9f5eab6 owner=st3_attention::store::terminal_gate_evidence::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE test_terminal_members")
        .unwrap();
    fixture.record(mutation("gate/other", Some(facts())));
    assert!(fixture.installer.root(&fixture.db, VIEW).is_err());
    assert!(!fixture.installer.status(&fixture.db, VIEW).unwrap().ready);
}

#[test]
fn dormant_membership_is_physically_capped_and_overflow_fences_without_dropping_witnesses() {
    let mut fixture = Fixture::new();
    for n in 0..MEMBERS {
        fixture.record(mutation(&format!("gate/{n:03}"), Some(facts())));
    }
    let root = fixture.installer.root(&fixture.db, VIEW).unwrap();
    assert_eq!(fixture.member_rows(&root.namespace), MEMBERS);
    fixture.record(mutation("gate/overflow", Some(facts())));
    assert!(fixture.installer.root(&fixture.db, VIEW).is_err());
    assert_eq!(fixture.member_rows(&root.namespace), MEMBERS);
    assert!(
        fixture
            .installer
            .status(&fixture.db, VIEW)
            .unwrap()
            .error
            .unwrap()
            .contains("membership cap")
    );
}
