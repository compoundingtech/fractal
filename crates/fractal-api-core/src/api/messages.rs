use crate::RetirementsStore;
use crate::DeliveryStore;
use crate::DelegationStore;
use super::*;

fn message_party_identity(store: &Store, party: &str) -> Result<String, ApiError> {
    crate::store::identities::resolve_reference_conn(
        &store.readers.get(), &normalize_message_party(party),
    ).map_err(ApiError::bad)
}

pub async fn message_delivery(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let subject = if subject.starts_with("message/") {
        subject
    } else {
        format!("message/{subject}")
    };
    let (message, sent_at, host) = {
        let (state, subject) = (state.clone(), subject.clone());
        blocking_api(move || {
            let message = state
                .store
                .message(&subject)
                .map_err(ApiError::internal)?
                .ok_or_else(|| {
                    ApiError::not_found(format!("message `{subject}` does not exist"))
                })?;
            let claims = state
                .store
                .claims_for(&subject, Some("message.sent"))
                .map_err(ApiError::internal)?;
            // A stream item's sent claim is in the messages shard, not the graph.
            let sent = match claims.into_iter().next() {
                Some(claim) => Some(claim),
                None => {
                    crate::store::messages_shard::mailbox_reads::sent_claim(&state.store, &subject)
                        .map_err(ApiError::internal)?
                }
            };
            let sent_at = sent
                .map(|claim| claim.accepted_at_unix_ms)
                .unwrap_or_default();
            let host = super::message_reads::relay_host(&state.store, &message)
                .map_err(ApiError::internal)?;
            Ok((message, sent_at, host))
        })
        .await?
    };
    // The recipient's host keeps how far it got: ask it, within the status deadline.
    let relayed = match host {
        Some(host) => super::message_reads::read(
            &state,
            BTreeMap::from([(host, vec![subject.clone()])]),
            super::message_reads::STATUS_DEADLINE,
        )
        .await
        .remove(&subject),
        None => None,
    };
    blocking_api(move || {
        Ok(Json(json!({ "id": subject, "from": message.from, "to": message.to,
            "delivery": message_delivery_for(&state.store, &message, sent_at, client_now_ms(), relayed.as_ref())
                .map_err(ApiError::internal)? })))
    }).await
}

#[derive(Deserialize)]
pub struct MessageKeyQuery {
    key: String,
}

/// The message a send's idempotency key landed as, so a client whose send went unanswered can
/// tell whether it was sent before it sends again. Only a message send or reply answers; any
/// other key is `message-not-sent`.
pub async fn message_by_key(
    State(state): State<AppState>,
    Query(query): Query<MessageKeyQuery>,
) -> Result<Json<MessageSendReceipt>, ApiError> {
    let reads = state.clone();
    let mut receipt = blocking_api(move || {
        let not_sent = || ApiError {
            status: StatusCode::NOT_FOUND,
            code: "message-not-sent".into(),
            message: format!("no message was sent with idempotency key `{}`", query.key),
            details: Box::default(),
        };
        let claim = state
            .store
            .operation_claim(&query.key)
            .map_err(ApiError::internal)?
            .filter(|claim| claim.kind == "message.sent")
            .ok_or_else(not_sent)?;
        let message = state
            .store
            .message(&claim.subject)
            .map_err(ApiError::internal)?
            .ok_or_else(not_sent)?;
        let requested_recipient = message.to.clone();
        let (recipient, recipient_identity) = receipt_recipient(&state.store, &requested_recipient)?;
        Ok(MessageSendReceipt {
            requested_recipient,
            recipient,
            recipient_identity,
            kind: Some(
                if crate::silent::is_held(&message) {
                    "silent"
                } else {
                    "wake"
                }
                .into(),
            ),
            message,
            idempotency_key: query.key.clone(),
            already_sent: true,
            sent_at: Some(client_timestamp(claim.accepted_at_unix_ms)),
        })
    })
    .await?;
    // The recipient's host keeps how far it got.
    super::message_reads::read_view(&reads, &mut receipt.message)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(receipt))
}

#[derive(Deserialize)]
pub struct SubscriptionRequestQuery {
    subscription: String,
}

