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

/// Select the last authoritative transition, not the last arbitrary claim overlay.
/// Foreign results still require cryptographic member-writer verification by the
/// caller; the SQL provenance predicate excludes legacy and unsigned records.
pub fn validated_transition(
    connection: &Connection, granted: &ClaimRecord, owner: &str, local_origin: &str, kind: &str,
) -> Result<Option<ClaimRecord>, FractalError> {
    connection.query_row(
        validated_transition_query(),
        params![granted.subject, kind, granted.origin, owner, granted.id, local_origin],
        claim_from_row,
    ).optional().map_err(internal)
}

/// The origin/actor prefix bounds work independently of unauthoritative overlays.
/// All field checks precede LIMIT, so malformed newer policies cannot hide a
/// previous valid transition. Ordering intentionally matches subject history.
pub fn validated_transition_query() -> &'static str {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
        "WITH original AS (SELECT body FROM claims WHERE id=?5) SELECT {CLAIM_COLUMNS}
        FROM claims INDEXED BY claims_assistant_v1_transition_index
        WHERE claims.subject=?1 AND claims.kind=?2 AND claims.origin=?3 AND claims.actor=?4
        AND ?2 IN ('custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')
        AND json_type(body,'$.fields')='object'
        AND json_extract(body,'$.fields.issuer')=?4
        AND json_extract(body,'$.fields.grant_id')=json_extract((SELECT body FROM original),'$.fields.grant_id')
        AND json_extract(body,'$.fields.issuer')=json_extract((SELECT body FROM original),'$.fields.issuer')
        AND (json_type(body,'$.fields.principal') IS NULL
             OR json_extract(body,'$.fields.principal') IN ('assistant/chatgpt','assistant/mcp'))
        AND (json_type((SELECT body FROM original),'$.fields.principal') IS NULL
             OR json_extract((SELECT body FROM original),'$.fields.principal') IN ('assistant/chatgpt','assistant/mcp'))
        AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')
            =COALESCE(json_extract((SELECT body FROM original),'$.fields.principal'),'assistant/chatgpt')
        AND (json_type(body,'$.fields.profile') IS NULL OR json_extract(body,'$.fields.profile') IN ('assistant','service'))
        AND (json_type((SELECT body FROM original),'$.fields.profile') IS NULL OR json_extract((SELECT body FROM original),'$.fields.profile') IN ('assistant','service'))
        AND COALESCE(json_extract(body,'$.fields.profile'),'assistant')
            =COALESCE(json_extract((SELECT body FROM original),'$.fields.profile'),'assistant')
        AND (?2='custom.client.assistant.v1.revoked' OR (
            json_extract(body,'$.fields.session_actor')=json_extract((SELECT body FROM original),'$.fields.session_actor')
            AND json_extract(body,'$.fields.profile') IS json_extract((SELECT body FROM original),'$.fields.profile')
            AND json_extract(body,'$.fields.session_actor')
                =COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt') || '/session/' || json_extract(body,'$.fields.grant_id')
            AND ((length(json_extract(body,'$.fields.grant_id'))=24
                  AND json_extract(body,'$.fields.grant_id') NOT GLOB '*[^0-9a-fA-F]*'
                  AND claims.subject='custom/client/assistant-v1-' || json_extract(body,'$.fields.grant_id'))
                 OR (length(json_extract(body,'$.fields.grant_id'))=40
                  AND substr(json_extract(body,'$.fields.grant_id'),1,16)='assistant-grant/'
                  AND substr(json_extract(body,'$.fields.grant_id'),17) NOT GLOB '*[^0-9a-fA-F]*'
                  AND claims.subject='custom/client/assistant-v1-' || substr(json_extract(body,'$.fields.grant_id'),17)))
            AND json_extract(body,'$.fields.minimal_read_mode') IN ('approval-once','caller-bound-allow')
            AND json_type(body,'$.fields.disclosure_consent')='true'
            AND json_type(body,'$.fields.expires_at_unix_ms') IN ('integer','real')
            AND (body -> '$.fields.expires_at_unix_ms') NOT GLOB '*[^0-9]*'
            AND (length(body -> '$.fields.expires_at_unix_ms')<20
                 OR (length(body -> '$.fields.expires_at_unix_ms')=20
                     AND (body -> '$.fields.expires_at_unix_ms')<='18446744073709551615'))
            AND json_type(body,'$.fields.scopes')='array'
            AND NOT EXISTS (SELECT 1 FROM json_each(body,'$.fields.scopes') AS scope
                WHERE scope.type<>'text' OR scope.value NOT IN (
                    'read.assistant.summary','read.assistant.attention','read.assistant.missions',
                    'read.assistant.agents','read.assistant.conversation','read.assistant.terminal',
                    'write.assistant.messages','write.assistant.custom-reply','write.assistant.answers'))
            AND json_type(body,'$.fields.credential') IS NULL AND json_type(body,'$.fields.code') IS NULL
            AND json_type((SELECT body FROM original),'$.fields.credential') IS NULL AND json_type((SELECT body FROM original),'$.fields.code') IS NULL
            AND json_type((SELECT body FROM original),'$.fields.credential_hash')='text'
            AND length(json_extract((SELECT body FROM original),'$.fields.credential_hash'))=64
            AND json_extract((SELECT body FROM original),'$.fields.credential_hash') NOT GLOB '*[^0-9a-fA-F]*'
            AND NOT EXISTS (SELECT 1 FROM json_each(body,'$.fields') AS digest
                WHERE digest.key IN ('credential_hash','code_hash')
                  AND (digest.type<>'text' OR length(digest.value)<>64 OR digest.value GLOB '*[^0-9a-fA-F]*'))
            AND NOT EXISTS (SELECT 1 FROM json_each((SELECT body FROM original),'$.fields') AS digest
                WHERE digest.key IN ('credential_hash','code_hash')
                  AND (digest.type<>'text' OR length(digest.value)<>64 OR digest.value GLOB '*[^0-9a-fA-F]*'))
        ))
        AND (?3=?6 OR EXISTS (
            SELECT 1 FROM replica_records AS records
            JOIN replica_envelopes AS envelopes USING(writer,sequence,envelope_hash)
            JOIN replica_envelope_signatures AS signatures USING(writer,sequence,envelope_hash)
            WHERE records.claim_id=claims.id AND records.writer=claims.origin
              AND records.state='valid' AND envelopes.receipt_state IN ('validated','degraded')))
        ORDER BY claims.store_index DESC LIMIT 1"));
    QUERY.as_str()
}

