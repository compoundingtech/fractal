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
    pub approval_subject: Option<String>,
}

/// The connection must be the writer transaction that commits the protected mutation.
pub fn authorize(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<(), St3Error> {
    let denied = || St3Error::new("forbidden", "assistant write authorization changed or expired before commit");
    let claim = |kind: &str| -> Result<Option<ClaimRecord>, St3Error> {
        connection.query_row(&latest_claim_of_kind_query(),
            params![authorization.grant_subject,kind],claim_from_row).optional().map_err(internal)
    };
    let granted = claim("custom.client.assistant.v1.granted")?.ok_or_else(denied)?;
    let policy_claim = claim("custom.client.assistant.v1.policy")?;
    let policy = policy_claim.as_ref().unwrap_or(&granted);
    if granted.origin != origin || policy.origin != origin || claim("custom.client.assistant.v1.revoked")?.is_some() {
        return Err(denied());
    }
    let fields = &policy.body["fields"];
    if fields["principal"] != "assistant/chatgpt" || fields["session_actor"] != authorization.actor
        || fields["issuer"] != authorization.owner || fields["disclosure_consent"] != true
        || fields["policy_revision"].as_u64() != Some(authorization.policy_revision)
        || fields["expires_at_unix_ms"].as_u64().map(u128::from).is_none_or(|expiry| expiry <= now_ms())
        || !fields["scopes"].as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some(&authorization.scope)))
    { return Err(denied()); }
    if let Some(recipient) = &authorization.recipient {
        if !fields["recipient_allowlist"].as_array().is_some_and(|recipients| recipients.iter().any(|value| value.as_str() == Some(recipient))) {
            return Err(denied());
        }
    }
    if let Some(subject) = &authorization.approval_subject {
        let approved: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='work.person-done' AND actor=?2 AND origin=?3 AND json_extract(body,'$.fields.answer.id')='approve' AND json_extract(body,'$.fields.acted_for') IS NULL)",
            params![subject,authorization.owner,origin],|row| row.get(0)).map_err(internal)?;
        if !approved { return Err(denied()); }
    }
    Ok(())
}