pub async fn list_subscription_requests(
    State(state): State<AppState>,
    Query(query): Query<SubscriptionRequestQuery>,
) -> Result<Json<Vec<crate::model::SubscriptionRequestView>>, ApiError> {
    let subscription = if query.subscription.starts_with("subscription/") {
        query.subscription
    } else {
        format!("subscription/{}", query.subscription)
    };
    let store = state.store.clone();
    blocking_store(move || store.subscription_requests(&subscription))
        .await
        .map(Json)
}

/// Release a held subscription request or cancel an open one as a person.
pub async fn decide_subscription_request(
    State(state): State<AppState>,
    AxumPath((decision, request)): AxumPath<(String, String)>,
    Json(input): Json<crate::model::SubscriptionRequestDecision>,
) -> Result<Json<crate::model::SubscriptionRequestView>, ApiError> {
    let store = state.store.clone();
    let response =
        blocking_action(move || store.decide_subscription_request(&request, &decision, &input))
            .await?;
    signal_changed(&state);
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct RecipientQuery {
    to: String,
}

/// Native send preflight: O(1) indexed lookups for one identity, never a roster or queue fold.
pub async fn recipient(
    State(state): State<AppState>,
    Query(query): Query<RecipientQuery>,
) -> Result<Json<Value>, ApiError> {
    blocking_api(move || {
        let requested = normalize_message_party(&query.to);
        crate::model::refuse_person_recipient(&requested).map_err(ApiError::bad)?;
        let (recipient, identity) = receipt_recipient(&state.store, &requested)?;
        if requested.starts_with("agent/") {
            // These are exact subject-key reads; runtime evidence may come from another host
            // and remains proof of existence even when the seat is stopped or undeclared.
            let declared = state.store.selected_desired_token(&identity).map_err(ApiError::internal)?;
            let runtime = state.store.latest_claim(&identity, Some("runtime.observed"))
                .map_err(ApiError::internal)?;
            if declared.is_none() && runtime.is_none() {
                return Err(ApiError::bad(St3Error::new(
                    "unknown-recipient",
                    format!(
                        "recipient `{requested}` is not known to this daemon; check `fractal agents ls` and fleet replication, or explicitly use --allow-undeclared-recipient if the seat exists on another host. Nothing was sent",
                    ),
                ).with_detail("recipient", requested)));
            }
        }
        Ok(Json(json!({"recipient": recipient, "recipient_identity": identity})))
    }).await
}

pub async fn send_message(
    State(state): State<AppState>,
    Json(request): Json<MessageSendRequest>,
) -> Result<Json<MessageSendReceipt>, ApiError> {
    blocking_api(move || accept_message_receipt(&state, request, None, None).map(Json)).await
}

#[derive(Deserialize)]
pub struct MessagesPageQuery {
    to: Option<String>,
    from: Option<String>,
    subject: Option<String>,
    subject_prefix: Option<String>,
    acting_as: Option<String>,
    #[serde(default)]
    include_closed: bool,
    cursor: Option<String>,
    limit: Option<usize>,
    /// A seat delivery process's report; see [`delivery_presence`].
    delivery: Option<String>,
}

impl MessagesPageQuery {
    fn validate_filters(&self) -> Result<(), ApiError> {
        if (self.subject.is_some() || self.subject_prefix.is_some()) && self.from.is_none() {
            return Err(ApiError::bad(St3Error::new(
                "validation-failed",
                "subject filters need a sender filter",
            )));
        }
        if self.subject.is_some() && self.subject_prefix.is_some() {
            return Err(ApiError::bad(St3Error::new(
                "validation-failed",
                "subject and subject_prefix are mutually exclusive",
            )));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct MessagesPageCursor {
    /// Where the next page starts in the graph. A cursor past the graph names its position in
    /// `stream_after` instead and leaves this out: every earlier build needs it, so it refuses
    /// such a cursor as invalid, and one before the messages stream doesn't read a stream position
    /// as a graph index and end the listing early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<u64>,
    /// Where the next page starts in the messages stream, at or past `STREAM_INDEX_BASE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream_after: Option<u64>,
    through: u64,
    to: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    subject_prefix: Option<String>,
    #[serde(default)]
    acting_as: Option<String>,
    include_closed: bool,
    limit: usize,
}

impl MessagesPageCursor {
    /// A cursor to the page after `after`, in the store it falls in.
    fn at(after: u64) -> (Option<u64>, Option<u64>) {
        if after >= crate::store::messages_shard::STREAM_INDEX_BASE {
            (None, Some(after))
        } else {
            (Some(after), None)
        }
    }

    /// Where the page starts. A 4a build (v0.3.25) still gives a stream position in `after`, and
    /// it reads as one. A stream position on a member without a messages shard, as when the
    /// shard's file did not open, is a gap: the listing starts over.
    fn position(&self, stream: bool) -> Result<u64, ApiError> {
        let base = crate::store::messages_shard::STREAM_INDEX_BASE;
        let after = match (self.after, self.stream_after) {
            (Some(after), None) => after,
            (None, Some(after)) if after >= base => after,
            _ => return Err(ApiError::bad(St3Error::new("validation-failed", "invalid message cursor"))),
        };
        if after >= base && !stream {
            return Err(ApiError {
                status: StatusCode::GONE,
                code: "cursor-gap".into(),
                message: "this member has no messages stream to continue the listing in; start it over".into(),
                details: Box::new(serde_json::Map::from_iter([("full_resync".into(), Value::Bool(true))])),
            });
        }
        Ok(after)
    }

    fn validate(
        &self,
        query: &MessagesPageQuery,
        to: Option<&str>,
        from: Option<&str>,
        acting_as: Option<&str>,
        limit: usize,
    ) -> Result<(), ApiError> {
        let matches = self.to.as_deref() == to
            && self.acting_as.as_deref() == acting_as
            && self.from.as_deref() == from
            && self.subject == query.subject
            && self.subject_prefix == query.subject_prefix
            && self.include_closed == query.include_closed
            && self.limit == limit;
        if !matches {
            return Err(ApiError::bad(St3Error::new(
                "validation-failed",
                "message cursor filters changed",
            )));
        }
        Ok(())
    }
}

pub async fn list_messages_page(
    State(state): State<AppState>,
    Query(query): Query<MessagesPageQuery>,
    peer: Option<Extension<NativeDeliveryPeer>>,
) -> Result<Json<MessagePage>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let peer = peer.map(|Extension(mut peer)| {
        peer.agent = message_party_identity(&state.store, &peer.agent)?;
        Ok::<_, ApiError>(Extension(peer))
    }).transpose()?;
    let to = query.to.as_deref().map(|party| message_party_identity(&state.store, party)).transpose()?;
    let from = query.from.as_deref().map(|party| message_party_identity(&state.store, party)).transpose()?;
    let acting_as = query.acting_as.as_deref().map(|party| message_party_identity(&state.store, party)).transpose()?;
    query.validate_filters()?;
    if acting_as.is_some() && to.is_none() {
        return Err(ApiError::bad(St3Error::new(
            "validation-failed", "acting_as needs one explicit mailbox",
        )));
    }
    if let (Some(to), Some(actor)) = (to.as_deref(), acting_as.as_deref()) {
        let retirement = if to.starts_with("agent/") {
            state.store.agent_retirement(to).map_err(ApiError::bad)?
        } else { None };
        let permitted = if let Some(retirement) = retirement.as_ref() {
            retirement.body["fields"]["successor"].as_str()
                .map(|successor| message_party_identity(&state.store, successor)).transpose()?.as_deref() == Some(actor)
        } else { to == actor };
        if !permitted {
            return Err(ApiError::bad(St3Error::new("wrong-message-recipient", format!(
                "`{actor}` has no mail custody for `{to}`"
            ))));
        }
    }
    if let (Some(to), Some(report)) = (to.as_deref(), query.delivery.as_deref()) {
        delivery_presence::record(to, report);
    } else {
        record_legacy_poll(
            peer.as_ref().map(|Extension(peer)| peer),
            to.as_deref(),
            query.include_closed,
        );
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(|encoded| {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded.strip_prefix("messages/").unwrap_or(""))
                .map_err(|_| {
                    ApiError::bad(St3Error::new("validation-failed", "invalid message cursor"))
                })?;
            let cursor: MessagesPageCursor = serde_json::from_slice(&bytes).map_err(|_| {
                ApiError::bad(St3Error::new("validation-failed", "invalid message cursor"))
            })?;
            cursor.validate(&query, to.as_deref(), from.as_deref(), acting_as.as_deref(), limit)?;
            Ok(cursor)
        })
        .transpose()?;
    let through = match &cursor {
        Some(cursor) => cursor.through,
        None => state.store.index().map_err(ApiError::internal)?,
    };
    let stream = crate::store::messages_shard::shard(&state.store).is_some();
    let after = cursor.as_ref().map(|cursor| cursor.position(stream)).transpose()?;
    let subject = query.subject.clone();
    let subject_prefix = query.subject_prefix.clone();
    let store = state.store.clone();
    let cursor_to = to.clone();
    let cursor_from = from.clone();
    let cursor_actor = acting_as.clone();
    let native = !query.include_closed
        && peer
            .as_ref()
            .is_some_and(|peer| to.as_deref() == Some(peer.0.agent.as_str()));
    let include_closed = query.include_closed;
    let (mut items, next_after) = blocking_store(move || {
        let filtered = from.is_some() || subject.is_some() || subject_prefix.is_some();
        let (mut items, next_after) = if filtered {
            store.messages_page_filtered(
                to.as_deref(),
                from.as_deref(),
                subject.as_deref(),
                subject_prefix.as_deref(),
                query.include_closed,
                after,
                through,
                limit,
            )?
        } else if native {
            store.messages_page_for_delivery(to.as_deref().unwrap(), after, through, limit)?
        } else {
            store.messages_page(to.as_deref(), query.include_closed, after, through, limit)?
        };
        mailbox::hold_pre_boot_mail(
            &store, peer.as_ref().map(|peer| &peer.0), to.as_deref(), &mut items,
        )?;
        Ok((items, next_after))
    })
    .await?;
    // A seat's own delivery poll reads its own host's records. Any other page asks each
    // message's seat's host; a page of open mail can come back shorter than `limit` when a host
    // says some of it is closed, and the cursor goes on from where the records left off.
    if !native {
        super::message_reads::read_views(&state, &mut items, !include_closed)
            .await
            .map_err(ApiError::internal)?;
    }
    let next_cursor = next_after
        .map(|after| {
            let (after, stream_after) = MessagesPageCursor::at(after);
            let cursor = MessagesPageCursor {
                after,
                stream_after,
                through,
                to: cursor_to.clone(),
                from: cursor_from.clone(),
                subject: query.subject.clone(),
                subject_prefix: query.subject_prefix.clone(),
                acting_as: cursor_actor.clone(),
                include_closed: query.include_closed,
                limit,
            };
            serde_json::to_vec(&cursor).map(|bytes| {
                format!(
                    "messages/{}",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
                )
            })
        })
        .transpose()
        .map_err(ApiError::internal)?;
    Ok(Json(MessagePage {
        items,
        has_more: next_cursor.is_some(),
        next_cursor,
        limit,
    }))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Query(query): Query<MessagesQuery>,
    peer: Option<Extension<NativeDeliveryPeer>>,
) -> Result<Json<Vec<MessageView>>, ApiError> {
    let recipient = query.to.as_deref().map(|party| message_party_identity(&state.store, party)).transpose()?;
    let peer = peer.map(|Extension(mut peer)| {
        peer.agent = message_party_identity(&state.store, &peer.agent)?;
        Ok::<_, ApiError>(Extension(peer))
    }).transpose()?;
    record_legacy_poll(
        peer.as_ref().map(|Extension(peer)| peer),
        recipient.as_deref(),
        query.include_closed,
    );
    let store = state.store.clone();
    let native = !query.include_closed
        && peer
            .as_ref()
            .is_some_and(|peer| recipient.as_deref() == Some(peer.0.agent.as_str()));
    let include_closed = query.include_closed;
    let mut messages = blocking_store(move || {
        let mut messages = if native {
            store.messages_for_delivery_through(recipient.as_deref().unwrap(), store.index()?)?
        } else {
            store.messages(recipient.as_deref(), query.include_closed)?
        };
        mailbox::hold_pre_boot_mail(
            &store, peer.as_ref().map(|peer| &peer.0), recipient.as_deref(), &mut messages,
        )?;
        Ok(messages)
    })
    .await?;
    // A seat's own delivery poll reads its own host's records. Any other list asks each
    // message's seat's host.
    if !native {
        super::message_reads::read_views(&state, &mut messages, !include_closed)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(Json(messages))
}

pub async fn read_message(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
) -> Result<Json<MessageRead>, ApiError> {
    let subject = if subject.starts_with("message/") {
        subject
    } else {
        format!("message/{subject}")
    };
    let store = state.store.clone();
    let lookup = subject.clone();
    let mut message = blocking_store(move || store.message(&lookup))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("message `{subject}` does not exist")))?;
    // The recipient's host keeps how far it got.
    super::message_reads::read_view(&state, &mut message)
        .await
        .map_err(ApiError::internal)?;
    let provenance = {
        let state = state.clone();
        let lookup = message.subject.clone();
        blocking_store(move || {
            Ok(state
                .store
                .latest_claim(&lookup, Some("message.sent"))?
                .and_then(|claim| client_v0::message_provenance(&state, &claim)))
        })
        .await?
    };
    Ok(Json(MessageRead {
        message,
        provenance,
    }))
}

pub async fn post_message_claim(
    State(state): State<AppState>,
    AxumPath(message_id): AxumPath<String>,
    Json(request): Json<MessageLifecycleRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    let subject = message_subject(&message_id);
    let kind = match request.lifecycle.as_str() {
        "staged" => "message.staged",
        "delivered" => "message.delivered",
        "read" => "message.read",
        "closed" => "message.closed",
        other => {
            return Err(ApiError::bad(St3Error::new(
                "invalid-message-lifecycle",
                format!("message lifecycle `{other}` is not registered"),
            )));
        }
    };
    let store = state.store.clone();
    let record = blocking_api(move || {
        // One message, not every message the store has ever held: a lifecycle post read and folded
        // the whole mailbox, a quarter second on a busy host's store, while holding up the next write.
        let message = store
            .message(&subject)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("message `{subject}` does not exist")))?;
        let actor = request.actor.ok_or_else(|| {
            ApiError::bad(St3Error::new(
                "missing-message-actor",
                "message lifecycle transitions require the recipient actor",
            ))
        })?;
        let actor = message_party_identity(&store, &actor)?;
        let recipient = message_party_identity(&store, &message.to)?;
        let retirement = if recipient.starts_with("agent/") {
            store.agent_retirement(&recipient).map_err(ApiError::bad)?
        } else { None };
        let successor = retirement.as_ref()
            .and_then(|claim| claim.body["fields"]["successor"].as_str())
            .map(|successor| message_party_identity(&store, successor)).transpose()?;
        let acting_for = retirement.as_ref().and_then(|_| {
            (matches!(kind, "message.read" | "message.closed")
                && successor.as_deref() == Some(actor.as_str()))
                .then(|| recipient.clone())
        });
        if acting_for.is_none() && (retirement.is_some() || actor != recipient)
            && !request
                .delegation
                .as_ref()
                .is_some_and(|proof| proof.person == message.to && actor.starts_with("agent/"))
        {
            return Err(ApiError::bad(St3Error::new(
                "wrong-message-recipient",
                format!(
                    "message `{subject}` belongs to `{}`, not `{actor}`",
                    message.to
                ),
            )));
        }
        let mut fields =
            BTreeMap::from([("status".into(), Value::String(request.lifecycle.clone()))]);
        let mut evidence = request.evidence;
        if let Some(acting_for) = acting_for {
            fields.insert("acting_for".into(), Value::String(acting_for));
            evidence.push(retirement.as_ref().unwrap().id.clone());
        }
        if let Some(proof) = &request.delegation {
            crate::store::delegation::add_fields(&mut fields, proof);
            store.delegation_evidence(&mut evidence, proof).map_err(ApiError::bad)?;
        }
        if kind == "message.staged" {
            fields.insert("recipient".into(), Value::String(actor.clone()));
            if let Some(transport) = request.transport {
                fields.insert("transport".into(), Value::String(transport));
            }
            if let Some(runtime_id) = request.runtime_id {
                fields.insert("runtime_id".into(), Value::String(runtime_id));
            }
        }
        // A stream item's lifecycle is this member's record, not a graph claim.
        if message.created_index >= crate::store::messages_shard::STREAM_INDEX_BASE {
            if request.expected_subject.is_some() {
                return Err(ApiError::bad(St3Error::new(
                    "expected-subject-unsupported",
                    format!("`{subject}` is a stream item: it has no graph claim to compare with"),
                )));
            }
            // A successor's custody step and a delegated step pass the rules the graph applies to
            // their claims, read against the stream item; the step itself stays this member's
            // record, with its custody or delegation proof in its fields.
            let input = ClaimInput {
                subject: subject.clone(),
                kind: kind.into(),
                actor: Some(actor.clone()),
                fields: fields.clone(),
                evidence: evidence.clone(),
                expected_subject: None,
                idempotency_key: None,
            };
            crate::store::retirements::admit_mail_claim_to(&store.readers.get(), &input, &message)
                .map_err(ApiError::bad)?;
            if fields.contains_key("delegation") || fields.contains_key("acted_for") {
                crate::store::delegation::validate_stream_write(
                    &store, &subject, kind, Some(&actor), &fields, &evidence,
                )
                .map_err(ApiError::bad)?;
            }
            let appended = crate::store::messages_shard::record_lifecycle(
                &store, &subject, kind, Some(&actor), &fields,
            )
            .map_err(ApiError::bad)?;
            let record = crate::store::messages_shard::lifecycle_receipt(
                &store.graph.origin, &subject, kind, Some(&actor), &fields,
            );
            return Ok((record, appended, is_work_wake(&message.tags)));
        }
        let (record, appended) = store
            .append_claim_outcome(&ClaimInput {
                subject,
                kind: kind.into(),
                actor: Some(actor),
                fields,
                evidence,
                expected_subject: request.expected_subject,
                idempotency_key: Some(request.idempotency_key),
            })
            .map_err(ApiError::bad)?;
        Ok((record, appended, is_work_wake(&message.tags)))
    })
    .await?;
    let (record, appended, work_wake) = record;
    // An idempotent repeat or an already-settled transition changes nothing a reader can see.
    // Signalling it anyway re-reads every subscribed seat's mailbox (#1085).
    if appended {
        signal_message_changed(&state, kind, work_wake);
    }
    Ok(Json(record))
}

