//! Exact membership authority and the SQL budget of its signer lookup.
use super::*;
use crate::fleet::{FleetClaim, Membership};
use std::cell::Cell;

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
    pub(super) static SIGNATURE_VERIFICATIONS: Cell<usize> = const { Cell::new(0) };
}

fn fixture() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    // Invented admitted facts; signature verification and admission have separate controls.
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9cd53cdb5da9 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO meta VALUES('fleet_anchor_key','anchor-key');
         CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claims(id TEXT PRIMARY KEY, kind TEXT, subject TEXT, body TEXT, batch_id TEXT);
         CREATE TABLE replica_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
             batch_id TEXT, PRIMARY KEY(writer,sequence,envelope_hash));
         CREATE INDEX replica_envelopes_batch ON replica_envelopes(batch_id);
         CREATE TABLE replica_envelope_signatures(writer TEXT, sequence INTEGER,
             envelope_hash TEXT, member_key TEXT,
             PRIMARY KEY(writer,sequence,envelope_hash,member_key));"
    ).unwrap();
    connection
}

fn admitted(id: &str, name: &str, key: &str, via: &str, writer: &str, sequence: u64) -> FleetClaim {
    FleetClaim {
        id: id.into(),
        kind: "fleet.member-admitted".into(),
        subject: format!("host/{name}"),
        fields: BTreeMap::from([
            ("fleet_id".into(), serde_json::json!("fleet/example")),
            ("member_key".into(), serde_json::json!(key)),
            ("via".into(), serde_json::json!(via)),
            ("mode".into(), serde_json::json!("listening")),
        ]),
        writer: writer.into(),
        sequence,
        signers: BTreeSet::new(),
    }
}

fn insert_claim(connection: &Connection, claim: &FleetClaim) {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hdf7bb43c650f owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES(?1,?2,?3)",
            params![claim.id, claim.writer, claim.sequence],
        )
        .unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h771f68bc2561 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES(?1,?2,?3,?4,?1)",
            params![
                claim.id,
                claim.kind,
                claim.subject,
                serde_json::json!({"fields": claim.fields}).to_string()
            ],
        )
        .unwrap();
}

fn envelope(connection: &Connection, claim: &FleetClaim, hash: &str, keys: &[&str]) {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h78c914935ecd owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes VALUES(?1,?2,?3,?4)",
            params![claim.writer, claim.sequence, hash, claim.id],
        )
        .unwrap();
    for key in keys {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-heecc52686f3d owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES(?1,?2,?3,?4)",
                params![claim.writer, claim.sequence, hash, key],
            )
            .unwrap();
    }
}

#[test]
fn membership_join_preserves_forks_exact_signature_identity_and_local_fallback() {
    let connection = fixture();
    let mut anchor = admitted("0-anchor", "birch", "anchor-key", "anchor", "birch", 3);
    let mut unsigned = admitted("1-unsigned", "cedar", "cedar-key", "invite", "birch", 4);
    let mut good = admitted("2-good", "oak", "oak-key", "invite", "birch", 5);
    let mut local = admitted("3-local", "alder", "alder-key", "invite", "birch", 6);
    let remote = admitted("4-remote", "elm", "elm-key", "invite", "oak", 2);
    for claim in [&anchor, &unsigned, &good, &local, &remote] {
        insert_claim(&connection, claim);
    }
    envelope(&connection, &anchor, "fork-one", &["other-key"]);
    envelope(
        &connection,
        &anchor,
        "fork-two",
        &["anchor-key", "other-key"],
    );
    envelope(&connection, &anchor, "fork-empty", &[]);
    envelope(&connection, &unsigned, "unsigned", &["wrong-key"]);
    // A signature with the right key but a different hash, writer, or sequence proves nothing.
    for (writer, sequence, hash) in [
        ("birch", 4, "other-hash"),
        ("oak", 4, "unsigned"),
        ("birch", 99, "unsigned"),
    ] {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h340bdec4b8b2 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES(?1,?2,?3,'anchor-key')",
                params![writer, sequence, hash],
            )
            .unwrap();
    }
    envelope(&connection, &good, "good", &["anchor-key"]);
    anchor.signers = BTreeSet::from(["anchor-key".into(), "other-key".into()]);
    unsigned.signers = BTreeSet::from(["wrong-key".into()]);
    good.signers = BTreeSet::from(["anchor-key".into()]);
    let mut expected = vec![anchor, unsigned, good, local.clone(), remote];
    let sealed = fleet_membership_tx(&connection).unwrap();
    assert_eq!(sealed, Membership::fold(Some("anchor-key"), &expected));
    assert!(sealed.counts("0-anchor") && sealed.counts("2-good"));
    assert!(!sealed.counts("1-unsigned") && !sealed.counts("3-local"));
    local.signers.insert("anchor-key".into());
    expected[3] = local;
    let client =
        fleet_membership_tx_with_local_signer(&connection, Some(("birch", "anchor-key"))).unwrap();
    assert_eq!(client, Membership::fold(Some("anchor-key"), &expected));
    assert!(client.counts("3-local"));
    assert!(!client.counts("4-remote"));
}

