//! Server-established assistant provenance, rechecked against projected authority in the mutation's writer transaction.
use super::*;
use smallclaims::store::assistant_grant_authority as authority;

#[derive(Clone, Debug)]
pub struct AssistantWriteAuthorization {
    pub grant_subject: String,
    pub actor: String,
    pub owner: String,
    pub policy_revision: u64,
    pub scope: String,
    pub recipient: Option<String>,
}

impl AssistantWriteAuthorization {
    pub fn principal(&self) -> &str {
        self.actor.split_once("/session/").map_or("", |(principal, _)| principal)
    }
}


pub fn authorize_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<String, FractalError> {
    let granted = authorized_grant(connection, origin, authorization)?;
    granted.body["fields"]["grant_id"].as_str().filter(|id| id.starts_with("assistant-grant/") && id.len() > "assistant-grant/".len())
        .map(str::to_owned).ok_or_else(|| FractalError::new("forbidden", "assistant grant identity is absent"))
}

pub fn authorized_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<ClaimRecord, FractalError> {
    let denied = || FractalError::new("forbidden", "assistant write authorization changed or expired before commit");
    let authority_error = |error: FractalError| {
        if matches!(error.code.as_str(), "assistant-grant-authority-not-ready" | "assistant-grant-subject-not-ready") {
            denied()
        } else { error }
    };
    let granted = authority::latest(connection, &authorization.grant_subject, authority::GRANTED).map_err(authority_error)?
        .ok_or_else(denied)?;
    let policy_claim = authority::transition(connection, &granted, authority::POLICY).map_err(authority_error)?;
    let policy = policy_claim.as_ref().unwrap_or(&granted);
    if granted.origin != origin || policy.origin != origin
        || !authority::structurally_valid(&granted, policy)
        || authority::transition(connection, &granted, authority::REVOKED).map_err(authority_error)?.is_some() {
        return Err(denied());
    }
    let fields = &policy.body["fields"];
    let principal = authority::principal(fields).ok_or_else(denied)?;
    if fields.get("profile").is_some_and(|profile| profile != "assistant")
        || authorization.principal() != principal
        || fields["session_actor"] != authorization.actor
        || granted.actor.as_deref() != Some(authorization.owner.as_str())
        || policy.actor.as_deref() != Some(authorization.owner.as_str())
        || fields["issuer"] != authorization.owner
        || fields["policy_revision"].as_u64() != Some(authorization.policy_revision)
        || fields["expires_at_unix_ms"].as_u64().map(u128::from).is_none_or(|expiry| expiry <= now_ms())
        || !fields["scopes"].as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some(&authorization.scope)))
    { return Err(denied()); }
    if authorization.scope == "write.assistant.messages" {
        let recipient = authorization.recipient.as_deref().filter(|id| id.starts_with("agent/"))
            .ok_or_else(|| FractalError::new("forbidden", "assistant messages require an existing agent recipient"))?;
        if current_desired_row(connection, recipient).map_err(internal)?.is_none_or(|row| row.kind != "agent") {
            return Err(FractalError::new("forbidden", "assistant messages require an existing agent recipient"));
        }
    }
    Ok(granted)
}

/// Local generic writers cannot assert assistant identity or server-owned provenance.
pub fn refuse_unverified(kind: &str, actor: Option<&str>, from: Option<&str>, has_provenance: bool) -> Result<(), FractalError> {
    if kind == "message.sent"
        && (actor.is_some_and(|actor| actor.starts_with("assistant/"))
            || from.is_some_and(|from| from.starts_with("assistant/"))
            || has_provenance)
    {
        return Err(FractalError::new("assistant-send-refused", "assistant messages require the authorized native assistant path"));
    }
    Ok(())
}