#[cfg(test)]
mod page_cursor_tests {
    use super::*;

    #[test]
    fn message_cursor_rejects_changed_filters() {
        let cursor: MessagesPageCursor = serde_json::from_value(json!({
            "after": 10, "through": 100, "to": "agent/reviewer",
            "from": "agent/ada", "subject": "review", "subject_prefix": null,
            "include_closed": true, "limit": 2
        }))
        .unwrap();
        let binding = json!({
            "to": "agent/reviewer", "from": "agent/ada", "subject": "review",
            "include_closed": true, "limit": 2
        });
        let validate = |value: Value| {
            let query: MessagesPageQuery = serde_json::from_value(value).unwrap();
            let to = query.to.as_deref().map(normalize_message_party);
            let from = query.from.as_deref().map(normalize_message_party);
            cursor.validate(&query, to.as_deref(), from.as_deref(), None, query.limit.unwrap())
        };
        assert!(validate(binding.clone()).is_ok());
        for (field, value) in [
            ("to", json!("agent/operator")),
            ("from", json!("agent/alex")),
            ("from", Value::Null),
            ("subject", json!("other")),
            ("subject_prefix", json!("rev")),
            ("include_closed", json!(false)),
            ("limit", json!(3)),
        ] {
            let mut changed = binding.clone();
            changed[field] = value;
            let error = validate(changed).expect_err("accepted changed cursor filters");
            assert_eq!(error.code, "validation-failed");
            assert_eq!(error.message, "message cursor filters changed");
        }
    }