pub fn authorize_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<String, FractalError> {
    let granted = authorized_grant(connection, origin, authorization)?;
    granted.body["fields"]["grant_id"].as_str().filter(|id| id.starts_with("assistant-grant/") && id.len() > "assistant-grant/".len())
        .map(str::to_owned).ok_or_else(|| FractalError::new("forbidden", "assistant grant identity is absent"))
}

pub fn authorized_grant(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<ClaimRecord, FractalError> {
    let denied = || FractalError::new("forbidden", "assistant write authorization changed or expired before commit");
    let claim = |kind: &str| -> Result<Option<ClaimRecord>, FractalError> {
        connection.query_row(/* sqlite-large-value: allow-for-now lv-hd82c937baff5 owner=fractal_store::store::assistant_write TODO=trace-statement-and-value-representation */ &latest_claim_of_kind_query(),
            params![authorization.grant_subject,kind],claim_from_row).optional().map_err(internal)
    };
    let granted = claim("custom.client.assistant.v1.granted")?.ok_or_else(denied)?;
    let policy_claim = validated_transition(connection, &granted, &authorization.owner, origin, "custom.client.assistant.v1.policy")?;
    let policy = policy_claim.as_ref().unwrap_or(&granted);
    if granted.origin != origin || policy.origin != origin
        || validated_transition(connection, &granted, &authorization.owner, origin, "custom.client.assistant.v1.revoked")?.is_some() {
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

    fn transition_fixture() -> (Connection, String, Value) {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(smallclaims::store::SCHEMA).unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES('batch','local',1,'hash','1')", []).unwrap();
        let suffix = "0123456789abcdef01234567";
        let subject = format!("custom/client/assistant-v1-{suffix}");
        let body = json!({"fields":{
            "grant_id":format!("assistant-grant/{suffix}"), "issuer":"person/owner",
            "session_actor":format!("assistant/chatgpt/session/assistant-grant/{suffix}"),
            "credential_hash":"a".repeat(64), "minimal_read_mode":"approval-once",
            "disclosure_consent":true, "scopes":["write.assistant.messages"],
            "expires_at_unix_ms":u64::MAX
        }});
        insert_transition(&connection, &subject, "original", "custom.client.assistant.v1.granted", "local", "person/owner", &body);
        (connection, subject, body)
    }

    fn insert_transition(connection: &Connection, subject: &str, id: &str, kind: &str, origin: &str, actor: &str, body: &Value) {
        connection.execute(
            "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES(?1,'batch',?2,?3,?4,?5,?6,'[]','1')",
            params![id,subject,kind,origin,actor,body.to_string()],
        ).unwrap();
    }

    #[test]
    fn validated_transition_index_is_installed_for_existing_stores() {
        let (connection, _, _) = transition_fixture();
        connection.execute_batch("DROP INDEX claims_assistant_v1_transition_index").unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        let definition: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE name='claims_assistant_v1_transition_index'",
            [], |row| row.get(0),
        ).unwrap();
        assert!(definition.contains("(subject, kind, origin, actor, store_index)"));
        connection.execute_batch(SCHEMA).unwrap();
    }

    #[test]
    fn validated_transition_p1_indexed_seek_is_independent_of_wrong_origin_actor_overlays() {
        let (connection, subject, body) = transition_fixture();
        let kind = "custom.client.assistant.v1.policy";
        insert_transition(&connection, &subject, "valid", kind, "local", "person/owner", &body);
        let query = validated_transition_query();
        let original = "original";
        let mut prior = None;
        for count in [32, 512, 7900] {
            for index in 0..count {
                insert_transition(&connection, &subject, &format!("overlay-{count}-{index}"), kind,
                    if index % 2 == 0 { "other" } else { "local" },
                    if index % 2 == 0 { "person/owner" } else { "person/other" }, &body);
            }
            let args = params![subject,kind,"local","person/owner",original,"local"];
            let mut statement = connection.prepare(query).unwrap();
            assert_eq!(statement.query_row(args, |row| row.get::<_, String>(0)).unwrap(), "valid");
            let cost = statement.get_status(rusqlite::StatementStatus::VmStep);
            if let Some(prior) = prior { assert!(cost <= prior + 20, "{prior} -> {cost}"); }
            prior = Some(cost);
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap()
                .query_map(args, |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
            assert!(plan.contains("claims_assistant_v1_transition_index (subject=? AND kind=? AND origin=? AND actor=?)"), "{plan}");
            assert!(!plan.contains("USE TEMP B-TREE FOR ORDER BY"), "{plan}");
        }
    }

    #[test]
    fn validated_transition_ignores_malformed_newer_policies_and_revocations() {
        let (connection, subject, body) = transition_fixture();
        for kind in ["custom.client.assistant.v1.policy", "custom.client.assistant.v1.revoked"] {
            let valid = format!("valid-{kind}");
            insert_transition(&connection, &subject, &valid, kind, "local", "person/owner", &body);
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
            for (index, patch) in patches.into_iter().enumerate() {
                let mut malformed = body.clone();
                malformed["fields"].as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
                insert_transition(&connection, &subject, &format!("malformed-{kind}-{index}"), kind, "local", "person/owner", &malformed);
            }
            let selected = connection.query_row(validated_transition_query(),
                params![subject,kind,"local","person/owner","original","local"],
                |row| row.get::<_, String>(0)).unwrap();
            assert_eq!(selected, valid);
        }
    }

    #[test]
    fn validated_transition_accepts_raw_hex_grant_identity() {
        let (connection, subject, mut body) = transition_fixture();
        let suffix = "0123456789abcdef01234567";
        body["fields"]["grant_id"] = json!(suffix);
        body["fields"]["session_actor"] = json!(format!("assistant/chatgpt/session/{suffix}"));
        connection.execute("UPDATE claims SET body=?1 WHERE id='original'", [body.to_string()]).unwrap();
        let kind = "custom.client.assistant.v1.policy";
        insert_transition(&connection, &subject, "raw-policy", kind, "local", "person/owner", &body);
        let selected = connection.query_row(validated_transition_query(),
            params![subject,kind,"local","person/owner","original","local"],
            |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(selected, "raw-policy");
    }

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
