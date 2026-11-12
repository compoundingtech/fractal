//! Physical authority domains. Opening one store never opens or advances another.
//!
//! Source ownership and signer authorization are control-plane decisions. Callers must
//! verify those facts before they name a shard here; file presence grants no authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::store::{Runtime, Store};

/// A stable wire name and a single safe path component.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ShardId(String);

impl ShardId {
    pub fn parse(value: &str) -> Result<Self> {
        if value.is_empty()
            || value.len() > 64
            || value.starts_with('-')
            || value.ends_with('-')
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            bail!("invalid shard ID `{value}`");
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn database_path(&self, state_dir: &Path) -> PathBuf {
        state_dir
            .join("shards")
            .join(&self.0)
            .join("claims.sqlite3")
    }
}

/// Each entry is a complete Store: its own file, writer, envelope inventory,
/// admission cursors, checkpoint proof and trim frontier.
/// An empty set is the default and creates no shard files.
pub struct ShardStores {
    stores: BTreeMap<ShardId, Arc<Store>>,
}

impl ShardStores {
    /// `runtime` transfers a new runtime instance for each named database. A runtime
    /// holds projection caches, so it must not be shared with the default store.
    pub fn open(
        state_dir: &Path,
        origin: &str,
        mut runtime: impl FnMut(&ShardId) -> Box<dyn Runtime>,
        ids: impl IntoIterator<Item = ShardId>,
    ) -> Result<Self> {
        let ids = ids.into_iter().collect::<Vec<_>>();
        let mut unique = BTreeSet::new();
        for id in &ids {
            if !unique.insert(id) {
                bail!("duplicate shard ID `{}`", id.as_str());
            }
        }
        let mut stores = BTreeMap::new();
        for id in ids {
            let path = id.database_path(state_dir);
            std::fs::create_dir_all(path.parent().expect("shard database has a parent"))?;
            let store = Arc::new(Store::open(&path, origin, Arc::from(runtime(&id)))?);
            store.bind_shard(&id)?;
            stores.insert(id, store);
        }
        Ok(Self { stores })
    }

