//! The runtime of the messages shard: a separate store that holds one immutable `message.sent`
//! claim per message, replicated beside the graph under the shard ID `messages`.
//!
//! The shard takes its fleet from the graph (`bind_fleet_authority`) and has no checkpoint,
//! so this runtime keeps no tables of its own and never seals. It admits only the messages the
//! stream may carry: a `message.sent` the schema accepts, written by an agent or the daemon.
//! A person's device-signed message and an external assistant's message stay graph claims,
//! because judging their signatures and grants needs principal facts only the graph holds.
use serde_json::{Value, json};
use crate::model::MessageView;
use smallclaims::claim::{ClaimInput, ClaimRecord, ReplicaBatch};
use smallclaims::error::Error as St3Error;
use smallclaims::store::runtime::{IncrementalProjection, LegacyDigestTable, Runtime};
use smallclaims::store::{
    DropPlan, ReplicatedClaimAdmission, SealedSet, Store as GraphStore, append_claim_record_tx,
};
use rusqlite::{Connection, Transaction};
use smallclaims::shards::{ShardId, ShardStores};
use smallclaims::store::FleetAuthority;
use rusqlite::OptionalExtension as _;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

/// The shard's ID, its directory under the store root and its name on the replication wire.
pub const SHARD_ID: &str = "messages";

/// Where a stream item's `created_index` starts. A stream item has no graph index; its index is
/// this plus its index in the shard, so it never collides with a graph message and a page that
/// pages past every graph message continues into the stream.
pub const STREAM_INDEX_BASE: u64 = 1 << 48;

/// The shard's own tables. `stream_messages` indexes each message by recipient as it is
/// admitted; on open it catches up only on claims newer than its newest row, such as those a
/// rolled-back build admitted without the trigger. `stream_lifecycle` is this member's record of a stream item's delivery: it is
/// local, never replicated and outside every digest, as a lifecycle claim would not be.
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha5ce4a3a3ab1 owner=fractal_store::store::messages_shard TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS stream_messages (
    subject TEXT PRIMARY KEY,
    recipient TEXT NOT NULL,
    sender TEXT NOT NULL,
    store_index INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS stream_messages_recipient
    ON stream_messages(recipient, store_index);
CREATE TRIGGER IF NOT EXISTS stream_messages_admitted AFTER INSERT ON claims
WHEN NEW.kind='message.sent' BEGIN
    INSERT OR IGNORE INTO stream_messages(subject, recipient, sender, store_index)
    VALUES (NEW.subject, COALESCE(json_extract(NEW.body, '$.fields.to'), ''),
            COALESCE(json_extract(NEW.body, '$.fields.from'), ''), NEW.store_index);
END;
CREATE TABLE IF NOT EXISTS stream_lifecycle (
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    fields TEXT NOT NULL,
    recorded_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (subject, kind, actor)
);
INSERT OR IGNORE INTO stream_messages(subject, recipient, sender, store_index)
    SELECT subject, COALESCE(json_extract(body, '$.fields.to'), ''),
           COALESCE(json_extract(body, '$.fields.from'), ''), store_index
    FROM claims WHERE kind='message.sent'
      AND store_index > COALESCE((SELECT MAX(store_index) FROM stream_messages), 0);
";

/// Where each lifecycle kind stands; a stream item moves only forward through them.
fn lifecycle_rank(kind: &str) -> Option<u8> {
    match kind {
        "message.sent" => Some(0),
        "message.staged" => Some(1),
        "message.delivered" => Some(2),
        "message.read" => Some(3),
        "message.closed" => Some(4),
        _ => None,
    }
}

/// The stream item `subject` with its index in the shard, if the shard holds it.
fn stream_item(shard: &GraphStore, subject: &str) -> anyhow::Result<Option<(ClaimRecord, u64)>> {
    let claim = shard.claims_for(subject, Some("message.sent"))?.into_iter().next();
    Ok(claim.map(|claim| {
        let index = claim.store_index;
        (claim, index)
    }))
}

/// This member's lifecycle records for a stream item, in the order they move it.
fn lifecycle_records(shard: &GraphStore, subject: &str) -> anyhow::Result<Vec<(String, Value)>> {
    let connection = shard.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT kind, fields FROM stream_lifecycle WHERE subject=?1 ORDER BY recorded_at_unix_ms, kind",
    )?;
    let mut records = statement
        .query_map([subject], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(kind, fields)| Ok((kind, serde_json::from_str(&fields)?)))
        .collect::<anyhow::Result<Vec<(String, Value)>>>()?;
    records.sort_by_key(|(kind, _)| lifecycle_rank(kind));
    Ok(records)
}

