//! Thread delivery uses ordinary message claims, with separate quote and ask relations.

use super::*;
use fractal_schema::threads::{MESSAGE_READER_VERSION, MESSAGE_READER_VERSION_FIELD};
use thread_metadata::{PersonAuthority, creation_claim, latest_applied_claim};

pub(super) fn add_reader_marker(input: &mut ClaimInput) {
    input.fields.insert(MESSAGE_READER_VERSION_FIELD.into(), json!(MESSAGE_READER_VERSION));
}

pub(super) fn verified_signature(
    input: &ClaimInput,
    principal: &PersonAuthority<'_>,
    signature: Option<&smallclaims::principal::ClaimSignature>,
) -> Result<Option<String>, FractalError> {
    use smallclaims::principal::{FIELDS_FORMAT, Judged};
    let invalid = || {
        FractalError::new(
            "invalid-thread-signature",
            "the device signature must bind the thread and message",
        )
    };
    let Some(signature) = signature else {
        return if matches!(principal, PersonAuthority::Paired { .. }) {
            Err(invalid())
        } else {
            Ok(None)
        };
    };
    // Browser sends are bearer-authorized and are signed by the daemon's claim path.
    if matches!(principal, PersonAuthority::PairedBrowser(_)) {
        return Err(invalid());
    }
    let Some(actor) = input.actor.as_deref() else {
        return Err(invalid());
    };
    let binds = |name: &str| signature.signed_fields.iter().any(|field| field == name);
    let required = ["content", "from", "to", "thread_id", MESSAGE_READER_VERSION_FIELD];
    let contextual = [
        "thread_quote_id",
        "thread_origin_ask",
        "thread_origin_source",
        "thread_origin_episode",
        "title",
    ];
    if !actor.starts_with("person/")
        || signature.signer != actor
        || signature.on_behalf.is_some()
        || signature.format.as_deref() != Some(FIELDS_FORMAT)
        || !required.iter().all(|field| binds(field))
        || contextual
            .iter()
            .any(|field| input.fields.contains_key(*field) && !binds(field))
    {
        return Err(invalid());
    }
    let fields = json!(input.fields);
    let judged = Judged {
        id: "",
        subject: &input.subject,
        kind: &input.kind,
        actor: Some(actor),
        content: String::new(),
        fields: &fields,
    };
    if !signature.verifies(&judged) {
        return Err(invalid());
    }
    serde_json::to_string(signature).map(Some).map_err(internal)
}

/// Recheck enrolled device authority inside the same snapshot as the write or receipt replay.
pub(super) fn check_enrollment(
    connection: &Connection,
    principal: &PersonAuthority<'_>,
    person: &str,
    signature: Option<&smallclaims::principal::ClaimSignature>,
) -> Result<(), FractalError> {
    use smallclaims::principal::{KEY_GRANTED, KEY_REVOKED, KeyGrant, Role};
    let unavailable = || {
        FractalError::new(
            "device-key-not-enrolled",
            "the thread message signing key is unavailable",
        )
    };
    let Some(signature) = signature else {
        return Ok(());
    };
    let key_grant = signature.chain.first().ok_or_else(unavailable)?;
    let claim = claim_by_id_tx(connection, key_grant)
        .map_err(internal)?
        .ok_or_else(unavailable)?;
    let enrolled = claim.subject == person
        && claim.kind == KEY_GRANTED
        && KeyGrant::from_fields(&claim.body["fields"])
            .is_some_and(|grant| grant.key == signature.key && grant.role == Role::Device);
    let revoked: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind=?2 AND json_extract(body, '$.fields.key')=?3)",
        params![person, KEY_REVOKED, signature.key], |row| row.get(0),
    ).map_err(internal)?;
    if !enrolled || revoked {
        return Err(unavailable());
    }
    if let PersonAuthority::Paired { completion, .. } = principal {
        let pairing = claim_by_id_tx(connection, completion)
            .map_err(internal)?
            .ok_or_else(unavailable)?;
        if pairing.body["fields"]["device_public_key"].as_str() != Some(signature.key.as_str()) {
            return Err(unavailable());
        }
    }
    Ok(())
}

/// Determine the other stable participant without exposing a thread to a third party.
pub fn reply_recipient(store: &Store, thread: &str, actor: &str) -> Result<String, FractalError> {
    let connection = store.readers.get();
    let unavailable = || FractalError::new("thread-unavailable", "the thread is unavailable");
    let created = creation_claim(&connection, thread)?.ok_or_else(unavailable)?;
    let fields = &created.body["fields"];
    let person = fields["person"].as_str().ok_or_else(unavailable)?;
    let agent = fields["agent"].as_str().ok_or_else(unavailable)?;
    let actor = identities::resolve_reference_conn(&connection, actor)?;
    if actor == identities::resolve_reference_conn(&connection, person)? {
        return Ok(agent.into());
    }
    if actor == identities::resolve_reference_conn(&connection, agent)? {
        return Ok(person.into());
    }
    Err(unavailable())
}

