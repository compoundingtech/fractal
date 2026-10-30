//! The runtime of the messages shard: a separate store that holds one immutable `message.sent`
//! claim per message, replicated beside the graph under the shard ID `messages`.
//!
//! The shard takes its fleet from the graph (`bind_fleet_authority`) and has no checkpoint,
//! so this runtime keeps no tables of its own and never seals. It admits only the messages the
//! stream may carry: a `message.sent` the schema accepts, written by an agent or the daemon.
//! A person's device-signed message and an external assistant's message stay graph claims,
//! because judging their signatures and grants needs principal facts only the graph holds.
use serde_json::{Value, json};
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
    if actor.is_some_and(|actor| actor.starts_with("person/")) {
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

    fn create_schema(&self, _connection: &Connection) -> anyhow::Result<()> {
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
}
