//! Resolve declared reviewer identities in one caller-owned snapshot, without public gate dispatch.
use super::{Scope, FractalError};
use crate::store::{ReferenceGraph, identities, retirements};
use rusqlite::Connection;
use std::collections::BTreeSet;

/// Return the immutable storage identity, never a display name or canonical alias.
/// Run placeholders must be expanded before this check; unknown owners are not validated here.
pub(crate) fn resolve_reviewer(
    scope: &Scope<'_>,
    connection: &Connection,
    reviewer: &str,
) -> Result<String, FractalError> {
    fractal_schema::registry()
        .validate_subject(reviewer)
        .map_err(|error| FractalError::new(error.code, error.message))?;
    if !reviewer.starts_with("agent/") {
        return Err(FractalError::new(
            "invalid-reviewer",
            "agent-review reviewer must be an agent identity",
        ));
    }
    let identity = identities::resolve_reference_conn(connection, reviewer)?;
    if !scope.declared_subject(&ReferenceGraph { connection }, &identity)? {
        return Err(FractalError::new(
            "reviewer-unknown",
            format!("agent-review reviewer `{reviewer}` is not declared"),
        ));
    }
    if retirements::record(connection, &identity, None)?.is_some() {
        return Err(FractalError::new(
            "reviewer-retired",
            format!("agent-review reviewer `{reviewer}` is retired"),
        ));
    }
    Ok(identity)
}

/// Inputs are resolved identities. The caller supplies all known requesters, assignees,
/// current/submitting/carried workers, parent-step worker and the entire unclaimed pool.
/// This does not replace concrete run-owner validation or lifecycle-at-answer validation.
pub(crate) fn independent_reviewer(
    reviewer: &str,
    excluded: &BTreeSet<String>,
) -> Result<(), FractalError> {
    if excluded.contains(reviewer) {
        return Err(FractalError::new(
            "reviewer-not-independent",
            format!("agent-review reviewer `{reviewer}` is a known requester or work owner"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClaimInput, NormalizedIntent};
    use crate::store::identities::{IdentityKind, IdentityStore, RenameRequest};
    use crate::store::{ClaimCommitContext, Store, append_claim_with_commit_context};
    use serde_json::json;
    use std::collections::BTreeMap;

    const REVIEWER: &str = "agent/example/reviewer";
    const ALIAS: &str = "agent/example/renamed";

    fn intent() -> NormalizedIntent {
        crate::parse_intent(
            "version 2\nagent \"example/reviewer\" { command \"true\"; workspace \"/tmp\"; }",
            "example",
        )
        .unwrap()
    }

    fn declare(store: &Store) {
        let intent = intent();
        let expected = intent
            .subjects
            .keys()
            .map(|subject| {
                (
                    subject.clone(),
                    store
                        .selected_desired_token(subject)
                        .unwrap()
                        .into_iter()
                        .collect(),
                )
            })
            .collect();
        store.apply(&intent, &expected, "declare").unwrap();
    }

    fn empty_scope() -> Scope<'static> {
        Scope {
            run: BTreeSet::new(),
            intent: None,
        }
    }

    #[test]
    fn agent_review_resolver_same_publication() {
        let store = Store::open_memory("example").unwrap();
        assert_eq!(
            resolve_reviewer(
                &Scope::empty(&intent()),
                &store.connection.write(),
                REVIEWER
            )
            .unwrap(),
            REVIEWER
        );
    }

    #[test]
    fn agent_review_resolver_stored_declaration() {
        let store = Store::open_memory("example").unwrap();
        declare(&store);
        assert_eq!(
            resolve_reviewer(&empty_scope(), &store.connection.write(), REVIEWER).unwrap(),
            REVIEWER
        );
    }

    #[test]
    fn agent_review_resolver_unknown() {
        let store = Store::open_memory("example").unwrap();
        for reviewer in [REVIEWER, "agent/example/st-run-value-reviewer"] {
            let error =
                resolve_reviewer(&empty_scope(), &store.connection.write(), reviewer).unwrap_err();
            assert_eq!(error.code, "reviewer-unknown");
            assert!(error.message.contains(reviewer));
        }
    }

    #[test]
    fn agent_review_resolver_alias() {
        let store = Store::open_memory("example").unwrap();
        declare(&store);
        store
            .rename_identity(
                IdentityKind::Agent,
                &RenameRequest {
                    from: REVIEWER.into(),
                    to: ALIAS.into(),
                    actor: "person/robin".into(),
                    reason: "Clarify responsibility".into(),
                    expected_naming_revision: None,
                    expected_declaration: None,
                    idempotency_key: "rename".into(),
                },
                &[],
            )
            .unwrap();
        let identity = resolve_reviewer(&empty_scope(), &store.connection.write(), ALIAS).unwrap();
        assert_eq!(identity, REVIEWER);
        assert!(independent_reviewer(&identity, &BTreeSet::from([REVIEWER.into()])).is_err());
    }

    #[test]
    fn agent_review_resolver_retired_even_if_redeclared() {
        let store = Store::open_memory("example").unwrap();
        declare(&store);
        let retirement = ClaimInput {
            subject: REVIEWER.into(),
            kind: "agent.retired".into(),
            actor: Some("person/robin".into()),
            fields: BTreeMap::from([
                ("reason".into(), json!("Responsibility ended")),
                ("retired_by".into(), json!("person/robin")),
                (
                    "declaration".into(),
                    json!(store.selected_desired_token(REVIEWER).unwrap().unwrap()),
                ),
                ("owner_host".into(), json!(store.origin())),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("retire".into()),
        };
        append_claim_with_commit_context(
            &store.graph,
            &retirement,
            None,
            None,
            None,
            None,
            ClaimCommitContext {
                retirement_writer: true,
                ..Default::default()
            },
        )
        .unwrap();
        let error = resolve_reviewer(
            &Scope::empty(&intent()),
            &store.connection.write(),
            REVIEWER,
        )
        .unwrap_err();
        assert_eq!(error.code, "reviewer-retired");
        assert!(error.message.contains(REVIEWER));
    }

    #[test]
    fn agent_review_resolver_known_conflicts() {
        for role in [
            "requester",
            "assignee",
            "current-worker",
            "submitting-worker",
            "carried-worker",
            "parent-step-worker",
            "available-to-member",
        ] {
            let reviewer = format!("agent/example/{role}");
            let excluded = BTreeSet::from([reviewer.clone(), "agent/example/other".into()]);
            let error = independent_reviewer(&reviewer, &excluded).unwrap_err();
            assert_eq!(error.code, "reviewer-not-independent", "{role}");
            assert!(error.message.contains(&reviewer), "{role}");
            assert!(independent_reviewer(REVIEWER, &excluded).is_ok(), "{role}");
        }
        assert!(independent_reviewer(REVIEWER, &BTreeSet::new()).is_ok());
    }
}