/// Whether the shard holds the message `subject`.
pub fn holds(graph: &super::Store, subject: &str) -> anyhow::Result<bool> {
    let Some(shard) = shard(graph) else {
        return Ok(false);
    };
    Ok(stream_item(&shard, subject)?.is_some())
}

/// A stream item as a message: its sent fields, with this member's lifecycle over them, so its
/// status is as far as this member has taken it. Its `created_index` is past every graph index.
pub fn message_view(graph: &super::Store, subject: &str) -> anyhow::Result<Option<MessageView>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    let Some((sent, index)) = stream_item(&shard, subject)? else {
        return Ok(None);
    };
    let mut actual = sent.body.get("fields").cloned().unwrap_or_else(|| json!({}));
    for (_, fields) in lifecycle_records(&shard, subject)? {
        if let (Some(actual), Value::Object(fields)) = (actual.as_object_mut(), fields) {
            actual.extend(fields);
        }
    }
    let connection = graph.graph.readers.get();
    super::message_view_from(&connection, subject, STREAM_INDEX_BASE + index, actual, None).map(Some)
}

/// The stream items addressed to any of `names`, after shard index `after`, oldest first, at
/// most `limit` of them, with their shard indexes.
pub fn recipient_page(
    graph: &super::Store,
    names: &[String],
    after: u64,
    limit: usize,
) -> anyhow::Result<Vec<(String, u64)>> {
    let Some(shard) = shard(graph) else {
        return Ok(Vec::new());
    };
    let connection = shard.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT subject, store_index FROM stream_messages
         WHERE recipient IN (SELECT value FROM json_each(?1)) AND store_index>?2
         ORDER BY store_index LIMIT ?3",
    )?;
    let rows = statement
        .query_map(
            rusqlite::params![serde_json::to_string(names)?, after, limit],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// What a lifecycle step for a stream item answers in place of a claim: a receipt that names the
/// step, `offset-receipt/<sha256>`, the same for every repeat of it. It is not in the graph.
pub fn lifecycle_receipt(
    origin: &str,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    fields: &std::collections::BTreeMap<String, Value>,
) -> ClaimRecord {
    use sha2::Digest as _;
    let identity = format!("{subject}\n{kind}\n{}", actor.unwrap_or(""));
    ClaimRecord {
        id: format!("offset-receipt/{}", hex::encode(sha2::Sha256::digest(identity.as_bytes()))),
        store_index: 0,
        batch_id: String::new(),
        subject: subject.to_owned(),
        kind: kind.to_owned(),
        origin: origin.to_owned(),
        actor: actor.map(str::to_owned),
        operation_id: None,
        request_digest: None,
        body: json!({ "fields": fields, "evidence": [] }),
        predecessors: Vec::new(),
        accepted_at_unix_ms: smallclaims::store::now_ms(),
    }
}

/// A delivery channel's receipt for a stream item, admitted under the fence a graph receipt
/// is: a live incarnation, the lease, and the current owner's token. The shard is a separate
/// database, so the fence is read just before the record rather than in its transaction.
pub fn record_fenced_lifecycle(
    graph: &super::Store,
    fence: &crate::mailbox::Fence,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    fields: &std::collections::BTreeMap<String, Value>,
) -> Result<bool, St3Error> {
    {
        let connection = graph.readers.get();
        super::check_mailbox_fence(&connection, fence, &graph.graph.origin)?;
    }
    record_lifecycle(graph, subject, kind, actor, fields)
}

/// Record a lifecycle step a member took for a stream item: staged, delivered, read or closed.
/// It is this member's record, not a claim, so nothing replicates. A repeat or a step behind
/// where the item already is changes nothing and returns false, as a settled claim would.
pub fn record_lifecycle(
    graph: &super::Store,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    fields: &std::collections::BTreeMap<String, Value>,
) -> Result<bool, St3Error> {
    let internal = |error: anyhow::Error| St3Error::new("internal", error.to_string());
    let shard = shard(graph)
        .ok_or_else(|| St3Error::new("missing-message", "this member holds no messages shard"))?;
    let rank = lifecycle_rank(kind)
        .filter(|rank| *rank > 0)
        .ok_or_else(|| St3Error::new("invalid-message-transition", format!("`{kind}` is not a lifecycle step")))?;
    if matches!(kind, "message.read" | "message.closed") && actor.is_none() {
        return Err(St3Error::new("missing-message-actor", format!("`{kind}` needs an actor")));
    }
    let Some((sent, _)) = stream_item(&shard, subject).map_err(internal)? else {
        return Err(St3Error::new("missing-message", format!("no stream item `{subject}`")));
    };
    let current = lifecycle_records(&shard, subject)
        .map_err(internal)?
        .iter()
        .filter_map(|(kind, _)| lifecycle_rank(kind))
        .max()
        .unwrap_or(0);
    // A replayed or earlier step settles to what is recorded, as a graph receipt does.
    if rank <= current {
        return Ok(false);
    }
    // Forward steps follow the graph's message transitions, so a stream item moves as the same
    // mail would through the graph.
    let recipient = sent.body.pointer("/fields/to").and_then(Value::as_str);
    let valid = matches!((current, rank), (0, 1) | (1, 2) | (0, 2) | (2, 3) | (3, 4))
        // A person's displayed message has no harness handoff; their read is its delivery.
        || (rank == 3 && current <= 1 && actor.is_some_and(|actor| actor.starts_with("person/")) && actor == recipient)
        || (rank == 3 && current <= 1 && fields.contains_key("acting_for"))
        || (rank == 4 && current <= 1 && actor == Some("daemon/runtime"))
        || (rank == 4 && fields.contains_key("delegation"));
    if !valid {
        const STATUS: [&str; 5] = ["sent", "staged", "delivered", "read", "closed"];
        return Err(St3Error::new(
            "invalid-message-transition",
            format!(
                "message `{subject}` cannot move from `{}` to `{}`",
                STATUS[usize::from(current)],
                STATUS[usize::from(rank)]
            ),
        ));
    }
    let fields = serde_json::to_string(fields).map_err(|error| internal(error.into()))?;
    let connection = shard.connection.write();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h2314176b1be8 owner=fractal_store::store::messages_shard TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO stream_lifecycle(subject, kind, actor, fields, recorded_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![subject, kind, actor.unwrap_or(""), fields, smallclaims::store::now_ms() as i64],
        )
        .map(|inserted| inserted > 0)
        .map_err(|error| internal(error.into()))
}

/// The graph, as the shard's fleet authority. It is held weakly: the daemon owns the graph,
/// and the shard must not keep it open after the daemon lets it go.
struct GraphAuthority(std::sync::Weak<super::Store>);

impl GraphAuthority {
    fn graph(&self) -> anyhow::Result<Arc<super::Store>> {
        self.0
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("the graph this shard takes its fleet from has closed"))
    }
}