    /// A cursor past the graph names its stream position apart, so an earlier build, whose cursor
    /// needs `after`, refuses it rather than misread it; this build reads both,
    /// and a member without a shard answers a stream cursor with a cursor-gap.
    #[test]
    fn a_stream_cursor_is_refused_by_an_older_build_and_a_gap_without_a_shard() {
        /// The cursor as an earlier build reads it (v0.3.25 and before).
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct OlderCursor {
            after: u64,
            through: u64,
            to: Option<String>,
            include_closed: bool,
            limit: usize,
        }
        let base = crate::store::messages_shard::STREAM_INDEX_BASE;
        let encode = |after: u64| {
            let (after, stream_after) = MessagesPageCursor::at(after);
            serde_json::to_vec(&MessagesPageCursor {
                after, stream_after, through: 100, to: Some("agent/reviewer".into()), from: None,
                subject: None, subject_prefix: None, acting_as: None, include_closed: false, limit: 2,
            })
            .unwrap()
        };
        let graph = encode(10);
        assert_eq!(serde_json::from_slice::<OlderCursor>(&graph).unwrap().after, 10, "a graph cursor reads as before");
        let stream = encode(base + 7);
        assert!(serde_json::from_slice::<OlderCursor>(&stream).is_err(), "an earlier build refuses a stream cursor");
        let read = |bytes: &[u8]| serde_json::from_slice::<MessagesPageCursor>(bytes).unwrap();
        assert_eq!(read(&graph).position(false).unwrap(), 10);
        assert_eq!(read(&stream).position(true).unwrap(), base + 7);
        let gap = read(&stream).position(false).unwrap_err();
        assert_eq!((gap.status, gap.code.as_str()), (StatusCode::GONE, "cursor-gap"));
        // An older build's own cursor, `after` alone, still reads.
        let older: MessagesPageCursor = serde_json::from_value(json!({
            "after": 10, "through": 100, "to": null, "include_closed": true, "limit": 2
        }))
        .unwrap();
        assert_eq!(older.position(true).unwrap(), 10);
        // A 4a build's stream cursor names its position in `after`, and reads as one.
        let from_4a: MessagesPageCursor = serde_json::from_value(json!({
            "after": base + 3, "through": 100, "to": null, "include_closed": true, "limit": 2
        }))
        .unwrap();
        assert_eq!(from_4a.position(true).unwrap(), base + 3);
        assert_eq!(from_4a.position(false).unwrap_err().code, "cursor-gap");
        // A graph index in `stream_after`, neither, or both, is no cursor a build gave.
        for bad in [json!({"stream_after": 5}), json!({}), json!({"after": 1, "stream_after": base})] {
            let mut value = json!({"through": 100, "to": null, "include_closed": true, "limit": 2});
            value.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let cursor: MessagesPageCursor = serde_json::from_value(value).unwrap();
            assert_eq!(cursor.position(true).unwrap_err().code, "validation-failed");
        }
    }

