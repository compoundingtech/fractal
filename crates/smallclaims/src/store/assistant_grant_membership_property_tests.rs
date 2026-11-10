//! Real-envelope event properties. Expected windows always come from a fresh raw fleet fold.
use super::*;

type Windows = BTreeMap<(String, String), (u64, Option<u64>)>;

fn windows(membership: &crate::fleet::Membership) -> Windows {
    membership.incarnations().map(|member| (
        (member.name.clone(), member.member_key.clone()), (member.start, member.end),
    )).collect()
}

fn projection_rows(connection: &Connection) -> [Vec<Vec<SqlValue>>; 3] {
    [
        "SELECT * FROM assistant_grant_authority ORDER BY claim_id,base_claim_id",
        "SELECT * FROM assistant_grant_pending ORDER BY claim_id",
        "SELECT * FROM assistant_grant_effective ORDER BY claim_id",
    ].map(|query| {
        let mut statement = connection.prepare(query).unwrap();
        let columns = statement.column_count();
        statement.query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
            .unwrap().collect::<rusqlite::Result<_>>().unwrap()
    })
}

// Recreate the projection from the same raw evidence, then roll it back. In
// particular, never seed the oracle with cached authenticated pending flags.
fn assert_fresh_backfill(store: &Store, expected_windows: &Windows, context: &str) {
    let mut connection = store.connection.write();
    let actual = projection_rows(&connection);
    let transaction = connection.transaction().unwrap();
    reset_projection(&transaction);
    for ((writer, key), (start, end)) in expected_windows {
        transaction.execute(
            "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,?4)",
            params![writer,key,start,end],
        ).unwrap();
    }
    transaction.execute(
        "UPDATE assistant_grant_membership_state SET applied_generation=?1 WHERE id=1",
        [fleet_generation(&transaction).unwrap()],
    ).unwrap();
    while !authority::backfill_slice(&transaction, Duration::from_secs(1)).unwrap() {}
    assert_eq!(actual, projection_rows(&transaction),
        "authority, pending authentication, or effective rows differ from fresh backfill: {context}");
    transaction.rollback().unwrap();
}

fn assert_windows(store: &Store, authority_store: &Store, context: &str) {
    drain_authority(store);
    // Do not reuse the projection's preparation/diff helper, nor its membership cache.
    let expected = windows(&fleet_membership_tx(&authority_store.readers.get()).unwrap());
    assert_eq!(windows(&authority_store.fleet_membership_sealed().unwrap()), expected, "sealed fold: {context}");
    let actual: Windows = store.readers.get().prepare(
        "SELECT writer,member_key,start,end FROM assistant_grant_membership_windows ORDER BY writer,member_key",
    ).unwrap().query_map([], |row| Ok(((row.get(0)?, row.get(1)?), (row.get(2)?, row.get(3)?))))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(actual, expected, "persisted membership: {context}");
    assert_fresh_backfill(store, &expected, context);
}

// Same deterministic xorshift generator as the existing event-permutation regression.
// Fixed seeds make every failing mutation/order reproducible without external RNG state.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn shuffle<T>(values: &mut [T], state: &mut u64) {
    for index in (1..values.len()).rev() {
        let selected = (next(state) % (index as u64 + 1)) as usize;
        values.swap(index, selected);
    }
}