impl FleetAuthority for GraphAuthority {
    fn fleet_id(&self) -> anyhow::Result<Option<String>> {
        FleetAuthority::fleet_id(&self.graph()?.graph)
    }

    fn fleet_generation(&self) -> anyhow::Result<i64> {
        FleetAuthority::fleet_generation(&self.graph()?.graph)
    }

    fn fleet_membership(&self) -> anyhow::Result<smallclaims::fleet::Membership> {
        FleetAuthority::fleet_membership(&self.graph()?.graph)
    }

    fn fleet_membership_sealed(&self) -> anyhow::Result<smallclaims::fleet::Membership> {
        FleetAuthority::fleet_membership_sealed(&self.graph()?.graph)
    }

    fn is_default_store(&self) -> anyhow::Result<bool> {
        FleetAuthority::is_default_store(&self.graph()?.graph)
    }
}

/// The hosts whose daemon advertises holding the messages shard: `features.messages_stream=1`
/// on the newest `daemon.started` the host wrote in its current incarnation. A member's
/// earlier incarnation, or a host that never advertised, does not count, so a host that rolls
/// back to a build without the shard stops counting once its own new start replicates. Peers
/// configured without membership count by their newest start. This host is left out.
pub fn advertising_hosts(
    graph: &super::Store,
    configured_peers: &[String],
) -> anyhow::Result<BTreeSet<String>> {
    let membership = graph.graph.fleet_membership()?;
    let mut hosts = membership
        .incarnations()
        .filter(|member| member.end.is_none())
        .map(|member| (member.name.clone(), member.start))
        .collect::<BTreeMap<_, _>>();
    for peer in configured_peers {
        hosts.entry(peer.clone()).or_insert(0);
    }
    hosts.remove(&graph.graph.origin);
    let connection = graph.graph.readers.get();
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbe3ddc31e72e owner=fractal_store::store::messages_shard TODO=trace-statement-and-value-representation */ &smallclaims::store::canonical_sql(
        "SELECT json_extract(claims.body,'$.fields.features.messages_stream')=1
         FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
             AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    ))?;
    let mut advertising = BTreeSet::new();
    for (host, start) in hosts {
        let holds: Option<Option<bool>> = statement
            .query_row(/* sqlite-large-value: allow-for-now lv-h516c5fb23a80 owner=fractal_store::store::messages_shard TODO=trace-statement-and-value-representation */ rusqlite::params![format!("daemon/{host}"), host, start], |row| row.get(0))
            .optional()?;
        if holds.flatten() == Some(true) {
            advertising.insert(host);
        }
    }
    Ok(advertising)
}