    #[test]
    fn message_cursor_prefix_binding_and_legacy_defaults() {
        let mut cursor: MessagesPageCursor = serde_json::from_value(json!({
            "after": 10, "through": 100, "to": null,
            "include_closed": true, "limit": 2
        }))
        .unwrap();
        let mut query: MessagesPageQuery = serde_json::from_value(json!({
            "include_closed": true, "limit": 2
        }))
        .unwrap();
        assert!(cursor.validate(&query, None, None, None, 2).is_ok());
        query.subject_prefix = Some("literal_%".into());
        assert!(cursor.validate(&query, None, None, None, 2).is_err());
        cursor.subject_prefix = query.subject_prefix.clone();
        assert!(cursor.validate(&query, None, None, None, 2).is_ok());
        query.subject_prefix = Some("literal".into());
        assert!(cursor.validate(&query, None, None, None, 2).is_err());
    }
}

#[cfg(test)]
mod subject_filter_tests {
    use super::*;

    #[test]
    fn subject_filters_require_sender_before_store_access() {
        for field in ["subject", "subject_prefix"] {
            let query: MessagesPageQuery =
                serde_json::from_value(json!({(field): "review", "to": "agent/reviewer"}))
                    .unwrap();
            let error = query.validate_filters().expect_err("accepted subject-only query");
            assert_eq!(error.code, "validation-failed");
            assert_eq!(error.message, "subject filters need a sender filter");
        }
    }
}
