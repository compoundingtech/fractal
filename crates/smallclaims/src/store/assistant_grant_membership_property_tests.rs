//! Real-envelope event properties. Expected windows always come from a fresh raw fleet fold.
use super::*;

type Windows = BTreeMap<(String, String), (u64, Option<u64>)>;

fn windows(membership: &crate::fleet::Membership) -> Windows {
    membership.incarnations().map(|member| (
        (member.name.clone(), member.member_key.clone()), (member.start, member.end),
    )).collect()
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
        let proofs = signatures(&exchange).signatures;
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