/// Where the graph keeps its open messages shard, for the request handlers that serve it. Empty
/// until the daemon opens the shard; a process that never opens it serves no shard routes.
#[derive(Default)]
pub struct Slot(std::sync::OnceLock<Arc<GraphStore>>);

impl Slot {
    pub fn get(&self) -> Option<&Arc<GraphStore>> {
        self.0.get()
    }
}

/// The messages shard the graph opened, if it opened one.
pub fn shard(graph: &super::Store) -> Option<Arc<GraphStore>> {
    graph.extension::<Slot>().get().cloned()
}

/// Open the messages shard under `root`, the directory that holds the graph's database, and
/// bind it to the graph's fleet. The file is created and bound the first time; a later open
/// finds it bound. It signs with the graph's member key.
pub fn open(graph: &Arc<super::Store>, root: &Path) -> anyhow::Result<Arc<GraphStore>> {
    let id = ShardId::parse(SHARD_ID)?;
    let stores = ShardStores::open(
        root,
        &graph.graph.origin,
        |_| Box::new(MessagesRuntime),
        [id.clone()],
    )?;
    let shard = stores.get(&id).expect("the shard just opened").clone();
    shard.bind_fleet_authority(Arc::new(GraphAuthority(Arc::downgrade(graph))))?;
    let key = graph
        .graph
        .member_key
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    shard.set_member_key(key)?;
    graph
        .extension::<Slot>()
        .0
        .set(shard.clone())
        .map_err(|_| anyhow::anyhow!("the graph already has a messages shard"))?;
    Ok(shard)
}

/// What a stream item may not carry: the fields of a message an external assistant sent.
const ASSISTANT_FIELDS: &[&str] = &[
    "sender_class",
    "assistant_grant_id",
    "assistant_owner",
    "assistant_session_actor",
];

#[derive(Clone, Copy, Debug, Default)]
pub struct MessagesRuntime;