pub fn append_reply(
    store: &Store,
    input: &ClaimInput,
    principal: PersonAuthority<'_>,
    signature: Option<&smallclaims::principal::ClaimSignature>,
) -> Result<(ClaimRecord, bool), FractalError> {
    let unavailable = || FractalError::new("thread-unavailable", "the thread is unavailable");
    let invalid = || FractalError::new("invalid-thread-message", "the thread message is invalid");
    let thread = input
        .fields
        .get("thread_id")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let from = input
        .fields
        .get("from")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let to = input
        .fields
        .get("to")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let quote = input.fields.get("thread_quote_id").and_then(Value::as_str);
    let person_sends = from.starts_with("person/");
    if input.kind != "message.sent"
        || input.actor.as_deref() != Some(from)
        || input.idempotency_key.is_none()
        || input
            .fields
            .get("in_reply_to")
            .is_some_and(|value| !value.is_null())
        || input.fields.contains_key("thread_origin_ask")
        || input.fields.contains_key("thread_origin_source")
        || input.fields.contains_key("thread_origin_episode")
    {
        return Err(invalid());
    }
    let creation = |connection: &Connection| -> Result<ClaimRecord, FractalError> {
        creation_claim(connection, thread)?.ok_or_else(unavailable)
    };
    let authority = |connection: &Connection| {
        store.thread_write_rollout(connection)?;
        let thread = creation(connection)?;
        let person = thread.body["fields"]["person"]
            .as_str()
            .ok_or_else(unavailable)?;
        let agent = thread.body["fields"]["agent"]
            .as_str()
            .ok_or_else(unavailable)?;
        let (sender, recipient) = if person_sends {
            (person, agent)
        } else {
            (agent, person)
        };
        if identities::resolve_reference_conn(connection, from)?
            != identities::resolve_reference_conn(connection, sender)?
            || identities::resolve_reference_conn(connection, to)?
                != identities::resolve_reference_conn(connection, recipient)?
        {
            return Err(unavailable());
        }
        if let PersonAuthority::BoundAgent { actor } = &principal {
            if person_sends
                || identities::resolve_reference_conn(connection, actor)?
                    != identities::resolve_reference_conn(connection, agent)?
            {
                return Err(unavailable());
            }
        } else {
            principal.check(store, connection, person)?;
            check_enrollment(connection, &principal, person, signature)?;
        }
        Ok(())
    };
    let admission = |connection: &Connection| {
        if let Some(quote) = quote {
            let target =
                latest_applied_claim(connection, quote, "message.sent")?.ok_or_else(invalid)?;
            match target.body["fields"]["thread_id"].as_str() {
                Some(current) if current == thread => {}
                None => {
                    let created = creation(connection)?;
                    if created.body["fields"]["first_message"].as_str() != Some(quote)
                        || target.batch_id != created.batch_id
                    {
                        return Err(invalid());
                    }
                }
                _ => return Err(invalid()),
            }
            let party = |name| identities::resolve_reference_conn(connection, name);
            let target_from = target.body["fields"]["from"].as_str().ok_or_else(invalid)?;
            let target_to = target.body["fields"]["to"].as_str().ok_or_else(invalid)?;
            if target.actor.as_deref().map(party).transpose()?.as_deref()
                != Some(party(target_from)?.as_str())
            {
                return Err(invalid());
            }
            let mut expected = [party(from)?, party(to)?];
            let mut quoted = [party(target_from)?, party(target_to)?];
            expected.sort();
            quoted.sort();
            if quoted != expected {
                return Err(invalid());
            }
        }
        Ok(())
    };
    store.graph.ensure_principal_key(from)?;
    let mut marked = input.clone();
    add_reader_marker(&mut marked);
    let encoded_signature = verified_signature(&marked, &principal, signature)?;
    append_claim_with_commit_context(
        &store.graph,
        &marked,
        None,
        None,
        None,
        None,
        ClaimCommitContext {
            signature: encoded_signature.as_deref(),
            authority: Some(&authority),
            admission: Some(&admission),
            thread_delivery: true,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use thread_metadata::{Edit, apply};

    #[test]
    fn thread_delivery_keeps_quote_and_close_separate_from_ordinary_settlement() {
        let store = Store::open_memory("thread-delivery").unwrap();
        store.configure_thread_write_rollout([]);
        let person = "person/avery";
        let agent = "agent/reviewer";
        let thread = "thread/worker";
        let (_created, _) = apply(
            &store,
            person,
            thread,
            "create",
            Edit::Create {
                agent,
                title: "Worker test",
            },
        )
        .unwrap();
        let input = |subject: &str, from: &str, to: &str, quote: Option<&str>| {
            let mut fields = BTreeMap::from([
                ("thread_id".into(), json!(thread)),
                ("from".into(), json!(from)),
                ("to".into(), json!(to)),
                ("content".into(), json!("Worker test output")),
                ("status".into(), json!("sent")),
            ]);
            if let Some(quote) = quote {
                fields.insert("thread_quote_id".into(), json!(quote));
            }
            ClaimInput {
                subject: subject.into(),
                kind: "message.sent".into(),
                actor: Some(from.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(subject.into()),
            }
        };
        let first = input("message/first", person, agent, None);
        assert_eq!(
            store.append_client_claim(&first).unwrap_err().code,
            "thread-delivery-required"
        );
        let (sent, _) = append_reply(&store, &first, PersonAuthority::TrustedLocal, None).unwrap();
        let foreign = input("message/foreign", person, agent, Some("message/missing"));
        assert_eq!(
            append_reply(&store, &foreign, PersonAuthority::TrustedLocal, None)
                .unwrap_err()
                .code,
            "invalid-thread-message"
        );
        let (closed, _) = apply(
            &store,
            person,
            thread,
            "close",
            Edit::Close {
                revision: &crate::store::thread_metadata::summary(&store, person, thread).unwrap().revision,
            },
        )
        .unwrap();
        let ordinary = input("message/closed-person-reply", person, agent, Some("message/first"));
        let (posted, appended) = append_reply(&store, &ordinary, PersonAuthority::TrustedLocal, None).unwrap();
        assert!(appended);
        let (retried, appended) = append_reply(&store, &ordinary, PersonAuthority::TrustedLocal, None).unwrap();
        assert!(!appended);
        assert_eq!(retried.id, posted.id);
        assert_eq!(latest_applied_claim(&store.readers.get(), thread, "thread.state-set").unwrap().unwrap().id, closed.id);
        assert_eq!(
            append_reply(&store, &first, PersonAuthority::TrustedLocal, None)
                .unwrap()
                .0
                .id,
            sent.id
        );
        let inbound = input("message/inbound", agent, person, Some("message/first"));
        assert_eq!(
            super::super::messages_shard::validate_input(&inbound)
                .unwrap_err()
                .code,
            "thread-delivery-required",
        );
        let (reply, _) =
            append_reply(&store, &inbound, PersonAuthority::TrustedLocal, None).unwrap();
        assert_eq!(reply.body["fields"]["thread_quote_id"], "message/first");
        assert!(reply.body["fields"]["in_reply_to"].is_null());
        assert_eq!(
            store.latest_claim_id(thread).unwrap().as_deref(),
            Some(closed.id.as_str())
        );
        assert!(
            store
                .claims_for("message/first", Some("message.closed"))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn signed_thread_message_binds_membership_and_quote() {
        use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, fields_signing_bytes};

        let mut input = super::super::message_send_tests_shared_delivery::request();
        input
            .fields
            .insert("thread_id".into(), json!("thread/review"));
        input
            .fields
            .insert("thread_quote_id".into(), json!("message/quoted"));
        input.fields.insert("thread_origin_source".into(), json!("agent/reviewer"));
        input.fields.insert("thread_origin_episode".into(), json!("native-prompt-episode"));
        add_reader_marker(&mut input);
        let actor = input.actor.as_deref().unwrap();
        let (key, _) = smallclaims::fleet::MemberKey::generate().unwrap();
        let mut signature = ClaimSignature::sign(&key, "", actor, None, vec![], now_ms() as u64);
        signature.format = Some(FIELDS_FORMAT.into());
        signature.signed_fields = [
            "content", "from", "thread_id", "thread_quote_id", "thread_origin_source",
            "thread_origin_episode", MESSAGE_READER_VERSION_FIELD, "to",
        ].into_iter().map(str::to_owned).collect();
        signature.signature = key.sign(&fields_signing_bytes(
            &input.subject,
            &input.kind,
            input.actor.as_deref(),
            &json!(input.fields),
            &signature.signed_fields,
            &signature.signer,
            None,
            &signature.key,
            &signature.chain,
            &signature.nonce,
            signature.signed_at_unix_ms,
        ));
        assert!(
            verified_signature(&input, &PersonAuthority::TrustedLocal, Some(&signature))
                .unwrap()
                .is_some()
        );
        for (field, replacement) in [
            ("thread_id", "thread/other"),
            ("thread_quote_id", "message/other"),
            ("thread_origin_source", "agent/other"),
            ("thread_origin_episode", "other-prompt-episode"),
        ] {
            let mut changed = input.clone();
            changed.fields.insert(field.into(), json!(replacement));
            assert_eq!(
                verified_signature(&changed, &PersonAuthority::TrustedLocal, Some(&signature))
                    .unwrap_err()
                    .code,
                "invalid-thread-signature",
            );
        }
    }
}