#[test]
fn membership_without_anchor_does_not_inspect_or_admit_claims() {
    let connection = fixture();
    connection.execute(/* sqlite-large-value: allow-for-now lv-haba18bfca731 owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta", []).unwrap();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h9be0a7a129f5 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='malformed'", [])
        .unwrap();
    envelope(&connection, &anchor, "anchor", &["anchor-key"]);
    assert_eq!(
        fleet_membership_tx(&connection).unwrap(),
        Membership::default()
    );
}

#[test]
fn membership_signer_lookup_uses_two_statements_independent_of_claim_count() {
    for count in [1, 128] {
        let mut connection = fixture();
        let anchor = admitted("0-anchor", "birch", "anchor-key", "anchor", "birch", 3);
        insert_claim(&connection, &anchor);
        envelope(&connection, &anchor, "anchor", &["anchor-key"]);
        for index in 1..count {
            let claim = admitted(
                &format!("peer-{index}"),
                &format!("cedar-{index}"),
                &format!("key-{index}"),
                "invite",
                "birch",
                3 + index,
            );
            insert_claim(&connection, &claim);
            envelope(
                &connection,
                &claim,
                &format!("envelope-{index}"),
                &["anchor-key"],
            );
        }
        // This connection-local trace does not include other tests or setup statements.
        STATEMENTS.with(|count| count.set(0));
        connection.trace(Some(|_| {
            STATEMENTS.with(|count| count.set(count.get() + 1))
        }));
        let membership = fleet_membership_tx(&connection).unwrap();
        connection.trace(None);
        assert!(membership.counts("0-anchor"));
        STATEMENTS.with(|statements| {
            assert_eq!(
                statements.get(),
                2,
                "membership fold exceeded its two-statement SQL budget at {count} claims"
            )
        });
    }
}

#[test]
fn membership_signers_do_not_multiply_claim_rows_or_scan_unrelated_signatures() {
    use rusqlite::StatementStatus;
    let connection = fixture();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    envelope(&connection, &anchor, "anchor", &["anchor-key"]);
    for index in 0..128 {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h9c78d72f8d0e owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES('birch',3,'anchor',?1)",
                [format!("other-key-{index}")],
            )
            .unwrap();
    }
    let measure = || {
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h99012a678132 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ &fleet_claims_with_signers_query())
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        let mut count = 0;
        while let Some(row) = rows.next().unwrap() {
            count += 1;
            let signers: BTreeSet<String> =
                serde_json::from_str(&row.get::<_, String>(7).unwrap()).unwrap();
            assert_eq!(signers.len(), 129);
        }
        drop(rows);
        assert_eq!(count, 1, "signature count must not multiply claim bodies");
        statement.get_status(StatementStatus::VmStep)
    };
    let before = measure();
    for index in 0..4096 {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h40aaea1d01ec owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES('elm',?1,'unrelated','other-key')",
                [index],
            )
            .unwrap();
    }
    let after = measure();
    assert!(
        before < 2_000,
        "signer aggregation exceeded its bounded fixture VM budget: {before}"
    );
    assert!(
        after <= before + 32,
        "unrelated signatures increased fold VM work: {before} -> {after}"
    );
}

#[test]
fn sealed_fleet_cache_reuses_the_query_and_invalidates_every_fold_input() {
    let mut connection = fixture();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hebccae97eca4 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ &fleet_generation_schema()).unwrap();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    envelope(&connection, &anchor, "anchor", &["anchor-key"]);
    let mut cache = None;
    // Count the actual history query, not cache publications or elapsed time.
    connection.trace(Some(|sql| {
        if sql.starts_with("SELECT claims.id, claims.kind, claims.subject, claims.body,") {
            STATEMENTS.with(|count| count.set(count.get() + 1));
        }
    }));
    STATEMENTS.with(|count| count.set(0));
    let first = sealed_fleet_view_tx(&connection, &mut cache).unwrap();
    assert_eq!(first.current("birch").len(), 1);
    for _ in 0..8 {
        assert_eq!(sealed_fleet_view_tx(&connection, &mut cache).unwrap(), first);
    }
    STATEMENTS.with(|count| assert_eq!(count.get(), 1));

    // No time-dependent fields exist: "current" is an open writer window,
    // not a wall-clock lease. Only a claim closes that window.
    let mutations = [
        /* sqlite-large-value: allow-for-now lv-hfe7bc53ccf35 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.mode','dial-out')",
        /* sqlite-large-value: allow-for-now lv-h29c99d643170 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/cedar'",
        /* sqlite-large-value: allow-for-now lv-h603ade762e9a owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET id='renamed'",
        /* sqlite-large-value: allow-for-now lv-h55e08ce4e425 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=4",
        /* sqlite-large-value: allow-for-now lv-h2d780a78ea0a owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET origin='cedar'",
        /* sqlite-large-value: allow-for-now lv-h09426db0fbaa owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET origin='birch', replica_sequence=3",
        /* sqlite-large-value: allow-for-now lv-h7bb07031ba8c owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET batch_id=NULL",
        /* sqlite-large-value: allow-for-now lv-he92456f7fb9b owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET batch_id='anchor'",
        /* sqlite-large-value: allow-for-now lv-h2c3a8c113139 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET envelope_hash='changed'",
        /* sqlite-large-value: allow-for-now lv-h9d8387c139c5 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET envelope_hash='changed'",
        /* sqlite-large-value: allow-for-now lv-h23116f518f45 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key='other-key'",
        /* sqlite-large-value: allow-for-now lv-hba1cdada2276 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures",
        /* sqlite-large-value: allow-for-now lv-hfe05e3188e97 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES('birch',3,'changed','anchor-key')",
        /* sqlite-large-value: allow-for-now lv-hc58680be3c16 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes",
        /* sqlite-large-value: allow-for-now lv-hf31184c6716d owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes VALUES('birch',3,'changed','anchor')",
        /* sqlite-large-value: allow-for-now lv-hc6ed37a4e642 owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='other-anchor' WHERE key='fleet_anchor_key'",
        /* sqlite-large-value: allow-for-now lv-h294c8934b41c owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='fleet_anchor_key'",
        /* sqlite-large-value: allow-for-now lv-h361bb9e86be3 owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta VALUES('fleet_anchor_key','anchor-key')",
        /* sqlite-large-value: allow-for-now lv-he2d6d2705ec5 owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM batches",
        /* sqlite-large-value: allow-for-now lv-h8d6172d79300 owner=smallclaims::store::membership_query_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES('anchor','birch',3)",
        /* sqlite-large-value: allow-for-now lv-h130b3e63e6de owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims",
        /* sqlite-large-value: allow-for-now lv-h1a9cad174821 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES('new','fleet.member-left','host/birch','{\"fields\":{}}','anchor')",
    ];
    for sql in mutations {
        let before = fleet_generation(&connection).unwrap();
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h2cdc92932736 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        let after = fleet_generation(&connection).unwrap();
        assert!(after > before, "generation did not invalidate: {sql}");
        STATEMENTS.with(|count| count.set(0));
        let actual = sealed_fleet_view_tx(&connection, &mut cache).unwrap();
        let expected_queries = usize::from(actual.anchor.is_some());
        STATEMENTS.with(|count| assert_eq!(count.get(), expected_queries, "{sql}"));
        assert_eq!(
            actual,
            crate::fleet::FleetView::from_membership(&fleet_membership_tx(&connection).unwrap()),
            "{sql}"
        );
        STATEMENTS.with(|count| count.set(0));
        assert_eq!(sealed_fleet_view_tx(&connection, &mut cache).unwrap(), actual);
        STATEMENTS.with(|count| assert_eq!(count.get(), 0, "{sql}"));
    }
    connection.trace(None);
}

#[test]
fn sealed_fleet_cache_never_includes_the_local_signer_fallback() {
    let connection = fixture();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hb247a89ed0c1 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ &fleet_generation_schema()).unwrap();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    let local = fleet_membership_tx_with_local_signer(
        &connection, Some(("birch", "anchor-key")),
    ).unwrap();
    assert!(local.counts("anchor"));
    let mut cache = None;
    assert!(sealed_fleet_view_tx(&connection, &mut cache).unwrap().members.is_empty());
    envelope(&connection, &anchor, "sealed", &["anchor-key"]);
    let sealed = sealed_fleet_view_tx(&connection, &mut cache).unwrap();
    assert_eq!(sealed.current("birch")[0].state, "current");
    assert_eq!(sealed_fleet_view_tx(&connection, &mut cache).unwrap(), sealed);
}

#[test]
fn sealed_fleet_cache_tracks_batch_signers_after_envelope_identity_changes() {
    let connection = fixture();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h0854bfdff2a6 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ &fleet_generation_schema()).unwrap();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    envelope(&connection, &anchor, "sealed", &["anchor-key"]);
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h2757229ad360 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET writer='other-writer', sequence=99", [],
    ).unwrap();
    let mut cache = None;
    assert_eq!(
        sealed_fleet_view_tx(&connection, &mut cache).unwrap().current("birch").len(), 1,
    );
    for sql in [
        /* sqlite-large-value: allow-for-now lv-h779f1f624a47 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures",
        /* sqlite-large-value: allow-for-now lv-h150e2cf396af owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures VALUES('birch',3,'sealed','anchor-key')",
        /* sqlite-large-value: allow-for-now lv-h71413c178150 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key='other-key'",
        /* sqlite-large-value: allow-for-now lv-h6b76f19a47e9 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key='anchor-key'",
    ] {
        let before = fleet_generation(&connection).unwrap();
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hc7fc61537933 owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        assert!(fleet_generation(&connection).unwrap() > before, "{sql}");
        assert_eq!(
            sealed_fleet_view_tx(&connection, &mut cache).unwrap(),
            crate::fleet::FleetView::from_membership(&fleet_membership_tx(&connection).unwrap()),
            "{sql}",
        );
    }
}

#[test]
fn sealed_membership_cache_reuses_cold_folds_and_recomputes_on_generation_changes() {
    let mut connection = fixture();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hf348da4e378d owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ &fleet_generation_schema()).unwrap();
    let anchor = admitted("anchor", "birch", "anchor-key", "anchor", "birch", 3);
    insert_claim(&connection, &anchor);
    let mut cache = None;
    connection.trace(Some(|sql| {
        if sql.starts_with("SELECT claims.id, claims.kind, claims.subject, claims.body,") {
            STATEMENTS.with(|count| count.set(count.get() + 1));
        }
    }));
    STATEMENTS.with(|count| count.set(0));
    // Even an empty fold with local unsealed claims must be cached.
    for _ in 0..8 {
        assert!(!sealed_membership_tx(&connection, &mut cache).unwrap().counts("anchor"));
    }
    STATEMENTS.with(|count| assert_eq!(count.get(), 1));
    let unsealed_generation = cache.as_ref().unwrap().0;
    envelope(&connection, &anchor, "sealed", &["anchor-key"]);
    for _ in 0..8 {
        assert!(sealed_membership_tx(&connection, &mut cache).unwrap().counts("anchor"));
    }
    assert!(cache.as_ref().unwrap().0 > unsealed_generation);
    STATEMENTS.with(|count| assert_eq!(count.get(), 2));
    connection.execute(/* sqlite-large-value: allow-for-now lv-hbea4b67b6a13 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures", []).unwrap();
    for _ in 0..8 {
        assert!(!sealed_membership_tx(&connection, &mut cache).unwrap().counts("anchor"));
    }
    STATEMENTS.with(|count| assert_eq!(count.get(), 3));
    connection.trace(None);
}

#[test]
fn sealed_membership_request_cache_does_not_bypass_local_batch_sealing() {
    let store = Store::open_memory("birch", Arc::new(super::runtime::Plain)).unwrap();
    store.bind_fleet("3e7a9c41-5d2b-4f86-b0c3-9a1e6d4f2b58").unwrap();
    let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
    store.load_member_key(Some(key.clone())).unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    let mut anchor = admitted("anchor", "birch", key.public(), "anchor", "birch", 1);
    anchor.fields.insert(
        "fleet_id".into(),
        serde_json::json!("3e7a9c41-5d2b-4f86-b0c3-9a1e6d4f2b58"),
    );
    let claim = store.append_claim(&ClaimInput {
        subject: anchor.subject,
        kind: anchor.kind,
        actor: None,
        fields: anchor.fields,
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    assert!(store.membership_cache.lock().unwrap().is_none());
    assert!(store.sealed_membership_cache.lock().unwrap().is_none());
    for _ in 0..3 {
        assert!(!store.fleet_membership_sealed().unwrap().counts(&claim.id));
    }
    assert!(store.sealed_membership_cache.lock().unwrap().is_some());
    assert!(store.membership_cache.lock().unwrap().is_none());
    assert!(store.fleet_membership().unwrap().counts(&claim.id));
    assert!(store.fleet_membership_sealed().unwrap().counts(&claim.id));
}

fn cache_member_window(store: &Store, key: &str, start: u64) {
    let mut anchor = admitted("anchor", "birch", key, "anchor", "birch", start.max(1));
    anchor.fields.insert("writer_floor".into(), serde_json::json!(start.saturating_sub(1)));
    anchor.signers.insert(key.into());
    let membership = Membership::fold(Some(key), &[anchor]);
    assert!(membership.counts("anchor"), "the synthetic keyed window must have a valid anchor");
    let generation = fleet_generation(&store.readers.get()).unwrap();
    *store.membership_cache.lock().unwrap() = Some((generation, membership));
}

fn authenticated_member_fixture() -> (Store, String, Arc<crate::fleet::MemberKey>) {
    const FLEET: &str = "3e7a9c41-5d2b-4f86-b0c3-9a1e6d4f2b58";
    let source = Store::open_memory("birch", Arc::new(super::runtime::Plain)).unwrap();
    source.bind_fleet(FLEET).unwrap();
    let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
    source.load_member_key(Some(key.clone())).unwrap();
    let claim = source.append_claim(&ClaimInput {
        subject: "note/provenance".into(),
        kind: "example.note".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    let target = Store::open_memory("cedar", Arc::new(super::runtime::Plain)).unwrap();
    target.bind_fleet(FLEET).unwrap();
    let exchange = source.export_replication_exchange(
        FLEET, &target.replication_inventory().unwrap(),
    ).unwrap();
    target.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
    assert_eq!(target.validate_replication_backlog().unwrap().valid, 1);
    target.project_replication_backlog().unwrap();
    assert!(target.claim_by_id(&claim.id).unwrap().is_some());
    // Membership folding is tested above. Pin a sealed keyed window so these controls
    // exercise provenance itself, including its warm request-path cache.
    cache_member_window(&target, key.public(), 0);
    (target, claim.id, key)
}

#[test]
fn authenticated_member_writer_accepts_a_foreign_origin_from_sealed_membership() {
    let (store, claim, _) = authenticated_member_fixture();
    assert!(!store.claim_has_authenticated_self_writer(&claim).unwrap());
    for _ in 0..3 {
        assert!(store.claim_has_authenticated_member_writer(&claim).unwrap());
    }
    assert!(!store.claim_has_authenticated_member_writer("missing").unwrap());
}

#[test]
fn authenticated_member_writer_accepts_valid_records_in_a_degraded_envelope() {
    let (store, claim, key) = authenticated_member_fixture();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h6d03218a7182 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_envelopes SET receipt_state='degraded'", [],
    ).unwrap();
    cache_member_window(&store, key.public(), 0);
    assert!(store.claim_has_authenticated_member_writer(&claim).unwrap());
    assert_eq!(
        store.claim_ids_with_authenticated_member_writers(std::slice::from_ref(&claim)).unwrap(),
        BTreeSet::from([claim.clone()]),
    );
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h9e7e7a841384 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_envelope_signatures SET signature='invalid'", [],
    ).unwrap();
    cache_member_window(&store, key.public(), 0);
    assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap());
    assert!(store.claim_ids_with_authenticated_member_writers(&[claim]).unwrap().is_empty());
}

#[test]
fn authenticated_member_writer_rejects_legacy_wrong_key_and_outside_window() {
    let (store, claim, key) = authenticated_member_fixture();
    assert!(store.claim_has_authenticated_member_writer(&claim).unwrap());
    let generation = fleet_generation(&store.readers.get()).unwrap();
    *store.membership_cache.lock().unwrap() = Some((generation, Membership::default()));
    assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap());
    let other = crate::fleet::MemberKey::generate().unwrap().0;
    cache_member_window(&store, other.public(), 0);
    assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap());
    cache_member_window(&store, key.public(), u64::MAX);
    assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap());
}

#[test]
fn authenticated_member_writer_requires_origin_valid_record_admitted_envelope_and_signature() {
    for sql in [
        /* sqlite-large-value: allow-for-now lv-h908d794ad3e8 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */
        "UPDATE claims SET origin='forged-origin'",
        /* sqlite-large-value: allow-for-now lv-h6ff49ec4f026 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_records SET state='invalid'",
        /* sqlite-large-value: allow-for-now lv-h98f2eff88ab2 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_records SET state='pending'",
        /* sqlite-large-value: allow-for-now lv-ha41272fe6b10 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_envelopes SET receipt_state='pending'",
        /* sqlite-large-value: allow-for-now lv-h93f4bed1c2b4 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "DELETE FROM replica_envelope_signatures",
        /* sqlite-large-value: allow-for-now lv-h1c91a0dec004 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_envelope_signatures SET signature='invalid'",
        /* sqlite-large-value: allow-for-now lv-hc8361ab9b858 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
        "UPDATE replica_envelope_signatures SET envelope_hash='other-envelope'",
    ] {
        let (store, claim, key) = authenticated_member_fixture();
        store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h622f4fdd0f6d owner=smallclaims::store::membership_query_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        cache_member_window(&store, key.public(), 0);
        assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap(), "{sql}");
        assert!(
            store.claim_ids_with_authenticated_member_writers(&[claim]).unwrap().is_empty(),
            "{sql}",
        );
    }
}

#[test]
fn authenticated_member_writer_rejects_wrong_fleet_and_shard_signature_domains() {
    for shard in [None, Some("another-shard")] {
        let (store, claim, key) = authenticated_member_fixture();
        let connection = store.connection.write();
        let (sequence, hash): (u64, String) = connection.query_row(
            "SELECT sequence, envelope_hash FROM replica_records WHERE claim_id=?1",
            [&claim], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        let signature = key.sign(&crate::fleet::envelope_signature_message_in(
            shard,
            if shard.is_some() {
                "3e7a9c41-5d2b-4f86-b0c3-9a1e6d4f2b58"
            } else {
                "another-fleet"
            },
            "birch", sequence, &hash,
        ));
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hf3e2d678372b owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
            "UPDATE replica_envelope_signatures SET signature=?1", [&signature],
        ).unwrap();
        drop(connection);
        cache_member_window(&store, key.public(), 0);
        assert!(!store.claim_has_authenticated_member_writer(&claim).unwrap());
        assert!(store.claim_ids_with_authenticated_member_writers(&[claim]).unwrap().is_empty());
    }
}

#[test]
fn batch_authenticated_member_writers_match_scalar_keyed_and_legacy_windows() {
    let (store, claim, key) = authenticated_member_fixture();
    let requested = vec![claim.clone(), "missing".into(), claim.clone()];
    let assert_parity = |expected: bool| {
        let batch = store.claim_ids_with_authenticated_member_writers(&requested).unwrap();
        assert_eq!(
            batch,
            if expected { BTreeSet::from([claim.clone()]) } else { BTreeSet::new() },
        );
        for id in &requested {
            assert_eq!(batch.contains(id), store.claim_has_authenticated_member_writer(id).unwrap());
        }
    };
    assert_parity(true);
    // The same signed envelope is legacy before its incarnation's floor.
    cache_member_window(&store, key.public(), u64::MAX);
    assert_parity(false);
    let generation = fleet_generation(&store.readers.get()).unwrap();
    *store.membership_cache.lock().unwrap() = Some((generation, Membership::default()));
    assert_parity(false);
    let other = crate::fleet::MemberKey::generate().unwrap().0;
    cache_member_window(&store, other.public(), 0);
    assert_parity(false);
    cache_member_window(&store, key.public(), 0);
    assert_parity(true);
    assert!(store.claim_ids_with_authenticated_member_writers(&[]).unwrap().is_empty());
}

#[test]
fn batch_authenticated_member_writers_verify_shared_envelope_signatures_once() {
    let (store, claim, key) = authenticated_member_fixture();
    let sibling = store.append_claim(&ClaimInput {
        subject: "note/provenance-sibling".into(),
        kind: "example.note".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    // Invent a second admitted record in the same envelope: only the signed envelope
    // identity matters to this query, not the record's retained payload.
    {
        let connection = store.connection.write();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hb5e52f377f02 owner=smallclaims::store::membership_query_tests TODO=prepare-body-and-predecessor-values */
            "UPDATE claims SET origin='birch' WHERE id=?1", [&sibling.id],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-he6cdb35f9041 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
            "INSERT INTO replica_records(
                record_ref, writer, sequence, envelope_hash, position, raw, state,
                claim_id, updated_at_unix_ms
             ) SELECT record_ref || '-sibling', writer, sequence, envelope_hash,
                      position + 1, raw, state, ?1, updated_at_unix_ms
               FROM replica_records WHERE claim_id=?2",
            params![sibling.id, claim],
        ).unwrap();
    }
    let requested = vec![claim.clone(), sibling.id.clone(), claim.clone(), sibling.id.clone()];
    for valid in [true, false] {
        if !valid {
            store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h85da58ff0363 owner=smallclaims::store::membership_query_tests TODO=chunk-or-reference-exact-wire-values */
                "UPDATE replica_envelope_signatures SET signature='invalid'", [],
            ).unwrap();
        }
        cache_member_window(&store, key.public(), 0);
        SIGNATURE_VERIFICATIONS.with(|count| count.set(0));
        let batch = store.claim_ids_with_authenticated_member_writers(&requested).unwrap();
        SIGNATURE_VERIFICATIONS.with(|count| assert_eq!(count.get(), 1));
        assert_eq!(
            batch,
            if valid {
                BTreeSet::from([claim.clone(), sibling.id.clone()])
            } else {
                BTreeSet::new()
            },
        );
        for id in &requested {
            assert_eq!(batch.contains(id), store.claim_has_authenticated_member_writer(id).unwrap());
        }
    }
}
