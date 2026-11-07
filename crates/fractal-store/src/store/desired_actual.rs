//! Cheap current-cut summaries of selected declarations, not a runtime status reduction.
//!
//! Call inside the bulk reader's pinned `read_snapshot_unindexed` closure. An agent's
//! `revision` is its published declaration revision, as on its card, not an observation's
//! revision or the desired token. Runtime/card fields without a maintained folded source
//! are deliberately absent: fetch the card for status, operation_status, state, host,
//! incarnation_id, suspension, hold, kind, repo, actual_origin and projection. In particular,
//! a sparse newest observation cannot substitute for the actual-state merge, and card state
//! additionally depends on harness, authority, handoff, faults, suspension and retirement.
//!
//! Nonagents expose the selected declaration revision and digest, never its body. Mission
//! state comes from the transactionally selected mission definition (draft/ready/retired),
//! not a run's state. Other kinds have no common maintained actual state and omit it.
//! Digest format is `sha256:<lowercase hex>` over the UTF-8 bytes of
//! `smallclaims::hash::canonical_json_text(PublicationDefinition.declaration)`: recursively
//! sorted object keys, unchanged array order and compact JSON. The entire serialized selected
//! declaration is included (including its own revision when present); publication wrapper
//! metadata, token and actual summary are not. This is not the declaration revision algorithm.
use super::*;

impl Store {
    pub fn desired_actual_summary(&self, desired: &DesiredSubject) -> Result<Option<Value>> {
        let connection = self.readers.get();
        let Some(definition) = declarations::selected(&connection, &desired.subject, None)? else {
            return Ok(None);
        };
        let mut summary = serde_json::Map::new();
        summary.insert("revision".into(), json!(definition.revision));
        if !desired.subject.starts_with("agent/") {
            let canonical = canonical_json_text(&definition.declaration)?;
            summary.insert("digest".into(), json!(format!(
                "sha256:{}", hex::encode(Sha256::digest(canonical.as_bytes()))
            )));
            if let Some(id) = desired.subject.strip_prefix("mission/") {
                let state: String = connection.query_row(
                    "SELECT state FROM mission_definitions WHERE mission_id=?1",
                    [id], |row| row.get(0),
                )?;
                summary.insert("state".into(), json!(state));
            }
        }
        Ok(Some(Value::Object(summary)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::publication_tests_shared::{declaration, observe, publish};

    fn summary(store: &Store, desired: &DesiredSubject) -> Value {
        store.read_snapshot_unindexed(|| store.desired_actual_summary(desired))
            .unwrap().unwrap()
    }

    fn assert_agent_fields(store: &Store, expected_status: &str) {
        let desired = store.desired_subjects().unwrap().into_iter()
            .find(|desired| desired.subject == "agent/example/worker").unwrap();
        let actual = summary(store, &desired);
        // The card uses desired_revision when a selected declaration exists (including stop).
        let connection = store.readers.get();
        let (card, _) = subject_status_at(
            &connection, &desired.subject, None, None, ClaimsReduction::AgentCard,
        ).unwrap().unwrap();
        assert_eq!(actual["revision"], json!(card.desired_revision.unwrap()));
        assert_eq!(card.actual.unwrap()["status"], expected_status);
        assert_eq!(actual.as_object().unwrap().len(), 1,
            "runtime fields must not be guessed from sparse observations or desired members");
    }

    #[test]
    fn running_agent_revision_matches_card_despite_sparse_latest_observation() {
        let store = Store::open_memory("node").unwrap();
        publish(&store, &declaration("sleep 100"), "publish");
        observe(&store, "running-one", None);
        store.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(),
            kind: "runtime.action.succeeded".into(), actor: None,
            fields: BTreeMap::from([
                ("action".into(), json!("start")),
                ("operation_status".into(), json!("verified")),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert_agent_fields(&store, "running");
    }

    #[test]
    fn stopped_agent_revision_matches_card_without_inventing_runtime_fields() {
        let store = Store::open_memory("node").unwrap();
        publish(&store, &declaration("sleep 100"), "publish");
        observe(&store, "stopped-one", None);
        let stop = crate::graph::parse_intent(
            "version 2\nstop \"agent/example/worker\"\n", "node",
        ).unwrap();
        publish(&store, &stop, "stop");
        store.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(), kind: "runtime.observed".into(),
            actor: None, fields: BTreeMap::from([
                ("status".into(), json!("stopped")),
                ("incarnation_id".into(), json!("stopped-one")),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert_agent_fields(&store, "stopped");
    }

    #[test]
    fn published_mission_revision_state_and_digest_match_selected_definition() {
        let store = Store::open_memory("node").unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nmission \"summary\" state=\"ready\" {\n goal \"Summary parity.\"\n step \"work\" { agentless }\n}\n", "node",
        ).unwrap();
        publish(&store, &intent, "publish-mission");
        let desired = DesiredSubject {
            subject: "mission/summary".into(), kind: "mission".into(),
            desired: Value::Null, accountable: None, member: None,
            owner_run: None, owner_generation: None, owner_step: None,
        };
        let actual = summary(&store, &desired);
        let connection = store.readers.get();
        let selected = declarations::selected(&connection, &desired.subject, None)
            .unwrap().unwrap();
        let (status, _) = subject_status_at(
            &connection, &desired.subject, None, None, ClaimsReduction::Full,
        ).unwrap().unwrap();
        let full = status.actual.unwrap();
        assert_eq!(actual["revision"], full["revision"]);
        assert_eq!(actual["state"], full["state"]);
        assert_eq!(actual["revision"], selected.revision);
        let canonical = canonical_json_text(&selected.declaration).unwrap();
        assert_eq!(actual["digest"], format!(
            "sha256:{}", hex::encode(Sha256::digest(canonical.as_bytes()))
        ));
        assert_eq!(actual.as_object().unwrap().len(), 3);
        assert!(actual.get("body").is_none());
    }
}
