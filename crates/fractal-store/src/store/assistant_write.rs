//! Server-established assistant provenance, checked in the mutation's writer transaction.
use super::*;

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

pub fn principal(fields: &Value) -> Option<&str> {
    match fields.get("principal") {
        None => Some("assistant/chatgpt"),
        Some(value) => value.as_str().filter(|principal| matches!(*principal, "assistant/chatgpt" | "assistant/mcp")),
    }
}

pub fn authorize_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<String, St3Error> {
    let granted = authorized_grant(connection, origin, authorization)?;
    granted.body["fields"]["grant_id"].as_str().filter(|id| id.starts_with("assistant-grant/") && id.len() > "assistant-grant/".len())
        .map(str::to_owned).ok_or_else(|| St3Error::new("forbidden", "assistant grant identity is absent"))
}

pub fn authorized_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<ClaimRecord, St3Error> {
    let denied = || St3Error::new("forbidden", "assistant write authorization changed or expired before commit");
    let claim = |kind: &str| -> Result<Option<ClaimRecord>, St3Error> {
        connection.query_row(/* sqlite-large-value: allow-for-now lv-hd82c937baff5 owner=fractal_store::store::assistant_write TODO=trace-statement-and-value-representation */ &latest_claim_of_kind_query(),
            params![authorization.grant_subject,kind],claim_from_row).optional().map_err(internal)
    };
    let granted = claim("custom.client.assistant.v1.granted")?.ok_or_else(denied)?;
    let policy_claim = claim("custom.client.assistant.v1.policy")?;
    let policy = policy_claim.as_ref().unwrap_or(&granted);
    if granted.origin != origin || policy.origin != origin || claim("custom.client.assistant.v1.revoked")?.is_some() {
        return Err(denied());
    }
    let fields = &policy.body["fields"];
    let principal = principal(fields).ok_or_else(denied)?;
    if self::principal(&granted.body["fields"]) != Some(principal)
        || ["grant_id", "session_actor", "issuer"].iter().any(|key| fields[*key] != granted.body["fields"][*key])
        || authorization.principal() != principal
        || fields["session_actor"] != authorization.actor
        || fields["session_actor"].as_str().and_then(|actor| actor.strip_prefix(principal))
            .and_then(|suffix| suffix.strip_prefix("/session/")) != fields["grant_id"].as_str()
        || granted.actor.as_deref() != Some(authorization.owner.as_str())
        || policy.actor.as_deref() != Some(authorization.owner.as_str())
        || fields["issuer"] != authorization.owner || fields["disclosure_consent"] != true
        || fields["policy_revision"].as_u64() != Some(authorization.policy_revision)
        || fields["expires_at_unix_ms"].as_u64().map(u128::from).is_none_or(|expiry| expiry <= now_ms())
        || !fields["scopes"].as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some(&authorization.scope)))
    { return Err(denied()); }
    if authorization.scope == "write.assistant.messages" {
        let recipient = authorization.recipient.as_deref().filter(|id| id.starts_with("agent/"))
            .ok_or_else(|| St3Error::new("forbidden", "assistant messages require an existing agent recipient"))?;
        if current_desired_row(connection, recipient).map_err(internal)?.is_none_or(|row| row.kind != "agent") {
            return Err(St3Error::new("forbidden", "assistant messages require an existing agent recipient"));
        }
    }
    Ok(granted)
}

/// Local generic writers cannot assert assistant identity or server-owned provenance.
pub fn refuse_unverified(kind: &str, actor: Option<&str>, from: Option<&str>, has_provenance: bool) -> Result<(), St3Error> {
    if kind == "message.sent"
        && (actor.is_some_and(|actor| actor.starts_with("assistant/"))
            || from.is_some_and(|from| from.starts_with("assistant/"))
            || has_provenance)
    {
        return Err(St3Error::new("assistant-send-refused", "assistant messages require the authorized native assistant path"));
    }
    Ok(())
}

/// Replication already authenticates the originating writer. Do not reauthorize an old
/// committed message against a receiver's missing, expired or revoked local grant.
pub fn validate_replicated(kind: &str, actor: Option<&str>, fields: &Value) -> Result<(), St3Error> {
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
        return Err(St3Error::new("assistant-send-refused", "replicated assistant provenance is inconsistent"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_principal_defaults_only_absent_legacy_fields_to_chatgpt() {
        assert_eq!(principal(&json!({})), Some("assistant/chatgpt"));
        for expected in ["assistant/chatgpt", "assistant/mcp"] {
            assert_eq!(principal(&json!({"principal":expected})), Some(expected));
        }
        for forged in [Value::Null, json!("assistant/other"), json!("person/other"), json!(7)] {
            assert_eq!(principal(&json!({"principal":forged})), None);
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