/// Refuse anything the stream may not carry, before the schema checks the fields.
fn refuse_ineligible(kind: &str, actor: Option<&str>, fields: &Value) -> Result<(), St3Error> {
    if kind != "message.sent" {
        return Err(St3Error::new(
            "shard-kind-refused",
            format!("the messages shard holds only message.sent claims, not `{kind}`"),
        ));
    }
    let from = fields.get("from").and_then(Value::as_str);
    if actor.is_some_and(|actor| actor.starts_with("person/"))
        || from.is_some_and(|from| from.starts_with("person/"))
    {
        return Err(St3Error::new(
            "shard-kind-refused",
            "a person's message stays a graph claim: its device signature is judged there",
        ));
    }
    if ASSISTANT_FIELDS.iter().any(|field| fields.get(field).is_some()) {
        return Err(St3Error::new(
            "shard-kind-refused",
            "an external assistant's message stays a graph claim: its grant is judged there",
        ));
    }
    Ok(())
}

/// Whether the messages shard admits this replicated claim.
pub fn classify_stream_item(claim: &ClaimRecord) -> Result<ReplicatedClaimAdmission, St3Error> {
    refuse_ineligible(
        &claim.kind,
        claim.actor.as_deref(),
        claim.body.get("fields").unwrap_or(&Value::Null),
    )?;
    super::classify_replicated_claim_with_registry(claim, fractal_schema::registry())
}

fn validate_input(input: &ClaimInput) -> Result<(), St3Error> {
    let fields = serde_json::to_value(&input.fields).unwrap_or(Value::Null);
    refuse_ineligible(&input.kind, input.actor.as_deref(), &fields)?;
    fractal_schema::registry()
        .validate_claim(&input.subject, &input.kind, &input.fields)
        .map(|_| ())
        .map_err(|error| St3Error::new(error.code, error.message))
}

impl Runtime for MessagesRuntime {
    fn migrate_schema(&self, _connection: &Connection) -> anyhow::Result<()> {
        Ok(())
    }

