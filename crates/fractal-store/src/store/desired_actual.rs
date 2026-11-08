//! Cheap current-cut summaries of selected declarations, not a runtime status reduction.
//!
//! Call inside the bulk reader's pinned reader snapshot. An agent's
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

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum DesiredActualUnavailableCode {
    MemberVersionUnproven,
}

#[derive(serde::Serialize)]
struct DesiredActualUnavailable {
    code: DesiredActualUnavailableCode,
}

#[derive(serde::Serialize)]
struct DesiredActualUnavailableSummary {
    unavailable: DesiredActualUnavailable,
}

impl Store {
    pub fn desired_actual_summary(&self, desired: &DesiredSubject) -> Result<Option<Value>> {
        let connection = self.readers.get();
        let definition = match declarations::selected(&connection, &desired.subject, None) {
            Ok(Some(definition)) => definition,
            Ok(None) => return Ok(None),
            Err(error) if error.code == "member-version-unproven" => {
                return Ok(Some(serde_json::to_value(DesiredActualUnavailableSummary {
                    unavailable: DesiredActualUnavailable {
                        code: DesiredActualUnavailableCode::MemberVersionUnproven,
                    },
                })?));
            }
            Err(error) => return Err(error.into()),
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
    fn newer_member_payload_is_unavailable_without_failing_other_batch_items() {
        let store = Store::open_memory("node").unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"example/before\" { command \"true\"; workspace \".\"; restart \"never\"; }\nagent \"example/newer\" { command \"true\"; workspace \".\"; restart \"never\"; }\nagent \"example/after\" { command \"true\"; workspace \".\"; restart \"never\"; }\n",
            "node",
        ).unwrap();
        publish(&store, &intent, "publish-members");
        let request = desired_batch::DesiredBatchRequest::default();
        let before = store.desired_batch(&request).unwrap();
        let mut member = serde_json::to_value(
            intent.subjects["agent/example/newer"].member.as_ref().unwrap(),
        ).unwrap();
        member["kind"] = json!("future-member-kind");
        store.replace_desired_member_for_test(
            "agent/example/newer", &serde_json::to_string(&member).unwrap(),
        );

        let page = store.desired_batch(&request).unwrap();
        assert_eq!(page.items.len(), 3);
        assert!(page.next_cursor.is_none());
        assert_eq!(page.store_index, before.store_index);
        for (item, original) in page.items.iter().zip(&before.items) {
            assert_eq!(item.declaration.subject, original.declaration.subject);
            assert_eq!(item.desired_token, original.desired_token);
            if item.declaration.subject == "agent/example/newer" {
                assert_eq!(item.actual, Some(json!({
                    "unavailable": {"code": "member-version-unproven"},
                })));
                assert!(item.declaration.member.is_none());
            } else {
                assert_eq!(item.actual, original.actual);
                assert!(item.actual.as_ref().unwrap()["revision"].is_string());
            }
        }
        // Other declaration consumers still receive the precise decode failure.
        let connection = store.readers.get();
        let error = declarations::selected(&connection, "agent/example/newer", None).unwrap_err();
        assert_eq!(error.code, "member-version-unproven");
        assert!(error.message.contains("future-member-kind"));
    }

    #[test]
    fn nested_member_schema_extensions_are_unavailable_per_item() {
        let store = Store::open_memory("node").unwrap();
        let intent = declaration("sleep 100");
        publish(&store, &intent, "publish-nested-member");
        let member = serde_json::to_value(
            intent.subjects["agent/example/worker"].member.as_ref().unwrap(),
        ).unwrap();
        let request = desired_batch::DesiredBatchRequest::default();
        let mut future_launch = member.clone();
        future_launch["launch"] = json!({"type": "future-launch", "value": {}});
        let mut future_budget = member;
        future_budget["declared_budgets"] = json!({"future_budget": 1});
        for (payload, prefix) in [
            (future_launch, "unknown variant "),
            (future_budget, "unknown field "),
        ] {
            let payload = serde_json::to_string(&payload).unwrap();
            // Pin serde_json's classification wording so dependency changes cannot
            // silently turn supported per-item degradation into whole-page failures.
            let error = serde_json::from_str::<crate::model::MemberSpec>(&payload).unwrap_err();
            assert!(error.to_string().starts_with(prefix), "{error}");
            store.replace_desired_member_for_test("agent/example/worker", &payload);
            let page = store.desired_batch(&request).unwrap();
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].actual, Some(json!({
                "unavailable": {"code": "member-version-unproven"},
            })));
        }
    }

    #[test]
    fn malformed_known_member_payloads_still_fail_the_batch() {
        let store = Store::open_memory("node").unwrap();
        let intent = declaration("sleep 100");
        publish(&store, &intent, "publish-known-member");
        let member = serde_json::to_value(
            intent.subjects["agent/example/worker"].member.as_ref().unwrap(),
        ).unwrap();
        let mut corrupt = member.clone();
        corrupt["host"] = json!(42);
        let mut missing = member.clone();
        missing.as_object_mut().unwrap().remove("launch");
        let mut invalid_launch_tag = member.clone();
        invalid_launch_tag["launch"] = json!({"type": 42, "value": "true"});
        let mut invalid_launch_value = member.clone();
        invalid_launch_value["launch"] = json!({"type": "shell", "value": 42});
        let mut invalid_kind = member;
        invalid_kind["kind"] = json!(42);
        // This decoder regression intentionally stores syntactically invalid JSON.
        // The normal agent-host expression index rejects that fixture before the
        // reader sees it, so remove only that index in this isolated test store.
        store.connection.write().execute_batch(
            "DROP INDEX desired_agent_host_index",
        ).unwrap();
        for payload in [
            serde_json::to_string(&corrupt).unwrap(),
            serde_json::to_string(&missing).unwrap(),
            serde_json::to_string(&invalid_kind).unwrap(),
            serde_json::to_string(&invalid_launch_tag).unwrap(),
            serde_json::to_string(&invalid_launch_value).unwrap(),
            "{broken".into(),
            r#"{"kind":"future-member-kind","unfinished":"#.into(),
            r#"{"kind":"future-member-kind"} trailing-garbage"#.into(),
            r#"{"kind":"agent","launch":{"type":"future-launch"},"unfinished":"#.into(),
            r#"{"kind":"agent","launch":{"type":"future-launch"}} trailing-garbage"#.into(),
        ] {
            store.replace_desired_member_for_test("agent/example/worker", &payload);
            let error = store.desired_batch(
                &desired_batch::DesiredBatchRequest::default(),
            ).unwrap_err();
            let error = error.downcast_ref::<FractalError>().unwrap();
            assert_ne!(error.code, "member-version-unproven");
        }
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
