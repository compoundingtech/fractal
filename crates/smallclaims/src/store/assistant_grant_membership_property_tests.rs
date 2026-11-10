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
        "SELECT claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record
         FROM assistant_grant_pending ORDER BY claim_id",
        "SELECT * FROM assistant_grant_effective ORDER BY claim_id",
    ].map(|query| {
        let mut statement = connection.prepare(query).unwrap();
        let columns = statement.column_count();
        statement.query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
            .unwrap().collect::<rusqlite::Result<_>>().unwrap()
    })
}

// Only wire validation is shared with production. Authentication comes from raw
// provenance and durable local admission; attachment/latest/revocation are a
// separate Rust fold, never a projection query or a backfill invocation.
fn oracle_pending_valid(claim: &ClaimRecord) -> bool {
    let fields = &claim.body["fields"];
    let principal = fields.get("principal").and_then(Value::as_str).unwrap_or("assistant/chatgpt");
    let profile = fields.get("profile").and_then(Value::as_str).unwrap_or("assistant");
    let Some(id) = fields["grant_id"].as_str() else { return false; };
    let suffix = id.strip_prefix("assistant-grant/").unwrap_or(id);
    let identity = fields.is_object()
        && fields.get("principal").is_none_or(|value| value.as_str().is_some())
        && fields.get("profile").is_none_or(|value| value.as_str().is_some())
        && matches!(principal, "assistant/chatgpt" | "assistant/mcp")
        && matches!(profile, "assistant" | "service")
        && suffix.len() == 24 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        && claim.subject.strip_prefix("custom/client/assistant-v1-") == Some(suffix)
        && claim.actor.as_deref().is_some_and(|actor| fields["issuer"].as_str() == Some(actor));
    let digest = |value: &Value| value.as_str().is_some_and(|text|
        text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !identity { return false; }
    match claim.kind.as_str() {
        GRANTED => fields.get("credential").is_none() && fields.get("code").is_none()
            && digest(&fields["credential_hash"])
            && fields.get("code_hash").is_none_or(digest),
        POLICY => {
            // A policy need not carry a credential. Supply one only on its
            // synthetic base to ask the shared structural validator about it.
            let mut base = claim.clone();
            base.body["fields"]["credential_hash"] = json!("0".repeat(64));
            base.body["fields"].as_object_mut().unwrap().remove("credential");
            base.body["fields"].as_object_mut().unwrap().remove("code");
            base.body["fields"].as_object_mut().unwrap().remove("code_hash");
            authority::structurally_valid(&base, claim)
        }
        REVOKED => true,
        _ => false,
    }
}

fn assert_raw_projection_oracle(store: &Store, expected_windows: &Windows, context: &str) {
    let claims = raw_claims(store);
    let connection = store.readers.get();
    let local_ids: std::collections::BTreeSet<String> = connection.prepare(
        "SELECT claim_id FROM assistant_grant_local_appends",
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    let pending: Vec<_> = claims.iter().filter(|claim| oracle_pending_valid(claim))
        .map(|claim| {
            let authenticated = (claim.origin == store.origin && local_ids.contains(&claim.id))
                || store.claim_has_authenticated_member_writer(&claim.id).unwrap();
            (claim, authenticated)
        }).collect();
    let mut expected_pending: Vec<_> = pending.iter().map(|(claim, authenticated)|
        (claim.id.clone(), *authenticated)).collect();
    expected_pending.sort();
    let actual_pending: Vec<(String, bool)> = connection.prepare(
        "SELECT claim_id,authenticated FROM assistant_grant_pending ORDER BY claim_id",
    ).unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(actual_pending, expected_pending, "raw pending/authentication fold: {context}");
    for (claim, trusted) in &pending {
        let (key, sequence, hash, position): (Option<String>, Option<u64>, Option<String>, Option<u64>) =
            connection.query_row(
                "SELECT auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position
                 FROM assistant_grant_pending WHERE claim_id=?1",
                [&claim.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
        if !trusted {
            assert_eq!((key, sequence, hash, position), (None, None, None, None),
                "unauthenticated row retained a proof: {context}");
            continue;
        }
        if claim.origin == store.origin && local_ids.contains(&claim.id) {
            assert_eq!((key, sequence, hash, position), (None, None, None, None),
                "local admission is not a remote proof");
            continue;
        }
        let key = key.expect("authenticated remote row must record its verified key");
        let sequence = sequence.expect("authenticated remote row must record its proof sequence");
        let hash = hash.expect("authenticated remote row must record its proof envelope");
        let position = position.expect("authenticated remote row must record its proof record position");
        let (start, end) = expected_windows.get(&(claim.origin.clone(), key.clone()))
            .expect("recorded proof key must remain in fresh fleet membership");
        assert!(sequence >= *start && end.is_none_or(|end| sequence <= end),
            "recorded proof lost fresh window coverage: {context}");
        let fleet_id = fleet_meta(&connection, "assistant_grant_authority_fleet_id").unwrap().unwrap();
        let shard = fleet_meta(&connection, "shard_id").unwrap();
        let evidence: Vec<String> = connection.prepare(
            "SELECT signatures.signature FROM replica_records AS records
             JOIN replica_envelopes AS envelopes USING(writer,sequence,envelope_hash)
             JOIN replica_envelope_signatures AS signatures USING(writer,sequence,envelope_hash)
             WHERE records.claim_id=?1 AND records.writer=?2 AND records.sequence=?3
               AND signatures.member_key=?4 AND records.state='valid'
               AND records.envelope_hash=?5 AND records.position=?6
               AND envelopes.receipt_state IN ('validated','degraded')",
        ).unwrap().query_map(params![claim.id,claim.origin,sequence,key,hash,position],
            |row| row.get(0)).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert!(evidence.iter().any(|signature|
            VerifiedEnvelopeSignature::verify(shard.as_deref(), &fleet_id, &claim.origin,
                sequence, &hash, &key, signature).is_some()),
            "recorded proof was not actually verified: {context}");
    }

    let authenticated: Vec<_> = pending.iter().filter_map(|(claim, trusted)|
        trusted.then_some(*claim)).collect();
    let text = |value: &str| SqlValue::Text(value.to_owned());
    let optional_text = |value: Option<&str>| value.map_or(SqlValue::Null, |value| text(value));
    let mut expected_authority = Vec::new();
    let mut expected_effective = Vec::new();
    for grant in authenticated.iter().copied().filter(|claim| claim.kind == GRANTED) {
        let transitions: Vec<_> = authenticated.iter().copied().filter(|claim|
            claim.kind != GRANTED && authority::transition_valid(grant, claim)).collect();
        let selected_policy = transitions.iter().copied().filter(|claim| claim.kind == POLICY)
            .max_by_key(|claim| claim.store_index);
        let base_attached = authority::structurally_valid(grant, grant) || selected_policy.is_some();
        for claim in base_attached.then_some(grant).into_iter().chain(transitions.iter().copied()) {
            let fields = &claim.body["fields"];
            expected_authority.push(vec![
                text(&claim.id), text(&grant.id), text(&claim.subject), text(&claim.kind),
                text(&claim.origin), optional_text(claim.actor.as_deref()),
                text(fields["principal"].as_str().unwrap_or("assistant/chatgpt")),
                text(fields["profile"].as_str().unwrap_or("assistant")),
                text(&format!("{:020}", fields["expires_at_unix_ms"].as_u64().unwrap_or(0))),
                SqlValue::Integer(claim.store_index as i64),
                optional_text((claim.kind == GRANTED).then(|| fields["credential_hash"].as_str()).flatten()),
                text(&serde_json::to_string(claim).unwrap()),
            ]);
        }
        let policy = selected_policy.unwrap_or(grant);
        if !authority::structurally_valid(grant, policy) { continue; }
        let fields = &policy.body["fields"];
        expected_effective.push(vec![
            text(&grant.id), text(&grant.subject),
            text(fields["principal"].as_str().unwrap_or("assistant/chatgpt")),
            optional_text(grant.actor.as_deref()),
            text(fields["profile"].as_str().unwrap_or("assistant")),
            text(&format!("{:020}", fields["expires_at_unix_ms"].as_u64().unwrap())),
            SqlValue::Integer(i64::from(transitions.iter().any(|claim| claim.kind == REVOKED))),
            text(&serde_json::to_string(grant).unwrap()),
        ]);
    }
    let row_key = |row: &Vec<SqlValue>| {
        let SqlValue::Text(claim_id) = &row[0] else { unreachable!() };
        let SqlValue::Text(base_id) = &row[1] else { unreachable!() };
        (claim_id.clone(), base_id.clone())
    };
    expected_authority.sort_by_key(row_key);
    expected_effective.sort_by_key(row_key);
    let [actual_authority, _, actual_effective] = projection_rows(&connection);
    assert_eq!(actual_authority, expected_authority, "raw authority fold: {context}");
    assert_eq!(actual_effective, expected_effective, "raw effective fold: {context}");
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
    assert_raw_projection_oracle(store, &expected, context);
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
            // Only the recorded key beyond the cursor is queued. Its transition
            // fences this subject even though the base itself is behind it.
            connection.execute(
                "INSERT INTO assistant_grant_retractions
                 (writer,member_key,lower_sequence,upper_sequence,sequence,claim_id)
                 VALUES(?1,?2,?3,?4,?3,?5)",
                params![grant.origin,peer_key.public(),grant_sequence,i64::MAX,grant.id],
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

fn receive_without_authority_maintenance(target: &Store, exchange: &ReplicationExchange) {
    target.receive_replication_exchange(&exchange.peer, FLEET, exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    target.project_replication_backlog().unwrap();
}

fn assert_established_grants_ready(store: &Store, grants: &[ClaimRecord]) {
    let connection = store.readers.get();
    for grant in grants {
        assert!(authority::subject_ready(&connection, &grant.subject).is_ok(),
            "unaffected subject fenced: {}", grant.subject);
        assert!(authority::contains(&connection, &grant.id).unwrap(),
            "unaffected grant denied: {}", grant.id);
    }
}

#[test]
fn repeated_future_floor_window_additions_do_not_reauthenticate_or_fence_established_history() {
    let (sponsor, key) = source();
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    let source = node("peer", Some(key.public()));
    source.set_member_key(Some(peer_key.clone())).unwrap();
    let target = node("reader", Some(key.public()));
    append(&sponsor, "host/peer", "fleet.member-admitted", json!({
        "member_key": peer_key.public(), "via": "member", "sponsor": "anchor",
        "writer_floor": 0, "mode": "listening"
    }));
    receive(&source, &export(&sponsor, &source));
    receive(&target, &export(&sponsor, &target));
    let grants: Vec<_> = (0..authority::CLAIMS_PER_SLICE * 3).map(|index| {
        let suffix = format!("{index:024x}");
        append(&source, &format!("custom/client/assistant-v1-{suffix}"), GRANTED,
            fixture_fields(&suffix, "assistant", 'a'))
    }).collect();
    receive(&target, &export(&source, &target));
    assert_windows(&target, &target, "established authenticated history");
    for index in 0..6 {
        let future_key = MemberKey::generate().unwrap().0;
        append(&sponsor, "host/peer", "fleet.member-admitted", json!({
            "member_key": future_key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 1_000_000 + index, "mode": "listening"
        }));
        authority::take_authentication_attempts();
        authority::take_authentication_claims();
        receive_without_authority_maintenance(&target, &export(&sponsor, &target));
        assert_established_grants_ready(&target, &grants);
        loop {
            let complete = target.advance_assistant_grant_authority_backfill().unwrap();
            assert_established_grants_ready(&target, &grants);
            if complete { break; }
        }
        let fresh_windows = windows(&fleet_membership_tx(&target.readers.get()).unwrap());
        assert_eq!(fresh_windows.len(), 3 + index as usize,
            "future-floor admission failed to add a real window");
        assert!(fresh_windows.contains_key(&("peer".to_owned(), future_key.public().to_owned())));
        assert_eq!(authority::take_authentication_attempts(), 0,
            "future-floor addition reauthenticated established history at event {index}");
        assert!(authority::take_authentication_claims().is_empty());
        assert_windows(&target, &target, &format!("future-floor addition {index}"));
    }
}

#[test]
fn membership_retraction_reauthenticates_only_rows_using_the_lost_recorded_key() {
    let (anchor, anchor_key) = source();
    let first_key = Arc::new(MemberKey::generate().unwrap().0);
    let second_key = Arc::new(MemberKey::generate().unwrap().0);
    let peer = node("peer", Some(anchor_key.public()));
    let target = node("reader", Some(anchor_key.public()));
    for key in [&first_key, &second_key] {
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
    }
    receive(&target, &export(&anchor, &target));
    receive(&peer, &export(&anchor, &peer));
    peer.set_member_key(Some(first_key.clone())).unwrap();
    let removed_grant = append(&peer, SUBJECT, GRANTED, fields());
    receive(&target, &export(&peer, &target));
    peer.set_member_key(Some(second_key.clone())).unwrap();
    let suffix = "abcdef0123456789abcdef01";
    let retained_grant = append(&peer, &format!("custom/client/assistant-v1-{suffix}"),
        GRANTED, fixture_fields(suffix, "service", 'b'));
    receive(&target, &export(&peer, &target));
    assert_windows(&target, &target, "two recorded proof keys on one writer");
    for (grant, key) in [(&removed_grant, &first_key), (&retained_grant, &second_key)] {
        let recorded_key: String = target.readers.get().query_row(
            "SELECT auth_member_key FROM assistant_grant_pending WHERE claim_id=?1",
            [&grant.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(recorded_key, key.public());
    }
    append(&anchor, "host/peer", "fleet.member-removed", json!({
        "member_key": first_key.public(), "high_water": 0, "reason": "proof-key regression"
    }));
    authority::take_authentication_attempts();
    authority::take_authentication_claims();
    receive_without_authority_maintenance(&target, &export(&anchor, &target));
    loop {
        let complete = target.advance_assistant_grant_authority_backfill().unwrap();
        assert_established_grants_ready(&target, std::slice::from_ref(&retained_grant));
        if complete { break; }
    }
    // One failed key page followed by end-of-evidence may require more than one
    // authentication call. Record exactly which claims were refreshed below.
    assert!(authority::take_authentication_attempts() > 0);
    let refreshed = authority::take_authentication_claims();
    assert!(!refreshed.is_empty());
    assert!(refreshed.iter().all(|claim_id| claim_id == &removed_grant.id),
        "retraction refreshed rows using an unaffected proof key: {refreshed:?}");
    let connection = target.readers.get();
    let lost: bool = connection.query_row(
        "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
        [&removed_grant.id], |row| row.get(0),
    ).unwrap();
    assert!(!lost);
    assert!(authority::subject_ready(&connection, &removed_grant.subject).is_err());
    drop(connection);
    assert_windows(&target, &target, "only the lost proof key is retracted");
}

#[test]
fn sole_proof_withdrawal_survives_nonretracting_same_writer_window_addition() {
    for mutation in ["signature-delete", "signature-update", "record-invalid", "record-delete", "record-invalid-refresh", "record-repaired-refresh", "envelope-invalid", "envelope-delete", "claim-delete"] {
        let (anchor, anchor_key) = source();
        let peer_key = Arc::new(MemberKey::generate().unwrap().0);
        let peer = node("peer", Some(anchor_key.public()));
        peer.set_member_key(Some(peer_key.clone())).unwrap();
        let target = node("reader", Some(anchor_key.public()));
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": peer_key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
        receive(&peer, &export(&anchor, &peer));
        receive(&target, &export(&anchor, &target));
        let grant = append(&peer, SUBJECT, GRANTED, fields());
        receive(&target, &export(&peer, &target));
        assert_windows(&target, &target, "sole signed proof established");
        let (sequence, hash, position): (u64, String, u64) = target.readers.get().query_row(
            "SELECT auth_sequence,auth_envelope_hash,auth_record_position
             FROM assistant_grant_pending WHERE claim_id=?1",
            [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        {
            let mut writer = target.connection.write();
            let connection = writer.transaction().unwrap();
            let query = match mutation {
                "signature-delete" =>
                    "DELETE FROM replica_envelope_signatures
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                "signature-update" =>
                    "UPDATE replica_envelope_signatures SET signature='invalid'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                "record-invalid" | "record-invalid-refresh" =>
                    "UPDATE replica_records SET state='invalid'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "record-repaired-refresh" =>
                    "UPDATE replica_records SET state='repaired'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "record-delete" =>
                    "DELETE FROM replica_records
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "envelope-invalid" =>
                    "UPDATE replica_envelopes SET receipt_state=?4
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                "envelope-delete" =>
                    "DELETE FROM replica_envelopes
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND ?4 IS NOT NULL",
                "claim-delete" =>
                    "DELETE FROM claims WHERE id IN(
                       SELECT claim_id FROM assistant_grant_pending
                       WHERE origin=?1 AND auth_sequence=?2 AND auth_envelope_hash=?3 AND auth_record_position=?4)",
                _ => unreachable!(),
            };
            let last = if mutation.starts_with("signature-") {
                SqlValue::Text(peer_key.public().to_owned())
            } else if mutation == "envelope-invalid" {
                SqlValue::Text("pending".to_owned())
            } else {
                SqlValue::Integer(position as i64)
            };
            assert_eq!(connection.execute(query, params![grant.origin,sequence,hash,last]).unwrap(), 1);
            if mutation.ends_with("-refresh") {
                // Prepared-envelope revalidation changes evidence and invokes
                // its non-verifying hook in the same writer transaction.
                connection.execute(
                    "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&grant.id],
                ).unwrap();
                authority::refresh_envelope(&connection,&grant.origin,sequence,&hash).unwrap();
                assert!(!connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)",
                    [&grant.id], |row| row.get::<_,bool>(0),
                ).unwrap(), "fixture must exercise receipt clearing ordinary dirty work");
                assert!(connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
                    [&grant.id], |row| row.get::<_,bool>(0),
                ).unwrap(), "receipt must not discharge the real verification obligation");
            }
            assert!(authority::subject_ready(&connection, &grant.subject).is_err(),
                "withdrawal did not fence its dependent: {mutation}");
            assert!(!authority::contains(&connection, &grant.id).unwrap(),
                "withdrawn proof still authorized its bearer: {mutation}");
            connection.commit().unwrap();
        }
        // Keep the original key's coverage intact. This real new window on the
        // same writer must not hide the queued loss behind an authenticated flag.
        let future_key = MemberKey::generate().unwrap().0;
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": future_key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 1_000_000, "mode": "listening"
        }));
        receive_without_authority_maintenance(&target, &export(&anchor, &target));
        let expected = windows(&fleet_membership_tx(&target.readers.get()).unwrap());
        assert_eq!(expected.get(&("peer".to_owned(), peer_key.public().to_owned())), Some(&(0, None)));
        assert!(expected.contains_key(&("peer".to_owned(), future_key.public().to_owned())));
        assert!(!target.claim_has_authenticated_member_writer(&grant.id).unwrap(), "{mutation}");
        assert_windows(&target, &target, mutation);
        assert!(!authority::contains(&target.readers.get(), &grant.id).unwrap(), "{mutation}");
    }
}

#[test]
fn restored_window_coverage_cancels_pending_retraction_fence() {
    let (anchor, anchor_key) = source();
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    let peer = node("peer", Some(anchor_key.public()));
    peer.set_member_key(Some(peer_key.clone())).unwrap();
    let target = node("reader", Some(anchor_key.public()));
    append(&anchor, "host/peer", "fleet.member-admitted", json!({
        "member_key": peer_key.public(), "via": "member", "sponsor": "anchor",
        "writer_floor": 0, "mode": "listening"
    }));
    receive(&peer, &export(&anchor, &peer));
    receive(&target, &export(&anchor, &target));
    let grants: Vec<_> = (0..authority::CLAIMS_PER_SLICE * 3).map(|index| {
        let suffix = format!("{index:024x}");
        append(&peer, &format!("custom/client/assistant-v1-{suffix}"), GRANTED,
            fixture_fields(&suffix, "assistant", 'a'))
    }).collect();
    receive(&target, &export(&peer, &target));
    assert_windows(&target, &target, "restoration fixture established");
    let removal = append(&anchor, "host/peer", "fleet.member-removed", json!({
        "member_key": peer_key.public(), "high_water": 0, "reason": "restore coverage regression"
    }));
    let removed = export(&anchor, &target);
    let removal_sequence: u64 = anchor.readers.get().query_row(
        "SELECT replica_sequence FROM batches WHERE id=?1", [&removal.batch_id],
        |row| row.get(0),
    ).unwrap();
    let proof = signatures(&removed).signatures.into_iter().find(|proof|
        proof.writer == "anchor" && proof.sequence == removal_sequence).unwrap();
    receive_without_authority_maintenance(&target, &removed);
    assert!(!target.advance_assistant_grant_authority_backfill().unwrap());
    let retained_id: String = target.readers.get().query_row(
        "SELECT pending.claim_id FROM assistant_grant_pending AS pending
         JOIN assistant_grant_retractions AS retractions
           ON pending.origin=retractions.writer AND pending.auth_member_key=retractions.member_key
         WHERE pending.authenticated=1
           AND pending.auth_sequence BETWEEN retractions.lower_sequence AND retractions.upper_sequence
         ORDER BY pending.claim_id LIMIT 1",
        [], |row| row.get(0),
    ).unwrap();
    let retained = grants.iter().find(|grant| grant.id == retained_id).unwrap();
    assert!(authority::subject_ready(&target.readers.get(), &retained.subject).is_err());
    // Withdraw only the signed removal. Raw fleet history expands the original
    // key again while its old retraction page is still outstanding.
    target.connection.write().execute(
        "DELETE FROM replica_envelope_signatures
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
        params![proof.writer,proof.sequence,proof.hash,proof.member_key],
    ).unwrap();
    target.advance_assistant_grant_authority_backfill().unwrap();
    assert_established_grants_ready(&target, std::slice::from_ref(retained));
    let pending_retractions: u64 = target.readers.get().query_row(
        "SELECT COUNT(*) FROM assistant_grant_retractions WHERE writer='peer' AND member_key=?1",
        [peer_key.public()], |row| row.get(0),
    ).unwrap();
    assert_eq!(pending_retractions, 0, "restored coverage retained a stale subject fence");
    assert_windows(&target, &target, "restored coverage after partial retraction");
    assert_established_grants_ready(&target, &grants);
}