    fn create_schema(&self, connection: &Connection) -> anyhow::Result<()> {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h4fc020e207df owner=fractal_store::store::messages_shard TODO=trace-statement-and-value-representation */ SCHEMA)?;
        Ok(())
    }

    fn open_projections(
        &self,
        _transaction: &Transaction<'_>,
        _shared_memory: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn schema_digest(&self) -> String {
        format!("messages-shard-v1:{}", fractal_schema::registry().digest())
    }

    fn classify_replicated_claim(
        &self,
        _connection: &Connection,
        _batch: &ReplicaBatch,
        claim: &ClaimRecord,
    ) -> Result<ReplicatedClaimAdmission, St3Error> {
        classify_stream_item(claim)
    }

    fn classify_replicated_claim_unbound(
        &self,
        _batch: &ReplicaBatch,
        claim: &ClaimRecord,
    ) -> Option<Result<ReplicatedClaimAdmission, St3Error>> {
        // The answer depends only on the claim and this build's schema.
        Some(classify_stream_item(claim))
    }

    fn append_claim(
        &self,
        store: &GraphStore,
        input: &ClaimInput,
    ) -> Result<(ClaimRecord, bool), St3Error> {
        validate_input(input)?;
        let body = json!({ "fields": input.fields, "evidence": input.evidence });
        store
            .connection
            .batched(|transaction| {
                append_claim_record_tx(
                    transaction,
                    &store.origin,
                    &input.subject,
                    &input.kind,
                    input.actor.as_deref(),
                    &body,
                    &[],
                    None,
                )
                .map(|claim| (claim, true))
                .map_err(smallclaims::error::typed)
            })
            .map_err(|error| St3Error::new("internal", error))?
    }

    fn apply_repair_tx(
        &self,
        _transaction: &Transaction<'_>,
        _repaired: &str,
        _replacement: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn append_claim_tx(
        &self,
        transaction: &Transaction<'_>,
        origin: &str,
        subject: &str,
        kind: &str,
        actor: Option<&str>,
        body: &Value,
        predecessors: &[String],
        forced_batch: Option<&str>,
    ) -> anyhow::Result<ClaimRecord> {
        refuse_ineligible(kind, actor, body.get("fields").unwrap_or(&Value::Null))?;
        append_claim_record_tx(
            transaction,
            origin,
            subject,
            kind,
            actor,
            body,
            predecessors,
            forced_batch,
        )
    }

    fn project_incremental(
        &self,
        _transaction: &Transaction<'_>,
        _origin: &str,
        _through: u64,
    ) -> Result<IncrementalProjection, St3Error> {
        Ok(IncrementalProjection::Projected)
    }

    fn replay_from_nothing(&self, _transaction: &Transaction<'_>) -> Result<(), St3Error> {
        Ok(())
    }

    fn after_projection(&self, _transaction: &Transaction<'_>) -> Result<(), St3Error> {
        Ok(())
    }

    fn forget_views(&self) {}

    fn digest_tables(&self) -> &'static [(&'static str, &'static [&'static str])] {
        // The graph's own tables, as every store has them; the shard adds none.
        &[
            ("operations", &[]),
            ("blobs", &[]),
            ("documents", &["created_index"]),
        ]
    }

    fn legacy_digest_tables(&self) -> &'static [LegacyDigestTable] {
        &[]
    }

    fn checkpoint_rules_digest(&self) -> String {
        "messages-shard-v1".into()
    }

    fn plan_checkpoint_drops(&self, _sealed: &SealedSet) -> DropPlan {
        unreachable!("the messages shard is never checkpointed")
    }

    fn clear_checkpoint_projections(&self, _transaction: &Transaction<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    fn replay_checkpoint_projections(&self, _transaction: &Transaction<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    fn checkpoint_subject_answers(
        &self,
        _connection: &Connection,
        _subject: &str,
        _cut: u128,
    ) -> anyhow::Result<Value> {
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::collections::BTreeMap;

    const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

    fn input(kind: &str, actor: Option<&str>, fields: Value) -> ClaimInput {
        ClaimInput {
            subject: "message/0123456789abcdef".into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }
    }

    fn sent(from: &str) -> Value {
        json!({"from": from, "to": "agent/cedar", "content": "hello", "status": "sent"})
    }

    fn advertise(store: &Store, messages_stream: u8) {
        store
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.graph.origin),
                kind: "daemon.started".into(),
                actor: None,
                fields: serde_json::from_value(json!({
                    "status": "running",
                    "features": {"messages_stream": messages_stream}
                }))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("daemon-start:{}:{messages_stream}", store.graph.origin)),
            })
            .unwrap();
    }

    fn replicate(source: &Store, target: &Store) {
        let exchange = source
            .graph
            .export_replication_exchange(FLEET, &target.graph.replication_inventory().unwrap())
            .unwrap();
        target
            .graph
            .receive_replication_exchange(&source.graph.origin, FLEET, &exchange)
            .unwrap();
        target.graph.validate_replication_backlog().unwrap();
        target.graph.apply_replication_repairs().unwrap();
        target.graph.project_replication_backlog().unwrap();
    }

    #[test]
    fn the_shard_opens_once_beside_the_graph_and_takes_its_fleet() {
        let root = tempfile::tempdir().unwrap();
        let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        graph.bind_fleet(FLEET).unwrap();
        assert!(shard(&graph).is_none());
        let opened = open(&graph, root.path()).unwrap();
        assert!(root.path().join("shards/messages/claims.sqlite3").exists());
        assert_eq!(opened.bound_shard().unwrap().as_deref(), Some(SHARD_ID));
        assert_eq!(opened.fleet_membership().unwrap(), graph.graph.fleet_membership().unwrap());
        assert!(Arc::ptr_eq(&shard(&graph).unwrap(), &opened));
        assert!(open(&graph, root.path()).is_err(), "one shard per graph");
        // The shard holds the graph weakly: letting the graph go closes the authority.
        drop(graph);
        assert!(opened.fleet_membership().is_err());
    }

    #[test]
    fn the_shard_holds_only_messages_an_agent_or_the_daemon_sent() {
        let root = tempfile::tempdir().unwrap();
        let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        let shard = open(&graph, root.path()).unwrap();
        shard
            .append_claim(&input("message.sent", Some("agent/birch"), sent("agent/birch")))
            .unwrap();
        for (kind, actor, fields) in [
            ("message.read", Some("agent/cedar"), json!({"status": "read"})),
            ("message.sent", Some("person/ada"), sent("person/ada")),
            ("message.sent", Some("agent/birch"), sent("person/ada")),
            (
                "message.sent",
                Some("agent/birch"),
                json!({"from": "agent/birch", "to": "agent/cedar", "content": "hi",
                       "status": "sent", "sender_class": "external-assistant"}),
            ),
        ] {
            let error = shard.append_claim(&input(kind, actor, fields)).unwrap_err();
            assert_eq!(error.code, "shard-kind-refused", "{kind} {actor:?}");
        }
        // A peer's envelope is classified the same way: the refused claim is not admitted.
        let record = |kind: &str, actor: Option<&str>, fields: Value| ClaimRecord {
            id: "claim/test".into(),
            store_index: 1,
            batch_id: "batch/test".into(),
            subject: "message/0123456789abcdef".into(),
            kind: kind.into(),
            origin: "birch".into(),
            actor: actor.map(str::to_owned),
            operation_id: None,
            request_digest: None,
            body: json!({"fields": fields, "evidence": []}),
            predecessors: Vec::new(),
            accepted_at_unix_ms: 1,
        };
        assert!(matches!(
            classify_stream_item(&record("message.sent", Some("agent/birch"), sent("agent/birch"))),
            Ok(ReplicatedClaimAdmission::Valid)
        ));
        for refused in [
            record("message.closed", Some("agent/cedar"), json!({"status": "closed"})),
            record("message.sent", Some("person/ada"), sent("person/ada")),
            record("message.sent", None, sent("person/ada")),
            record("work.progress", None, json!({})),
        ] {
            let Err(error) = classify_stream_item(&refused) else {
                panic!("{} was admitted", refused.kind);
            };
            assert_eq!(error.code, "shard-kind-refused");
        }
    }

    #[test]
    fn a_host_counts_while_its_current_start_advertises_the_shard() {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        for store in [&alder, &birch] {
            store.bind_fleet(FLEET).unwrap();
        }
        let peers = ["birch".to_owned()];
        assert!(advertising_hosts(&alder, &peers).unwrap().is_empty());
        advertise(&birch, 1);
        replicate(&birch, &alder);
        assert_eq!(advertising_hosts(&alder, &peers).unwrap(), BTreeSet::from(["birch".to_owned()]));
        // A host that is neither a member nor configured is never dialed for the shard.
        assert!(advertising_hosts(&alder, &[]).unwrap().is_empty());
        // Rolling birch back to a build without the shard writes a start without it.
        advertise(&birch, 0);
        replicate(&birch, &alder);
        assert!(advertising_hosts(&alder, &peers).unwrap().is_empty());
        // This host never counts itself.
        advertise(&alder, 1);
        assert!(advertising_hosts(&alder, &["alder".to_owned()]).unwrap().is_empty());
    }

    /// A page lists the graph's messages to a recipient, then its stream items, and a stream
    /// item's lifecycle is this member's record: it moves only forward and writes no claim.
    #[test]
    fn readers_page_past_the_graph_into_the_stream_and_record_its_lifecycle_locally() {
        let root = tempfile::tempdir().unwrap();
        let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        let shard = open(&graph, root.path()).unwrap();
        let message = |subject: &str, to: &str| ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("agent/birch".into()),
            fields: serde_json::from_value(json!({
                "from": "agent/birch", "to": to, "content": subject, "status": "sent"
            }))
            .unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        };
        graph.append_claim(&message("message/00000000000000a1", "agent/cedar")).unwrap();
        for (subject, to) in [
            ("message/00000000000000b1", "agent/cedar"),
            ("message/00000000000000b2", "agent/cedar"),
            ("message/00000000000000b3", "agent/other"),
        ] {
            shard.append_claim(&message(subject, to)).unwrap();
        }
        let through = graph.index().unwrap();
        let mut after = None;
        let mut listed = Vec::new();
        loop {
            let (page, next) = graph
                .messages_page_inner(Some("agent/cedar"), false, after, through, 1, false)
                .unwrap();
            listed.extend(page.into_iter().map(|message| message.subject));
            match next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        assert_eq!(listed, ["message/00000000000000a1", "message/00000000000000b1", "message/00000000000000b2"]);
        let (page, next) = graph
            .messages_page_inner(Some("agent/cedar"), false, None, through, 10, false)
            .unwrap();
        assert_eq!(page.len(), 3);
        assert_eq!(next, None);

        // The unpaged and delivery readers stay on the graph until they learn stream items.
        let unpaged = graph.messages_through(Some("agent/cedar"), false, through).unwrap();
        assert_eq!(unpaged.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["message/00000000000000a1"]);
        let delivered = graph.messages_through_inner(Some("agent/cedar"), false, through, true).unwrap();
        assert_eq!(delivered.len(), 1);

        let subject = "message/00000000000000b1";
        let view = graph.message(subject).unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.to.as_str()), ("sent", "agent/cedar"));
        assert!(view.created_index > STREAM_INDEX_BASE);
        let status = |status: &str| BTreeMap::from([("status".to_owned(), json!(status))]);
        assert!(record_lifecycle(&graph, subject, "message.delivered", Some("agent/cedar"), &status("delivered")).unwrap());
        assert!(!record_lifecycle(&graph, subject, "message.delivered", Some("agent/cedar"), &status("delivered")).unwrap());
        assert!(!record_lifecycle(&graph, subject, "message.staged", Some("agent/cedar"), &status("staged")).unwrap());
        assert_eq!(graph.message(subject).unwrap().unwrap().status, "delivered");
        assert_eq!(
            record_lifecycle(&graph, subject, "message.read", None, &status("read")).unwrap_err().code,
            "missing-message-actor"
        );
        // Forward steps follow the graph's transitions: an agent reads before it closes.
        assert_eq!(
            record_lifecycle(&graph, subject, "message.closed", Some("agent/cedar"), &status("closed")).unwrap_err().code,
            "invalid-message-transition"
        );
        assert!(record_lifecycle(&graph, subject, "message.read", Some("agent/cedar"), &status("read")).unwrap());
        assert!(record_lifecycle(&graph, subject, "message.closed", Some("agent/cedar"), &status("closed")).unwrap());
        let (page, _) = graph
            .messages_page_inner(Some("agent/cedar"), false, None, through, 10, false)
            .unwrap();
        assert!(page.iter().all(|message| message.subject != subject), "a closed item is listed");
        assert!(graph.claims_for(subject, None).unwrap().is_empty(), "lifecycle reached the graph");
        assert_eq!(
            record_lifecycle(&graph, "message/0000000000000000", "message.read", Some("agent/cedar"), &status("read"))
                .unwrap_err()
                .code,
            "missing-message"
        );
        // A delivery channel whose fence is stale records nothing.
        let fresh = "message/00000000000000b2";
        let mut stale = crate::mailbox::Fence::new("agent/cedar", "incarnation-ended", "delivery");
        stale.epoch = 1;
        let refused = record_fenced_lifecycle(&graph, &stale, fresh, "message.delivered", Some("agent/cedar"), &status("delivered"))
            .unwrap_err();
        assert_eq!(refused.code, "stale-mailbox-session", "{refused:?}");
        assert_eq!(graph.message(fresh).unwrap().unwrap().status, "sent");
        let receipt = lifecycle_receipt("alder", subject, "message.read", Some("agent/cedar"), &status("read"));
        assert!(receipt.id.starts_with("offset-receipt/"));
        assert_eq!(receipt.id, lifecycle_receipt("alder", subject, "message.read", Some("agent/cedar"), &status("read")).id);
    }

}