/// Replication already authenticates the originating writer. Do not reauthorize an old
/// committed message against a receiver's missing, expired or revoked local grant.
pub fn validate_replicated(kind: &str, actor: Option<&str>, fields: &Value) -> Result<(), FractalError> {
    if kind != "message.sent" { return Ok(()); }
    if ["sender_class", "assistant_grant_id", "assistant_owner", "assistant_session_actor"].iter().all(|key| fields.get(*key).is_none()) {
        return refuse_unverified(kind, actor, fields["from"].as_str(), false);
    }
    if fields["sender_class"] != "external-assistant"
        || !fields["assistant_grant_id"].as_str().is_some_and(|id| id.starts_with("assistant-grant/") && id.len() > "assistant-grant/".len())
        || !matches!(actor, Some("assistant/chatgpt" | "assistant/mcp")) || fields["from"].as_str() != actor
        || !fields["assistant_owner"].as_str().is_some_and(|owner| owner.starts_with("person/") && owner.matches('/').count() == 1)
        || !fields["assistant_session_actor"].as_str().is_some_and(|session| {
            actor.is_some_and(|principal| session.strip_prefix(principal).and_then(|suffix| suffix.strip_prefix("/session/")) == fields["assistant_grant_id"].as_str())
        })
    {
        return Err(FractalError::new("assistant-send-refused", "replicated assistant provenance is inconsistent"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition_fixture() -> (Store, String, Value) {
        let store = Store::open_memory("local").unwrap();
        let suffix = "0123456789abcdef01234567";
        let subject = format!("custom/client/assistant-v1-{suffix}");
        let body = json!({"fields":{
            "grant_id":format!("assistant-grant/{suffix}"), "issuer":"person/owner",
            "session_actor":format!("assistant/chatgpt/session/assistant-grant/{suffix}"),
            "credential_hash":"a".repeat(64), "minimal_read_mode":"approval-once",
            "disclosure_consent":true, "scopes":["write.assistant.messages"],
            "expires_at_unix_ms":u64::MAX
        }});
        insert_transition(&store, &subject, authority::GRANTED, "person/owner", &body);
        (store, subject, body)
    }

    fn insert_transition(store: &Store, subject: &str, kind: &str, actor: &str, body: &Value) -> ClaimRecord {
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
            fields: serde_json::from_value(body["fields"].clone()).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    #[test]
    fn projected_transition_ignores_actor_overlay_flood() {
        let (store, subject, body) = transition_fixture();
        let granted = authority::latest(&store.readers.get(), &subject, authority::GRANTED).unwrap().unwrap();
        let valid = insert_transition(&store, &subject, authority::POLICY, "person/owner", &body);
        let mut prior = None;
        for count in [32, 512, 7900] {
            for index in 0..count {
                let mut overlay = body.clone();
                overlay["fields"]["nonce"] = json!(format!("{count}-{index}"));
                insert_transition(&store, &subject, authority::POLICY, "person/other", &overlay);
            }
            let selected = authority::transition(&store.readers.get(), &granted, authority::POLICY).unwrap().unwrap();
            assert_eq!(selected.id, valid.id);
            let connection = store.readers.get();
            let args = params![subject, authority::POLICY, granted.origin, "person/owner", granted.id];
            let mut statement = connection.prepare(authority::TRANSITION_QUERY).unwrap();
            let _: String = statement.query_row(args, |row| row.get(0)).unwrap();
            let cost = statement.get_status(rusqlite::StatementStatus::VmStep);
            if let Some(prior) = prior { assert!(cost <= prior + 20, "{prior} -> {cost}"); }
            prior = Some(cost);
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {}", authority::TRANSITION_QUERY)).unwrap()
                .query_map(args, |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
            assert!(plan.contains("assistant_grant_authority_transition_index"), "{plan}");
            assert!(!plan.contains("USE TEMP B-TREE FOR ORDER BY"), "{plan}");
        }
    }

    #[test]
    fn projected_transition_ignores_malformed_newer_policies_and_revocations() {
        let (store, subject, body) = transition_fixture();
        let granted = authority::latest(&store.readers.get(), &subject, authority::GRANTED).unwrap().unwrap();
        for kind in [authority::POLICY, authority::REVOKED] {
            let valid = insert_transition(&store, &subject, kind, "person/owner", &body);
            let mut patches = vec![
                json!({"principal":null}), json!({"principal":"assistant/mcp"}),
                json!({"profile":null}), json!({"profile":"service"}),
                json!({"issuer":"person/other"}), json!({"grant_id":"assistant-grant/other"}),
            ];
            if kind.ends_with(".policy") {
                patches.extend([
                    json!({"session_actor":"assistant/chatgpt/session/other"}),
                    json!({"minimal_read_mode":"unknown"}), json!({"disclosure_consent":false}),
                    json!({"expires_at_unix_ms":null}), json!({"expires_at_unix_ms":-1}),
                    json!({"expires_at_unix_ms":1.5}), json!({"scopes":["unknown"]}),
                    json!({"expires_at_unix_ms":1e19}),
                    json!({"scopes":[null]}), json!({"credential":"secret"}),
                    json!({"credential_hash":"invalid"}), json!({"code_hash":null}),
                ]);
            }
            for patch in patches {
                let mut malformed = body.clone();
                malformed["fields"].as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
                insert_transition(&store, &subject, kind, "person/owner", &malformed);
            }
            let selected = authority::transition(&store.readers.get(), &granted, kind).unwrap().unwrap();
            assert_eq!(selected.id, valid.id);
        }
    }

    #[test]
    fn projected_transition_accepts_raw_hex_grant_identity() {
        let store = Store::open_memory("local").unwrap();
        let suffix = "0123456789abcdef01234567";
        let subject = format!("custom/client/assistant-v1-{suffix}");
        let body = json!({"fields":{"grant_id":suffix,"issuer":"person/owner",
            "session_actor":format!("assistant/chatgpt/session/{suffix}"),
            "credential_hash":"a".repeat(64),"minimal_read_mode":"approval-once",
            "disclosure_consent":true,"scopes":[],"expires_at_unix_ms":u64::MAX}});
        let granted = insert_transition(&store, &subject, authority::GRANTED, "person/owner", &body);
        let valid = insert_transition(&store, &subject, authority::POLICY, "person/owner", &body);
        let selected = authority::transition(&store.readers.get(), &granted, authority::POLICY).unwrap().unwrap();
        assert_eq!(selected.id, valid.id);
    }

    #[test]
    fn grant_principal_defaults_only_absent_legacy_fields_to_chatgpt() {
        assert_eq!(authority::principal(&json!({})), Some("assistant/chatgpt"));
        for expected in ["assistant/chatgpt", "assistant/mcp"] {
            assert_eq!(authority::principal(&json!({"principal":expected})), Some(expected));
        }
        for forged in [Value::Null, json!("assistant/other"), json!("person/other"), json!(7)] {
            assert_eq!(authority::principal(&json!({"principal":forged})), None);
        }
    }

    #[test]
    fn replicated_assistant_sender_keeps_owner_and_session_as_provenance_only() {
        for principal in ["assistant/chatgpt", "assistant/mcp"] {
            let fields = json!({
                "from":principal, "sender_class":"external-assistant",
                "assistant_grant_id":"assistant-grant/one", "assistant_owner":"person/robin",
                "assistant_session_actor":format!("{principal}/session/assistant-grant/one"),
            });
            validate_replicated("message.sent", Some(principal), &fields).unwrap();
            for (actor, patch) in [
                ("person/robin", json!({"from":"person/robin"})),
                (principal, json!({"assistant_owner":null})),
                (principal, json!({"assistant_session_actor":format!("{principal}/session/assistant-grant/other")})),
                (principal, json!({"from":"assistant/other"})),
                (principal, json!({"assistant_session_actor":"assistant/other/session/assistant-grant/one"})),
            ] {
                let mut forged = fields.clone();
                forged.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
                assert!(validate_replicated("message.sent", Some(actor), &forged).is_err());
            }
            let other = if principal == "assistant/mcp" { "assistant/chatgpt" } else { "assistant/mcp" };
            assert!(validate_replicated("message.sent", Some(other), &fields).is_err());
            assert!(refuse_unverified("message.sent", Some(principal), Some(principal), false).is_err());
        }
    }

}
