//! Where a sent message goes (db break-up step 4c-2b): the messages shard, as a stream item, when
//! its recipient's host reads stream mail as it reads graph mail, and otherwise the graph, as a
//! legacy `message.sent` claim, as every message went before.
//!
//! The recipient's host is where the seat's mail is delivered and woken and where its lifecycle
//! is recorded; other members ask it (`relay_host`). It is the host the seat's declaration
//! places it on, from the replicated `desired` table, so every member answers alike. It reads
//! stream mail when it is this member or its current start advertises `messages_stream_inbox`
//! ([`messages_shard::inbox_hosts`]). Anything else stays a graph claim: no shard, a member
//! leaving its fleet (a send then lands as a claim and is never fenced), a message the stream
//! may not carry (a person's or an assistant's), one with attachments (a long message's text or
//! an image), a delivery probe's, a recipient without a host (a person, an undeclared or retired
//! seat), or a host that does not advertise the inbox.
//!
//! Work wakes, replies that settle a parent, and senders whose evidence or predecessors cite
//! other claims stay on the graph until 4c-3; they call the graph append, not [`send_message`].

use super::*;

/// Why `input` stays a legacy graph claim; `None` sends it to the stream.
pub fn legacy_reason(store: &Store, input: &ClaimInput) -> Result<Option<&'static str>> {
    if input.kind != "message.sent" {
        return Ok(Some("not-a-message"));
    }
    // A mission run's send stays on the graph append, where its run authority decides it
    // (docs/st3/program-engine.md §3); the stream append has no such decision.
    let sender = input.fields.get("from").and_then(Value::as_str);
    if [input.actor.as_deref(), sender].into_iter().flatten().any(|party| party.starts_with("mission-run/")) {
        return Ok(Some("run-sender"));
    }
    if messages_shard::shard(store).is_none() {
        return Ok(Some("no-shard"));
    }
    if store.graph.fleet_leaving()? {
        return Ok(Some("leaving"));
    }
    if messages_shard::validate_input(input).is_err() {
        return Ok(Some("ineligible"));
    }
    // A long message's text and an image live in files its sender's member serves by the claim;
    // they stay on the graph until that path is proven for stream items.
    if input.fields.contains_key("attachments") {
        return Ok(Some("attached"));
    }
    // A delivery probe proves its mail, its alerts and their clears by their graph claims and the
    // recipient's replicated read claim (docs/st3/delivery-probes.md), as marks leave them too.
    let tags = input.fields.get("tags").and_then(Value::as_array);
    if tags.is_some_and(|tags| tags.iter().filter_map(Value::as_str).any(|tag| tag.starts_with("delivery-probe"))) {
        return Ok(Some("probe"));
    }
    let Some(to) = input.fields.get("to").and_then(Value::as_str) else {
        return Ok(Some("no-recipient"));
    };
    // Mail to a retired seat goes to the graph append, whose producer guard refuses it as before.
    let seat = identities::resolve_reference_conn(&store.readers.get(), &normalize_message_party(to))?;
    if retirements::record(&store.readers.get(), &seat, None)?.is_some() {
        return Ok(Some("retired"));
    }
    let Some(host) = recipient_host(store, to)? else {
        return Ok(Some("no-host"));
    };
    if host == store.graph.origin || messages_shard::routing_inbox_hosts(store)?.contains(&host) {
        Ok(None)
    } else {
        Ok(Some("host-without-inbox"))
    }
}

/// The host `to`'s declaration places it on: a stopping seat's last member, and `None` for a
/// person, an undeclared seat or one stopped on purpose.
pub fn recipient_host(store: &Store, to: &str) -> Result<Option<String>> {
    let seat = identities::resolve_reference_conn(&store.readers.get(), &normalize_message_party(to))?;
    Ok(store.mailbox_member_named(&seat)?.flatten().map(|member| member.host))
}

/// Send `input`, a `message.sent`, where [`legacy_reason`] says: to the stream, or to the graph.
/// Either append replays the other store's message under the same key or subject, under the
/// send gate (`message_sent`). A stream append that fails for any reason but a changed request
/// under its key goes to the graph instead; that append checks the stream first, so a stream
/// write that committed before its error replays rather than doubling the message.
pub fn send_message(store: &Store, input: &ClaimInput) -> Result<(ClaimRecord, bool), FractalError> {
    let routed = legacy_reason(store, input).map_err(internal)?.is_none();
    if routed && let Some(shard) = messages_shard::shard(store) {
        match shard.append_claim_outcome(input) {
            Err(error) if error.code != "idempotency-mismatch" => {}
            sent => return sent,
        }
    }
    store.append_claim_outcome(input)
}

/// [`send_message`], answering the claim it wrote or replayed, as [`Store::append_claim`] does.
pub fn send(store: &Store, input: &ClaimInput) -> Result<ClaimRecord, FractalError> {
    send_message(store, input).map(|(claim, _)| claim)
}

/// [`send_message`] for a sender inside a graph transaction, which cannot write to the shard: a
/// message `route` (its [`legacy_reason`], read before the transaction) sends to the stream is
/// queued in the outbox, which the daemon drains to the shard after the commit; any other is
/// appended to the graph in the transaction, as before. True when it wrote or queued.
pub fn send_message_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    input: &ClaimInput,
    route: Option<&'static str>,
) -> Result<bool, FractalError> {
    if route.is_none() {
        return message_outbox::enqueue_tx(transaction, input);
    }
    append_claim_tx(
        transaction,
        origin,
        &input.subject,
        &input.kind,
        input.actor.as_deref(),
        &json!({"fields": input.fields, "evidence": input.evidence}),
        &[],
        None,
    )
    .map(|_| true)
    .map_err(claim_append_error)
}