#[test]
fn seeded_membership_events_match_fresh_fleet_fold_after_every_event() {
    for seed in 1..=8_u64 {
        let (source, anchor) = source();
        let target = node("reader", Some(anchor.public()));
        for index in 0..6 {
            let member = MemberKey::generate().unwrap().0;
            append(&source, &format!("host/member-{index}"), "fleet.member-admitted", json!({
                "member_key": member.public(), "via": "member", "sponsor": "anchor",
                "writer_floor": index, "mode": "listening"
            }));
            source.seal_local_batches().unwrap();
        }
        append(&source, SUBJECT, GRANTED, fields());
        let exchange = export(&source, &target);
        let proofs: Vec<_> = signatures(&exchange).signatures.into_iter().filter(|proof| {
            source.readers.get().query_row(
                "SELECT EXISTS(SELECT 1 FROM batches JOIN claims ON claims.batch_id=batches.id
                 WHERE batches.origin=?1 AND batches.replica_sequence=?2 AND claims.kind='fleet.member-admitted')",
                params![proof.writer,proof.sequence], |row| row.get::<_,bool>(0),
            ).unwrap()
        }).collect();
        let mut events = Vec::new();
        for envelope in &exchange.envelopes {
            let mut single = exchange.clone();
            single.envelopes = vec![envelope.clone()];
            single.signatures.clear();
            events.push(unsigned(&single));
            events.push(signatures(&single));
        }
        let mut state = seed;
        shuffle(&mut events, &mut state);
        // On alternating seeds force anchor authentication after every admission/grant.
        // Other seeds freely allow signatures before envelopes and admission records.
        if seed % 2 == 0 {
            let anchor_sequence = exchange.envelopes.iter().map(|e| e.sequence).min().unwrap();
            let position = events.iter().position(|event|
                event.envelopes.is_empty() && event.signatures.iter().any(|proof| proof.sequence == anchor_sequence)
            ).unwrap();
            let delayed = events.remove(position);
            events.push(delayed);
        }
        for (index, event) in events.iter().enumerate() {
            receive(&target, event);
            assert_windows(&target, &target, &format!("seed={seed} receipt={index}"));
        }
        // Ensure out-of-order held records have all become admissible before mutating history.
        receive(&target, &exchange);
        assert_windows(&target, &target, "complete signed history");
        assert!(!windows(&fleet_membership_tx(&target.readers.get()).unwrap()).is_empty());

        // Every seed exercises every category, then a random suffix repeats mutations on
        // random membership batches/signatures. Each mutation and restoration is an event.
        let mut mutations: Vec<u64> = (0..4).collect();
        mutations.extend((0..20).map(|_| next(&mut state) % 4));
        for (index, mutation) in mutations.into_iter().enumerate() {
            let proof = &proofs[(next(&mut state) as usize) % proofs.len()];
            let batch: (String, String, u64) = target.readers.get().query_row(
                "SELECT batches.id,batches.origin,batches.replica_sequence FROM batches JOIN claims ON claims.batch_id=batches.id
                 WHERE claims.kind='fleet.member-admitted' ORDER BY claims.store_index LIMIT 1 OFFSET ?1",
                [next(&mut state) % 7], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap();
            {
                let connection = target.connection.write();
                match mutation {
                    0 => { connection.execute("DELETE FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                        params![proof.writer,proof.sequence,proof.hash,proof.member_key]).unwrap(); }
                    1 => { connection.execute("UPDATE replica_envelope_signatures SET member_key=?5 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                        params![proof.writer,proof.sequence,proof.hash,proof.member_key,format!("untrusted-{seed}-{index}")]).unwrap(); }
                    2 => { connection.execute("UPDATE batches SET origin=?2 WHERE id=?1", params![batch.0,format!("unrelated-{seed}-{index}")]).unwrap(); }
                    _ => { connection.execute("UPDATE batches SET replica_sequence=?2 WHERE id=?1", params![batch.0,batch.2+10000]).unwrap(); }
                }
            }
            assert_windows(&target, &target, &format!("seed={seed} mutation={index} kind={mutation}"));
            {
                let connection = target.connection.write();
                if mutation == 1 {
                    connection.execute("DELETE FROM replica_envelope_signatures WHERE member_key=?1", [format!("untrusted-{seed}-{index}")]).unwrap();
                }
                if mutation < 2 { insert_raw_signature(&connection, proof); }
                else {
                    connection.execute("UPDATE batches SET origin=?2,replica_sequence=?3 WHERE id=?1", params![batch.0,batch.1,batch.2]).unwrap();
                }
            }
            assert_windows(&target, &target, &format!("seed={seed} restored={index}"));
        }
    }
}

#[test]
fn inherited_generation_membership_events_match_authoritative_raw_fold() {
    let (source, anchor) = source();
    let authority_store = Arc::new(node("reader", Some(anchor.public())));
    let shard = Store::open_memory("shard-reader", Arc::new(Plain)).unwrap();
    shard.bind_shard(&crate::shards::ShardId::parse("membership-property").unwrap()).unwrap();
    shard.bind_fleet_authority(authority_store.clone()).unwrap();
    assert_windows(&shard, &authority_store, "initial inherited snapshot");
    let own_generation = fleet_generation(&shard.readers.get()).unwrap();
    for index in 0..5 {
        let member = MemberKey::generate().unwrap().0;
        append(&source, &format!("host/inherited-{index}"), "fleet.member-admitted", json!({
            "member_key": member.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": index, "mode": "listening"
        }));
        let exchange = export(&source, &authority_store);
        receive(&authority_store, &unsigned(&exchange));
        assert_windows(&shard, &authority_store, "inherited unsigned admission");
        let mut proofs = signatures(&exchange);
        proofs.signatures.reverse();
        for proof in proofs.signatures.clone() {
            proofs.signatures = vec![proof];
            receive(&authority_store, &proofs);
            assert_windows(&shard, &authority_store, "inherited late signature");
        }
        // No local fleet event can be the trigger for any of these snapshot changes.
        assert_eq!(fleet_generation(&shard.readers.get()).unwrap(), own_generation);
    }
    assert_eq!(windows(&fleet_membership_tx(&authority_store.readers.get()).unwrap()).len(), 6);
}

fn fixture_fields(suffix: &str, profile: &str, digest: char) -> Value {
    let mut value = fields();
    value["grant_id"] = json!(format!("assistant-grant/{suffix}"));
    value["session_actor"] = json!(format!("assistant/chatgpt/session/assistant-grant/{suffix}"));
    value["profile"] = json!(profile);
    value["credential_hash"] = json!(digest.to_string().repeat(64));
    value
}

// Independent raw-history oracle for this closed fixture contract. In particular this
// does not call structurally_valid, transition_valid, rebuild_subject or candidates,
// and never reads assistant_grant_* tables. Pending and dirty state is not authority.
fn raw_conflict_witnesses(store: &Store, ignored_profile: &str, now: u64) -> Vec<String> {
    let claims = raw_claims(store);
    let trusted = |claim: &ClaimRecord| store.claim_has_authenticated_member_writer(&claim.id).unwrap();
    let matches = |grant: &ClaimRecord, claim: &ClaimRecord| {
        claim.subject == grant.subject && claim.origin == grant.origin && claim.actor == grant.actor
            && ["issuer", "grant_id", "principal", "profile"].iter().all(|field|
                claim.body["fields"][*field] == grant.body["fields"][*field])
    };
    let mut result = Vec::new();
    for grant in claims.iter().filter(|claim| claim.kind == GRANTED && trusted(claim)) {
        let fields = &grant.body["fields"];
        let suffix = fields["grant_id"].as_str().unwrap().strip_prefix("assistant-grant/").unwrap();
        assert_eq!(grant.subject, format!("custom/client/assistant-v1-{suffix}"));
        assert_eq!(grant.actor.as_deref(), Some(OWNER));
        assert_eq!(fields["issuer"], OWNER);
        assert_eq!(fields["principal"], "assistant/chatgpt");
        let selected = claims.iter().rev().find(|claim|
            claim.kind == POLICY && matches(grant, claim) && trusted(claim)
        ).unwrap_or(grant);
        let policy = &selected.body["fields"];
        assert_eq!(policy["session_actor"], fields["session_actor"]);
        assert_eq!(policy["disclosure_consent"], true);
        assert_eq!(policy["scopes"], json!(["read.assistant.summary"]));
        let revoked = claims.iter().any(|claim|
            claim.kind == REVOKED && matches(grant, claim) && trusted(claim));
        if !revoked && policy["expires_at_unix_ms"].as_u64().unwrap() > now
            && fields["profile"] != ignored_profile {
            result.push(grant.id.clone());
        }
    }
    result.sort();
    result
}

fn assert_conflict_oracle(store: &Store, own: &ClaimRecord, witness: &ClaimRecord, expected_witness: bool) {
    let expected = raw_conflict_witnesses(store, "assistant", 0);
    assert_eq!(expected.contains(&witness.id), expected_witness);
    let mut actual: Vec<_> = authority::candidates(&store.readers.get(), "assistant/chatgpt", OWNER,
        Some(0), Some(&own.id), Some("assistant")).unwrap().into_iter().map(|claim| claim.id).collect();
    actual.sort();
    assert_eq!(actual, expected, "conflict witnesses differ from authenticated raw history");
    assert!(authority::contains(&store.readers.get(), &own.id).unwrap(), "unrelated subject fence denied own bearer");
    assert_eq!(authority::by_credential(&store.readers.get(), &"a".repeat(64)).unwrap().unwrap().id, own.id);
}

#[test]
fn fenced_opposite_profile_witness_matches_independent_raw_history_oracle() {
    for seed in 1..=8_u64 {
        let (source, key) = source();
        let target = node("reader", Some(key.public()));
        let own = append(&source, SUBJECT, GRANTED, fields());
        let suffix = "abcdef0123456789abcdef01";
        let subject = format!("custom/client/assistant-v1-{suffix}");
        let service_fields = fixture_fields(suffix, "service", 'b');
        let witness = append(&source, &subject, GRANTED, service_fields.clone());
        receive(&target, &export(&source, &target));
        assert_conflict_oracle(&target, &own, &witness, true);
        // A valid, genuinely signed policy remains the effective snapshot until a
        // later revoke authenticates, even when that subject is dirty/fenced.
        let mut renewal = service_fields.clone();
        renewal["expires_at_unix_ms"] = json!(20000);
        append(&source, &subject, POLICY, renewal);
        receive(&target, &export(&source, &target));
        append(&source, &subject, REVOKED, service_fields);
        let revoke_exchange = export(&source, &target);
        receive_pre_projection_unsigned_history(&target, &revoke_exchange);
        assert_conflict_oracle(&target, &own, &witness, true);
        assert!(!authority::contains(&target.readers.get(), &witness.id).unwrap());
        {
            target.connection.write().execute("INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&witness.id]).unwrap();
        }
        assert_conflict_oracle(&target, &own, &witness, true);
        drain_authority(&target);
        assert_conflict_oracle(&target, &own, &witness, true);
        let mut proofs = signatures(&revoke_exchange);
        let mut state = seed;
        shuffle(&mut proofs.signatures, &mut state);
        for proof in proofs.signatures.clone() {
            proofs.signatures = vec![proof];
            receive(&target, &proofs);
            let expected = raw_conflict_witnesses(&target, "assistant", 0).contains(&witness.id);
            assert_conflict_oracle(&target, &own, &witness, expected);
        }
        assert_conflict_oracle(&target, &own, &witness, false);
        // Conversely, pending work on the bearer's own subject must deny that
        // bearer, without changing the independent conflict-history answer.
        append(&source, SUBJECT, POLICY, fields());
        receive_pre_projection_unsigned_history(&target, &export(&source, &target));
        assert!(!authority::contains(&target.readers.get(), &own.id).unwrap());
        assert!(authority::by_credential(&target.readers.get(), &"a".repeat(64)).unwrap().is_none());
        assert!(authority::subject_ready(&target.readers.get(), SUBJECT).is_err());
        assert!(raw_conflict_witnesses(&target, "assistant", 0).is_empty());
        assert!(authority::candidates(&target.readers.get(), "assistant/chatgpt", OWNER,
            Some(0), Some(&own.id), Some("assistant")).unwrap().is_empty());
    }
}

fn learned_fence_revalidates_signed_transition(kind: &str) {
    for seed in 1..=8_u64 {
        let (anchor, anchor_key) = source();
        let peer_key = Arc::new(MemberKey::generate().unwrap().0);
        let peer = node("peer", Some(anchor_key.public()));
        peer.set_member_key(Some(peer_key.clone())).unwrap();
        let target = node("reader", Some(anchor_key.public()));
        let admission = append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": peer_key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
        let admitted = export(&anchor, &target);
        receive(&target, &admitted);
        assert_windows(&target, &target, "peer admitted");
        receive(&peer, &admitted);
        let grant = append(&peer, SUBJECT, GRANTED, fields());
        let granted = export(&peer, &target);
        let grant_sequence: u64 = peer.readers.get().query_row(
            "SELECT replica_sequence FROM batches WHERE id=?1", [&grant.batch_id],
            |row| row.get(0),
        ).unwrap();
        receive(&target, &granted);
        assert_windows(&target, &target, "signed base grant");
        let mut changed = fields();
        changed["expires_at_unix_ms"] = json!(20000);
        let transition = append(&peer, SUBJECT, kind, changed);
        receive(&target, &export(&peer, &target));
        assert_windows(&target, &target, "signed transition before fence");
        assert!(authority::transition(&target.readers.get(), &grant, kind).unwrap().is_some());
        {
            let connection = target.connection.write();
            let authenticated: bool = connection.query_row(
                "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
                [&transition.id], |row| row.get(0),
            ).unwrap();
            assert!(authenticated);
            // The base is at the cursor, but its authenticated transition is
            // still ahead of it. A writer queue must fence the whole subject.
            connection.execute(
                "INSERT OR REPLACE INTO assistant_grant_membership_dirty VALUES(?1,?2,?3)",
                params![grant.origin,grant_sequence,grant.id],
            ).unwrap();
            assert!(authority::subject_ready(&connection, SUBJECT).is_err());
            assert!(!authority::contains(&connection, &grant.id).unwrap());
            assert!(authority::by_credential(&connection, &"a".repeat(64)).unwrap().is_none());
            assert!(authority::transition(&connection, &grant, kind).is_err());
        }
        assert_windows(&target, &target, "authenticated writer queue drained");
        let removal = append(&anchor, "host/peer", "fleet.member-removed", json!({
            "member_key": peer_key.public(), "high_water": grant_sequence,
            "reason": "membership equivalence regression"
        }));
        let fenced = export(&anchor, &target);
        receive(&target, &fenced);
        assert_windows(&target, &target, "later learned fence shrinks writer window");
        let retained: bool = target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_authority WHERE claim_id=?1)",
            [&transition.id], |row| row.get(0),
        ).unwrap();
        assert!(!retained, "signed transition retained authority beyond its learned fence");
        assert!(!target.claim_has_authenticated_member_writer(&transition.id).unwrap());
        // Fresh backfill retains the unauthenticated transition as a subject
        // fence, even though the effective conflict snapshot falls back.
        assert!(authority::subject_ready(&target.readers.get(), SUBJECT).is_err());
        assert!(!authority::contains(&target.readers.get(), &grant.id).unwrap());
        let effective: (String, bool) = target.readers.get().query_row(
            "SELECT expires_at,revoked FROM assistant_grant_effective WHERE claim_id=?1",
            [&grant.id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(effective, (authority::expiry_key(10000), false));
        let authenticated: bool = target.readers.get().query_row(
            "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
            [&transition.id], |row| row.get(0),
        ).unwrap();
        assert!(!authenticated, "cached transition authentication survived shrink");

        // Repeat window expansion/shrink and complete key deletion/restoration in
        // seeded orders. Every event compares all projection columns, not just
        // effective grants: a stale losing revoke must also disappear.
        let proof_for = |claim: &ClaimRecord, exchange: &ReplicationExchange| {
            let sequence: u64 = anchor.readers.get().query_row(
                "SELECT replica_sequence FROM batches WHERE id=?1", [&claim.batch_id],
                |row| row.get(0),
            ).unwrap();
            signatures(exchange).signatures.into_iter().find(|proof|
                proof.writer == "anchor" && proof.sequence == sequence).unwrap()
        };
        let admission_proof = proof_for(&admission, &admitted);
        let removal_proof = proof_for(&removal, &fenced);
        let mut events = vec![&admission_proof, &removal_proof];
        let mut state = seed;
        shuffle(&mut events, &mut state);
        for (index, proof) in events.into_iter().enumerate() {
            target.connection.write().execute(
                "DELETE FROM replica_envelope_signatures
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                params![proof.writer,proof.sequence,proof.hash,proof.member_key],
            ).unwrap();
            assert_windows(&target, &target, &format!("seed={seed} kind={kind} deleted={index}"));
            if proof.sequence == admission_proof.sequence {
                assert!(!authority::contains(&target.readers.get(), &grant.id).unwrap(),
                    "grant survived deletion of its membership key");
            }
            insert_raw_signature(&target.connection.write(), proof);
            assert_windows(&target, &target, &format!("seed={seed} kind={kind} restored={index}"));
        }
    }
}

#[test]
fn signed_policy_beyond_later_learned_fence_loses_authority() {
    learned_fence_revalidates_signed_transition(POLICY);
}

#[test]
fn signed_revoke_beyond_later_learned_fence_loses_authority_and_unrevokes_effective_grant() {
    learned_fence_revalidates_signed_transition(REVOKED);
}