    pub fn get(&self, id: &ShardId) -> Option<&Arc<Store>> {
        self.stores.get(id)
    }
    pub fn ids(&self) -> impl Iterator<Item = &ShardId> {
        self.stores.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimInput;
    use crate::store::runtime::Plain;
    use std::collections::BTreeMap;

    #[test]
    fn names_are_safe_path_components_and_default_opens_nothing() {
        for invalid in ["", "../other", "a/b", ".", "UPPER", "-a", "a-", "a_b"] {
            assert!(ShardId::parse(invalid).is_err(), "{invalid}");
        }
        let root = tempfile::tempdir().unwrap();
        let stores = ShardStores::open(root.path(), "node", |_| Box::new(Plain), []).unwrap();
        assert_eq!(stores.ids().count(), 0);
        assert!(!root.path().join("shards").exists());
        let github = ShardId::parse("github").unwrap();
        assert!(
            ShardStores::open(
                root.path(),
                "node",
                |_| Box::new(Plain),
                [github.clone(), github]
            )
            .is_err()
        );
        assert!(!root.path().join("shards").exists());
    }

    #[test]
    fn named_stores_have_independent_files_and_writer_locks() {
        let root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let work = ShardId::parse("work").unwrap();
        let stores = ShardStores::open(
            root.path(),
            "node",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        assert!(github.database_path(root.path()).exists());
        assert!(work.database_path(root.path()).exists());
        let first = stores.get(&github).unwrap();
        let second = stores.get(&work).unwrap();
        let _first_writer = first.connection.write();
        // A second writer is acquired while the first is still held. A shared lock
        // would deadlock this test, so the distinct locks are the required behavior.
        let _second_writer = second.connection.write();
    }

    #[test]
    fn envelopes_and_reopen_are_scoped_to_the_named_store() {
        let root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let work = ShardId::parse("work").unwrap();
        let first = ShardStores::open(
            root.path(),
            "node",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        let github_store = first.get(&github).unwrap();
        let work_store = first.get(&work).unwrap();
        for store in [github_store, work_store] {
            store
                .bind_fleet("5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12")
                .unwrap();
        }
        github_store
            .append_claim(&ClaimInput {
                subject: "note/github".into(),
                kind: "example.note".into(),
                actor: Some("person/ada".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert_eq!(
            github_store
                .replication_inventory()
                .unwrap()
                .envelopes
                .len(),
            1
        );
        assert!(
            work_store
                .replication_inventory()
                .unwrap()
                .envelopes
                .is_empty()
        );
        drop(first);
        let reopened = ShardStores::open(
            root.path(),
            "node",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        assert_eq!(
            reopened
                .get(&github)
                .unwrap()
                .bound_shard()
                .unwrap()
                .as_deref(),
            Some("github")
        );
        assert_eq!(
            reopened
                .get(&work)
                .unwrap()
                .bound_shard()
                .unwrap()
                .as_deref(),
            Some("work")
        );
        assert_eq!(
            reopened
                .get(&github)
                .unwrap()
                .replication_inventory()
                .unwrap()
                .envelopes
                .len(),
            1
        );
        assert!(
            reopened
                .get(&work)
                .unwrap()
                .replication_inventory()
                .unwrap()
                .envelopes
                .is_empty()
        );
    }

    #[test]
    fn replicating_one_shard_never_advances_another_shard() {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let work = ShardId::parse("work").unwrap();
        let first = ShardStores::open(
            first_root.path(),
            "first",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        let second = ShardStores::open(
            second_root.path(),
            "second",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        for stores in [&first, &second] {
            for id in [&github, &work] {
                stores.get(id).unwrap().bind_fleet(FLEET).unwrap();
            }
        }
        let source = first.get(&github).unwrap();
        let target = second.get(&github).unwrap();
        source
            .append_claim(&ClaimInput {
                subject: "note/github".into(),
                kind: "example.note".into(),
                actor: Some("person/ada".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let exchange = source
            .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
            .unwrap();
        target
            .receive_replication_exchange("first", FLEET, &exchange)
            .unwrap();
        target.validate_replication_backlog().unwrap();
        target.apply_replication_repairs().unwrap();
        assert!(target.project_replication_backlog().unwrap());
        assert_eq!(target.claims_for("note/github", None).unwrap().len(), 1);
        assert!(
            second
                .get(&work)
                .unwrap()
                .replication_inventory()
                .unwrap()
                .envelopes
                .is_empty()
        );
    }

    #[test]
    fn checkpoint_tombstones_are_scoped_to_one_shard() {
        use crate::store::EnvelopeTombstone;

        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let work = ShardId::parse("work").unwrap();
        let stores = ShardStores::open(
            root.path(),
            "node",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        let github_store = stores.get(&github).unwrap();
        let work_store = stores.get(&work).unwrap();
        for store in [github_store, work_store] {
            store.bind_fleet(FLEET).unwrap();
        }
        github_store
            .apply_checkpoint_drop(
                "checkpoint/2026-09-27",
                &[EnvelopeTombstone {
                    writer: "source".into(),
                    sequence: 1,
                    envelope_hash: format!("{:064x}", 1),
                    accepted_at_unix_ms: 1,
                }],
                &[],
            )
            .unwrap();
        assert_eq!(github_store.checkpointed_envelopes().unwrap(), 1);
        assert_eq!(work_store.checkpointed_envelopes().unwrap(), 0);
        assert!(
            work_store
                .replication_inventory()
                .unwrap()
                .envelopes
                .is_empty()
        );
    }

    #[test]
    fn an_exchange_for_one_shard_cannot_advance_another_or_the_default() {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let work = ShardId::parse("work").unwrap();
        let stores = ShardStores::open(
            root.path(),
            "source",
            |_| Box::new(Plain),
            [github.clone(), work.clone()],
        )
        .unwrap();
        let source = stores.get(&github).unwrap();
        let wrong_shard = stores.get(&work).unwrap();
        let default = Store::open(
            &root.path().join("default.sqlite3"),
            "default",
            Arc::new(Plain),
        )
        .unwrap();
        for store in [source.as_ref(), wrong_shard.as_ref(), &default] {
            store.bind_fleet(FLEET).unwrap();
        }
        source
            .append_claim(&ClaimInput {
                subject: "note/github".into(),
                kind: "example.note".into(),
                actor: Some("person/ada".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let exchange = source
            .export_replication_exchange(FLEET, &wrong_shard.replication_inventory().unwrap())
            .unwrap();
        assert_eq!(exchange.shard_id.as_deref(), Some("github"));
        for wrong in [wrong_shard.as_ref(), &default] {
            let before_index = wrong.index().unwrap();
            let before_inventory = wrong.replication_inventory().unwrap().digest;
            let error = wrong
                .receive_replication_exchange("source", FLEET, &exchange)
                .unwrap_err();
            assert_eq!(error.code, "shard-mismatch");
            assert_eq!(wrong.index().unwrap(), before_index);
            assert_eq!(
                wrong.replication_inventory().unwrap().digest,
                before_inventory
            );
            assert!(wrong.replication_inventory().unwrap().envelopes.is_empty());
        }
        // An older/default exchange omits the new field on the wire. It cannot enter a shard.
        let legacy = default
            .export_replication_exchange(FLEET, &source.replication_inventory().unwrap())
            .unwrap();
        let wire = serde_json::to_value(&legacy).unwrap();
        assert!(wire.get("shard_id").is_none());
        let legacy = serde_json::from_value(wire).unwrap();
        let before_index = source.index().unwrap();
        let error = source
            .receive_replication_exchange("default", FLEET, &legacy)
            .unwrap_err();
        assert_eq!(error.code, "shard-mismatch");
        assert_eq!(source.index().unwrap(), before_index);
    }

    /// A default store founded by `a`, whose anchor key `a` holds, with `a` admitted.
    fn founded_graph(path: &Path, origin: &str, anchor: &Arc<crate::fleet::MemberKey>) -> Arc<Store> {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let graph = Arc::new(Store::open(path, origin, Arc::new(Plain)).unwrap());
        graph.bind_fleet(FLEET).unwrap();
        graph.pin_fleet_anchor(anchor.public()).unwrap();
        graph
    }

    fn note(subject: &str) -> ClaimInput {
        ClaimInput {
            subject: subject.into(),
            kind: "example.note".into(),
            actor: Some("person/ada".into()),
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }
    }

    fn messages_shard(root: &Path, origin: &str, graph: &Arc<Store>) -> Arc<Store> {
        let id = ShardId::parse("messages").unwrap();
        let stores = ShardStores::open(root, origin, |_| Box::new(Plain), [id.clone()]).unwrap();
        let shard = stores.get(&id).unwrap().clone();
        shard.bind_fleet_authority(graph.clone()).unwrap();
        shard
    }

    /// A `fleet.*` claim the anchor `a` wrote, or, for a leave or its shard bounds, the leaver.
    fn fleet_claim(
        id: &str,
        kind: &str,
        name: &str,
        sequence: u64,
        fields: serde_json::Value,
    ) -> crate::fleet::FleetClaim {
        use crate::fleet::membership::{MEMBER_LEFT, SHARD_BOUNDS};
        let (writer, signer) = match fields["member_key"].as_str() {
            Some(key) if [MEMBER_LEFT, SHARD_BOUNDS].contains(&kind) => (name, key.to_owned()),
            _ => ("a", "ka".to_owned()),
        };
        crate::fleet::FleetClaim {
            id: id.into(),
            kind: kind.into(),
            subject: format!("host/{name}"),
            fields: serde_json::from_value(fields).unwrap(),
            writer: writer.into(),
            sequence,
            signers: BTreeSet::from([signer]),
        }
    }

    fn member_admitted(
        id: &str,
        name: &str,
        key: &str,
        sequence: u64,
        writer_floor: u64,
    ) -> crate::fleet::FleetClaim {
        let fields = serde_json::json!({
            "fleet_id": "fleet", "member_key": key,
            "via": if name == "a" { "anchor" } else { "invite" }, "mode": "listening",
            "writer_floor": writer_floor,
        });
        fleet_claim(id, crate::fleet::membership::MEMBER_ADMITTED, name, sequence, fields)
    }

    /// `b`'s leave at graph sequence 5 under `key`, after its bounds at 4 when it wrote the shard.
    fn leave(key: &str, shard_high_water: Option<u64>) -> Vec<crate::fleet::FleetClaim> {
        use crate::fleet::membership::{MEMBER_LEFT, SHARD_BOUNDS};
        let mut claims = Vec::new();
        if let Some(high_water) = shard_high_water {
            let fields = serde_json::json!({
                "member_key": key, "shard_high_water": {"messages": high_water},
            });
            claims.push(fleet_claim(&format!("3-{key}-bounds"), SHARD_BOUNDS, "b", 4, fields));
        }
        let fields = serde_json::json!({"member_key": key, "high_water": 5});
        claims.push(fleet_claim(&format!("4-{key}-left"), MEMBER_LEFT, "b", 5, fields));
        claims
    }

    /// How a messages shard judges `b`'s envelopes, one per (sequence, signing key). Each key's
    /// envelope at a sequence is its own, with its own hash.
    fn messages_holds(
        membership: &crate::fleet::Membership,
        envelopes: &[(u64, &str)],
    ) -> Vec<Option<&'static str>> {
        use crate::claim::ReplicaEnvelope;
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE replica_envelope_signatures(
                    writer TEXT, sequence INTEGER, envelope_hash TEXT, member_key TEXT)",
            )
            .unwrap();
        envelopes
            .iter()
            .map(|(sequence, signer)| {
                let envelope = ReplicaEnvelope {
                    writer: "b".into(),
                    sequence: *sequence,
                    previous_hash: None,
                    hash: format!("{sequence:032x}{:0>32}", hex::encode(signer)),
                    accepted_at_unix_ms: 0,
                    payload: Vec::new().into(),
                    member_key: None,
                    signature: None,
                };
                connection
                    .execute(
                        "INSERT INTO replica_envelope_signatures VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![envelope.writer, sequence, envelope.hash, signer],
                    )
                    .unwrap();
                crate::store::shard_admission_hold(&connection, membership, "messages", &envelope)
                    .unwrap()
            })
            .collect()
    }

    /// After a writer's key is superseded without a leave, a shard admits its envelopes only
    /// with a signature by the new key: the old key's incarnation never ended, but a signature
    /// by it stays unsigned at every sequence.
    #[test]
    fn a_shard_refuses_a_rotated_out_key() {
        let membership = crate::fleet::Membership::fold(
            Some("ka"),
            &[
                member_admitted("1-a", "a", "ka", 1, 0),
                member_admitted("2-b", "b", "kb-old", 2, 0),
                member_admitted("3-b", "b", "kb-new", 3, 30),
            ],
        );
        assert_eq!(
            messages_holds(&membership, &[(1, "kb-old"), (1, "kb-new"), (2, "kb-new")]),
            [Some("unsigned"), None, None]
        );
    }

    /// A leaver's shard envelopes at or below the high water of its own bounds are admitted,
    /// and one above it is fenced. After it rejoins with a new key, the new key's envelope at
    /// the same sequence is admitted too: it is a distinct envelope.
    #[test]
    fn a_shard_admits_a_leavers_envelopes_at_or_below_its_high_water_and_fences_the_rest() {
        use crate::fleet::Membership;
        let mut claims = vec![
            member_admitted("1-a", "a", "ka", 1, 0),
            member_admitted("2-b", "b", "kb", 2, 0),
        ];
        claims.extend(leave("kb", Some(6)));
        let left = Membership::fold(Some("ka"), &claims);
        assert!(left.counts("3-kb-bounds"));
        assert_eq!(
            messages_holds(&left, &[(1, "kb"), (6, "kb"), (7, "kb")]),
            [None, None, Some("fenced")]
        );
        claims.push(member_admitted("5-b-again", "b", "kb2", 3, 5));
        let rejoined = Membership::fold(Some("ka"), &claims);
        assert_eq!(
            messages_holds(&rejoined, &[(6, "kb"), (6, "kb2"), (7, "kb"), (7, "kb2")]),
            [None, None, Some("unsigned"), None]
        );
    }

    /// A leave without bounds, as an older build writes it or one that never wrote the shard,
    /// fences the leaver's key at every shard sequence.
    #[test]
    fn a_shard_fences_a_leaver_without_bounds_at_every_sequence() {
        let mut claims = vec![
            member_admitted("1-a", "a", "ka", 1, 0),
            member_admitted("2-b", "b", "kb", 2, 0),
        ];
        claims.extend(leave("kb", None));
        let membership = crate::fleet::Membership::fold(Some("ka"), &claims);
        assert_eq!(messages_holds(&membership, &[(1, "kb")]), [Some("fenced")]);
    }

    /// A shard holds no fleet claims. It signs its envelopes for its authority's fleet in a
    /// domain that names the shard, and admits a keyed member's envelopes only with a signature
    /// in that domain by a key the authority knows: one made for the default store does not count.
    #[test]
    fn a_shard_takes_its_fleet_from_the_default_store_and_signs_in_its_own_domain() {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let a_root = tempfile::tempdir().unwrap();
        let b_root = tempfile::tempdir().unwrap();
        let a_key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        let a_graph = founded_graph(&a_root.path().join("claims.sqlite3"), "a", &a_key);
        a_graph.set_member_key(Some(a_key.clone())).unwrap();
        a_graph
            .append_claim(&ClaimInput {
                subject: "host/a".into(),
                kind: "fleet.member-admitted".into(),
                actor: None,
                fields: serde_json::from_value(serde_json::json!({
                    "fleet_id": FLEET, "member_key": a_key.public(), "via": "anchor",
                    "mode": "listening"
                }))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        // b learns a's membership through the default store, as a member does.
        let b_graph = founded_graph(&b_root.path().join("claims.sqlite3"), "b", &a_key);
        let graph_exchange = a_graph
            .export_replication_exchange(FLEET, &b_graph.replication_inventory().unwrap())
            .unwrap();
        b_graph
            .receive_replication_exchange("a", FLEET, &graph_exchange)
            .unwrap();
        b_graph.validate_replication_backlog().unwrap();
        b_graph.apply_replication_repairs().unwrap();
        b_graph.project_replication_backlog().unwrap();
        assert!(matches!(
            b_graph.fleet_membership().unwrap().state("a"),
            crate::fleet::MemberState::Current(_)
        ));

        let a_shard = messages_shard(a_root.path(), "a", &a_graph);
        assert!(a_shard.bind_fleet_authority(a_graph.clone()).is_err(), "set once");
        assert!(a_graph.bind_fleet_authority(a_shard.clone()).is_err(), "default stores take none");
        a_shard.set_member_key(Some(a_key.clone())).unwrap();
        a_shard.append_claim(&note("note/one")).unwrap();
        let b_shard = messages_shard(b_root.path(), "b", &b_graph);
        let exchange = a_shard
            .export_replication_exchange(FLEET, &b_shard.replication_inventory().unwrap())
            .unwrap();
        assert_eq!(exchange.shard_id.as_deref(), Some("messages"));
        let envelope = &exchange.envelopes[0];
        let signature = envelope.signature.as_deref().expect("the shard signs for the fleet");
        assert!(crate::fleet::verify_signature(
            a_key.public(),
            &crate::fleet::envelope_signature_message_in(
                Some("messages"), FLEET, "a", envelope.sequence, &envelope.hash,
            ),
            signature,
        ));
        assert!(!crate::fleet::verify_signature(
            a_key.public(),
            &crate::fleet::envelope_signature_message(FLEET, "a", envelope.sequence, &envelope.hash),
            signature,
        ));

        // The same envelope signed in the default store's domain is unsigned to the shard, and
        // an unsigned envelope from a keyed member is held.
        let mut graph_signed = exchange.clone();
        for envelope in &mut graph_signed.envelopes {
            envelope.signature = Some(a_key.sign(&crate::fleet::envelope_signature_message(
                FLEET,
                "a",
                envelope.sequence,
                &envelope.hash,
            )));
        }
        b_shard
            .receive_replication_exchange("a", FLEET, &graph_signed)
            .unwrap();
        b_shard.validate_replication_backlog().unwrap();
        b_shard.project_replication_backlog().unwrap();
        let held: i64 = b_shard
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM replica_envelope_holds WHERE reason='unsigned'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(usize::try_from(held).unwrap(), exchange.envelopes.len());
        assert!(b_shard.claims_for("note/one", None).unwrap().is_empty());

        // Signed in the shard's domain, it is admitted on b's membership.
        b_shard
            .receive_replication_exchange("a", FLEET, &exchange)
            .unwrap();
        b_shard.validate_replication_backlog().unwrap();
        b_shard.apply_replication_repairs().unwrap();
        b_shard.project_replication_backlog().unwrap();
        assert_eq!(b_shard.claims_for("note/one", None).unwrap().len(), 1);
        assert!(b_shard.fleet_membership().unwrap().state("a") == b_graph.fleet_membership().unwrap().state("a"));

    }

    /// A fleet of configured peers has no membership. Its shards admit each other's envelopes
    /// as its default stores do, or the shard would never replicate there.
    #[test]
    fn shards_of_a_fleet_without_membership_admit_their_peers() {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        let a_root = tempfile::tempdir().unwrap();
        let b_root = tempfile::tempdir().unwrap();
        let graph = |root: &Path, origin: &str| {
            let graph = Arc::new(Store::open(&root.join("claims.sqlite3"), origin, Arc::new(Plain)).unwrap());
            graph.bind_fleet(FLEET).unwrap();
            graph
        };
        let a_graph = graph(a_root.path(), "a");
        let b_graph = graph(b_root.path(), "b");
        let a_shard = messages_shard(a_root.path(), "a", &a_graph);
        let b_shard = messages_shard(b_root.path(), "b", &b_graph);
        a_shard.append_claim(&note("note/legacy")).unwrap();
        let exchange = a_shard
            .export_replication_exchange(FLEET, &b_shard.replication_inventory().unwrap())
            .unwrap();
        b_shard.receive_replication_exchange("a", FLEET, &exchange).unwrap();
        b_shard.validate_replication_backlog().unwrap();
        b_shard.apply_replication_repairs().unwrap();
        b_shard.project_replication_backlog().unwrap();
        assert_eq!(b_shard.claims_for("note/legacy", None).unwrap().len(), 1);
        assert!(b_graph.claims_for("note/legacy", None).unwrap().is_empty());
    }

    #[test]
    fn a_used_default_file_cannot_be_relabelled_as_a_shard() {
        use crate::store::EnvelopeTombstone;

        let root = tempfile::tempdir().unwrap();
        let github = ShardId::parse("github").unwrap();
        let fleet_bound = Store::open(
            &root.path().join("fleet-bound.sqlite3"),
            "node",
            Arc::new(Plain),
        )
        .unwrap();
        fleet_bound.bind_fleet("fleet").unwrap();
        assert!(fleet_bound.bind_shard(&github).is_err());

        let trimmed = Store::open(
            &root.path().join("trimmed.sqlite3"),
            "node",
            Arc::new(Plain),
        )
        .unwrap();
        trimmed
            .apply_checkpoint_drop(
                "checkpoint/2026-09-27",
                &[EnvelopeTombstone {
                    writer: "source".into(),
                    sequence: 1,
                    envelope_hash: format!("{:064x}", 1),
                    accepted_at_unix_ms: 1,
                }],
                &[],
            )
            .unwrap();
        assert_eq!(trimmed.index().unwrap(), 0);
        assert!(trimmed.bind_shard(&github).is_err());

        let named = Store::open(
            &root.path().join("named.sqlite3"),
            "node",
            Arc::new(Plain),
        )
        .unwrap();
        named.bind_shard(&github).unwrap();
        drop(named);
        let moved = Store::open(
            &root.path().join("named.sqlite3"),
            "node",
            Arc::new(Plain),
        )
        .unwrap();
        assert!(moved.bind_shard(&ShardId::parse("work").unwrap()).is_err());
    }
}
