#![allow(unused_imports)]
use crate::store::*;
use crate::store::seat_status::*;

/// `current_harness`, after checking that the login-only fold, which the attention read
    /// uses, reaches the same answer to "does this seat need a login?" for the same claims.
    pub fn checked_harness(store: &Store, subject: &str) -> Result<Option<crate::model::CurrentHarnessView>> {
        let full = store.current_harness(subject)?;
        let fast = store.current_harness_for_login(subject)?;
        assert_eq!(
            fast.as_ref().is_some_and(|harness| harness.state == "needs-login"),
            full.as_ref().is_some_and(|harness| harness.state == "needs-login"),
            "the login-only fold disagrees for {subject}: {fast:?} against {full:?}"
        );
        if let (Some(fast), Some(full)) = (&fast, &full)
            && fast.state == "needs-login"
        {
            assert_eq!(fast.incarnation_id, full.incarnation_id);
            assert_eq!(fast.observed_at_unix_ms, full.observed_at_unix_ms);
        }
        Ok(full)
    }

pub fn input(kind: &str, fields: Value) -> ClaimInput {
        ClaimInput {
            subject: "agent/cedar".into(),
            kind: kind.into(),
            actor: Some("agent/cedar".into()),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }
    }

pub fn runtime(store: &Store, incarnation: &str) {
        store
            .append_claim(&input(
                "runtime.observed",
                json!({
                    "status":"running", "incarnation_id":incarnation, "runtime_id":"native"
                }),
            ))
            .unwrap();
    }

pub fn observe(
        store: &Store,
        incarnation: &str,
        state: &str,
        at: u128,
        reason: &str,
    ) -> ClaimRecord {
        store.append_latest_observation(&input("harness.observed", json!({
            "state":state, "incarnation_id":incarnation, "observed_at_ms":at as u64, "reason":reason
        })), at).unwrap().0
    }
