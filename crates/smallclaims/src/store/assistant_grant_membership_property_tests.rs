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
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hfff109a32004 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=trace-statement-and-value-representation */ query).unwrap();
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
    let optional_text = |value: Option<&str>| value.map_or(SqlValue::Null, text);
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
            /* sqlite-large-value: allow-for-now lv-hbb32eb74d7c6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,?4)",
            params![writer,key,start,end],
        ).unwrap();
    }
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h268eaf45d33c owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_state SET applied_generation=?1 WHERE id=1",
        [fleet_generation(&transaction).unwrap()],
    ).unwrap();
    let mut completed = false;
    for _ in 0..4096 {
        if authority::backfill_slice(&transaction, Duration::from_secs(1)).unwrap() {
            completed = true;
            break;
        }
    }
    assert!(completed, "assert_fresh_backfill exceeded 4096 slices: {context}");
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
                    0 => { connection.execute(/* sqlite-large-value: allow-for-now lv-hfb3ab4e9ffc0 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                        params![proof.writer,proof.sequence,proof.hash,proof.member_key]).unwrap(); }
                    1 => { connection.execute(/* sqlite-large-value: allow-for-now lv-hf715aec3d234 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key=?5 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                        params![proof.writer,proof.sequence,proof.hash,proof.member_key,format!("untrusted-{seed}-{index}")]).unwrap(); }
                    2 => { connection.execute(/* sqlite-large-value: allow-for-now lv-h8a99bdf3afc3 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET origin=?2 WHERE id=?1", params![batch.0,format!("unrelated-{seed}-{index}")]).unwrap(); }
                    _ => { connection.execute(/* sqlite-large-value: allow-for-now lv-h9cad1e7a0461 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=?2 WHERE id=?1", params![batch.0,batch.2+10000]).unwrap(); }
                }
            }
            assert_windows(&target, &target, &format!("seed={seed} mutation={index} kind={mutation}"));
            {
                let connection = target.connection.write();
                if mutation == 1 {
                    connection.execute(/* sqlite-large-value: allow-for-now lv-h55ce404e22db owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures WHERE member_key=?1", [format!("untrusted-{seed}-{index}")]).unwrap();
                }
                if mutation < 2 { insert_raw_signature(&connection, proof); }
                else {
                    connection.execute(/* sqlite-large-value: allow-for-now lv-h6007bc2d61c3 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET origin=?2,replica_sequence=?3 WHERE id=?1", params![batch.0,batch.1,batch.2]).unwrap();
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
            target.connection.write().execute(/* sqlite-large-value: allow-for-now lv-he4d6e9e7421e owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&witness.id]).unwrap();
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
        // A queued retraction alone is not a fence while the recorded proof
        // remains covered. Learn a real removal instead: only the transition
        // lies beyond its high-water, while the base grant keeps coverage.
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
                /* sqlite-large-value: allow-for-now lv-hf625193e6fe4 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures
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
        let mut completed = false;
        for _ in 0..4096 {
            let complete = target.advance_assistant_grant_authority_backfill().unwrap();
            assert_established_grants_ready(&target, &grants);
            if complete { completed = true; break; }
        }
        assert!(completed,
            "repeated_future_floor_window_additions_do_not_reauthenticate_or_fence_established_history exceeded 4096 slices at admission {index}");
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
    let mut completed = false;
    for _ in 0..4096 {
        let complete = target.advance_assistant_grant_authority_backfill().unwrap();
        assert_established_grants_ready(&target, std::slice::from_ref(&retained_grant));
        if complete { completed = true; break; }
    }
    assert!(completed,
        "membership_retraction_reauthenticates_only_rows_using_the_lost_recorded_key exceeded 4096 slices");
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
        let original_windows = windows(&fleet_membership_tx(&target.readers.get()).unwrap());
        let original_key = ("peer".to_owned(), peer_key.public().to_owned());
        let original_window = *original_windows.get(&original_key).unwrap();
        // writer_floor is the last excluded sequence, not the window start.
        assert!(sequence >= original_window.0
            && original_window.1.is_none_or(|end| sequence <= end));
        {
            let mut writer = target.connection.write();
            let connection = writer.transaction().unwrap();
            let query = match mutation {
                "signature-delete" =>
                    /* sqlite-large-value: allow-for-now lv-hbb5e6b92e1b0 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                "signature-update" =>
                    /* sqlite-large-value: allow-for-now lv-hbc780a1c9dac owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET signature='invalid'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                "record-invalid" | "record-invalid-refresh" =>
                    /* sqlite-large-value: allow-for-now lv-h054ba51f20f6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "record-repaired-refresh" =>
                    /* sqlite-large-value: allow-for-now lv-hc7948a0a84cd owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired'
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "record-delete" =>
                    /* sqlite-large-value: allow-for-now lv-ha76a98dcc427 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4",
                "envelope-invalid" =>
                    /* sqlite-large-value: allow-for-now lv-h6891eac9f9c6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state=?4
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                "envelope-delete" =>
                    /* sqlite-large-value: allow-for-now lv-hf168a0847ffd owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND ?4 IS NOT NULL",
                "claim-delete" =>
                    /* sqlite-large-value: allow-for-now lv-h1a8e7267f37b owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM claims WHERE id IN(
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
            assert_eq!(connection.execute(/* sqlite-large-value: allow-for-now lv-h565adf25d451 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=trace-statement-and-value-representation */ query, params![grant.origin,sequence,hash,last]).unwrap(), 1);
            if mutation.ends_with("-refresh") {
                // Prepared-envelope revalidation changes evidence and invokes
                // its non-verifying hook in the same writer transaction.
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-hf26931e7ebc2 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&grant.id],
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
        if matches!(mutation, "envelope-invalid" | "envelope-delete") {
            let mut writer = target.connection.write();
            let connection = writer.transaction().unwrap();
            let receipt_state: String = connection.query_row(
                "SELECT receipt_state FROM replica_envelopes
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                params![grant.origin,sequence,hash], |row| row.get(0),
            ).unwrap();
            assert!(matches!(receipt_state.as_str(), "validated" | "degraded"),
                "real receive must repair the pending receipt with its still-valid signature");
            // Backlog validation legitimately restores a pending or deleted
            // receipt during real admission receive. Renew the same withdrawal
            // after repair so maintenance still sees withdrawn evidence.
            let queued_before: i64 = connection.query_row(
                "SELECT COUNT(*) FROM assistant_grant_proof_invalidations
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=''",
                params![grant.origin,sequence,hash], |row| row.get(0),
            ).unwrap();
            let withdrawal = if mutation == "envelope-delete" {
                /* sqlite-large-value: allow-for-now lv-haa8b386b739d owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3"
            } else {
                /* sqlite-large-value: allow-for-now lv-h731a7b88489d owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state='pending'
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3"
            };
            assert_eq!(connection.execute(
                /* sqlite-large-value: allow-for-now lv-h80511dab4ee6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=trace-statement-and-value-representation */ withdrawal,
                params![grant.origin,sequence,hash],
            ).unwrap(), 1);
            let queued_after: i64 = connection.query_row(
                "SELECT COUNT(*) FROM assistant_grant_proof_invalidations
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=''",
                params![grant.origin,sequence,hash], |row| row.get(0),
            ).unwrap();
            assert_eq!(queued_after, queued_before+1,
                "renewed withdrawal must enqueue its own bounded verification obligation");
            assert!(authority::subject_ready(&connection, &grant.subject).is_err());
            assert!(!authority::contains(&connection, &grant.id).unwrap(),
                "renewed receipt withdrawal must immediately fence bearer authority");
            connection.commit().unwrap();
        }
        let expected = windows(&fleet_membership_tx(&target.readers.get()).unwrap());
        assert_eq!(expected.get(&original_key), Some(&original_window));
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
        /* sqlite-large-value: allow-for-now lv-h187f367a46c6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures
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

#[derive(Clone, Copy, Debug)]
enum AlternateProofCoverage {
    MidPage,
    AlreadyAdmitted,
    AlreadyAdmittedOneUnit,
    AfterDemotion,
    FutureOnly,
}

// Use only real admitted keys and cryptographically valid signatures. The
// alternate proof predates the cursor, so membership is the only late evidence.
fn admitted_recheck_finds_alternate_proof(seed: u64, coverage: AlternateProofCoverage) {
    let add_coverage = !matches!(coverage,
        AlternateProofCoverage::AlreadyAdmitted | AlternateProofCoverage::AlreadyAdmittedOneUnit);
    let key_pages = if matches!(coverage, AlternateProofCoverage::FutureOnly) { 4 } else { 1 };
    let (anchor, anchor_key) = source();
    let mut keys: Vec<_> = (0..authority::MEMBERSHIP_KEYS_PER_REFRESH * key_pages + 2)
        .map(|_| Arc::new(MemberKey::generate().unwrap().0)).collect();
    keys.sort_by(|left, right| left.public().cmp(right.public()));
    let low = keys.remove(0);
    let high = keys.pop().unwrap();
    let (original, alternate) = if add_coverage { (high, low) } else { (low, high) };
    let peer = node("peer", Some(anchor_key.public()));
    peer.set_member_key(Some(original.clone())).unwrap();
    let target = node("reader", Some(anchor_key.public()));
    let mut admitted = keys;
    admitted.push(original.clone());
    if !add_coverage { admitted.push(alternate.clone()); }
    let mut state = seed;
    shuffle(&mut admitted, &mut state);
    for key in admitted {
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
    }
    receive(&peer, &export(&anchor, &peer));
    receive(&target, &export(&anchor, &target));
    let grant = append(&peer, SUBJECT, GRANTED, fields());
    let granted = export(&peer, &target);
    receive(&target, &granted);
    assert_windows(&target, &target, "original proof established");
    let recorded_key: String = target.readers.get().query_row(
        "SELECT auth_member_key FROM assistant_grant_pending WHERE claim_id=?1",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    assert_eq!(recorded_key, original.public());
    let mut alternate_proof = signatures(&granted).signatures.into_iter().find(|proof| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records
             WHERE claim_id=?1 AND writer=?2 AND sequence=?3 AND envelope_hash=?4)",
            params![grant.id,proof.writer,proof.sequence,proof.hash],
            |row| row.get::<_,bool>(0),
        ).unwrap()
    }).unwrap();
    alternate_proof.member_key = alternate.public().to_owned();
    alternate_proof.signature = alternate.sign(&crate::fleet::envelope_signature_message_in(
        None, FLEET, &alternate_proof.writer, alternate_proof.sequence, &alternate_proof.hash,
    ));
    insert_raw_signature(&target.connection.write(), &alternate_proof);
    append(&anchor, "host/peer", "fleet.member-removed", json!({
        "member_key": original.public(), "high_water": 0, "reason": "alternate proof regression"
    }));
    receive_without_authority_maintenance(&target, &export(&anchor, &target));
    // A production slice may revisit this claim and finish its key search.
    // Stop real Store maintenance at the first durable mid-page boundary instead.
    let mut mid_page = false;
    for _ in 0..authority::CLAIMS_PER_SLICE {
        target.advance_assistant_grant_authority_backfill_for_test(Duration::ZERO, 1).unwrap();
        mid_page = target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)
             AND EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)
             AND EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
            [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap();
        if mid_page { break; }
    }
    assert!(mid_page, "fixture did not reach its bounded durable mid-page recheck");
    {
        let connection = target.readers.get();
        let (authenticated, key): (bool, String) = connection.query_row(
            "SELECT pending.authenticated,cursor.member_key
             FROM assistant_grant_pending AS pending
             JOIN assistant_grant_auth_cursor AS cursor USING(claim_id)
             WHERE pending.claim_id=?1",
            [&grant.id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert!(authenticated, "fixture must retry an already admitted row");
        if add_coverage {
            assert!(key.as_str() > alternate.public(), "new key must be below the saved cursor");
        }
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
            [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap(), "mid-page retry lost its durable obligation");
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)",
            [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap(), "fixture must dispatch a dirty claim also owned by its recheck");
        assert!(authority::subject_ready(&connection, SUBJECT).is_err());
        assert!(!authority::contains(&connection, &grant.id).unwrap());
    }
    if matches!(coverage, AlternateProofCoverage::FutureOnly) {
        let cursor = || target.readers.get().query_row(
            "SELECT position,record_rowid,membership_start,member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
            [&grant.id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?)),
        ).optional().unwrap();
        let mut previous = cursor();
        let mut completed_at = None;
        // Keep signed admissions arriving for substantially longer than the four
        // original key pages. None covers an existing receipt, so none may
        // restart that search or leave a durable addition task behind.
        for unit in 0..16 {
            let future_key = MemberKey::generate().unwrap().0;
            let floor = 1_000_000 + unit as u64;
            let highest: u64 = target.readers.get().query_row(
                "SELECT MAX(sequence) FROM replica_records WHERE writer='peer'",
                [], |row| row.get(0),
            ).unwrap();
            assert!(floor > highest);
            append(&anchor, "host/peer", "fleet.member-admitted", json!({
                "member_key": future_key.public(), "via": "member", "sponsor": "anchor",
                "writer_floor": floor, "mode": "listening"
            }));
            receive_without_authority_maintenance(&target, &export(&anchor, &target));
            assert_eq!(cursor(), previous, "receiving membership restarted the recheck");
            target.advance_assistant_grant_authority_backfill().unwrap();
            let next = cursor();
            let connection = target.readers.get();
            let queued: u64 = connection.query_row(
                "SELECT COUNT(*) FROM assistant_grant_additions WHERE writer='peer'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(queued, 0, "future-only admission queued historical retry work");
            assert!(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_membership_windows
                 WHERE writer='peer' AND member_key=?1 AND start=?2)",
                params![future_key.public(),floor + 1], |row| row.get::<_,bool>(0),
            ).unwrap(), "signed admission must actually reach membership");
            if let (Some(before), Some(after)) = (&previous, &next) {
                assert!(after > before, "future-only admission restarted or stalled the original cursor");
            }
            let rechecking: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
                [&grant.id], |row| row.get(0),
            ).unwrap();
            if !rechecking {
                completed_at.get_or_insert(unit);
                assert!(next.is_none(), "completed recheck retained its original cursor");
                let authenticated: bool = connection.query_row(
                    "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
                    [&grant.id], |row| row.get(0),
                ).unwrap();
                assert!(!authenticated, "uncovered alternate signature must fail closed");
                assert!(!authority::contains(&connection, &grant.id).unwrap());
            } else {
                assert!(completed_at.is_none(), "later future-only admission recreated a finished recheck");
            }
            previous = next;
        }
        assert!(completed_at.is_some_and(|unit| unit <= key_pages + 2),
            "sustained future-only admissions prevented bounded recheck completion");
        assert_windows(&target, &target, &format!("seed={seed} future-only recheck"));
        return;
    }
    if matches!(coverage, AlternateProofCoverage::AfterDemotion) {
        // The valid earlier-key signature is already in raw provenance. Exhaust
        // the old search before changing membership: restoration must come from
        // the new key/range task, not an original cursor that still happens to run.
        let mut demoted = false;
        for _ in 0..key_pages + 3 {
            target.advance_assistant_grant_authority_backfill().unwrap();
            let connection = target.readers.get();
            let authenticated: bool = connection.query_row(
                "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
                [&grant.id], |row| row.get(0),
            ).unwrap();
            if !authenticated {
                assert!(!connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)
                     OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
                    [&grant.id], |row| row.get::<_,bool>(0),
                ).unwrap(), "demotion must finish the original retry obligation");
                assert!(!authority::contains(&connection, &grant.id).unwrap());
                demoted = true;
                break;
            }
        }
        assert!(demoted, "original recheck did not demote within its bounded key pages");
    }
    if add_coverage {
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": alternate.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
        receive_without_authority_maintenance(&target, &export(&anchor, &target));
    }
    if matches!(coverage, AlternateProofCoverage::AlreadyAdmitted | AlternateProofCoverage::AlreadyAdmittedOneUnit) {
        // A dirty refresh and the durable recheck must share one verifier.
        // Bound completion here so cursor restarts fail before the oracle's drain.
        let mut finished = false;
        for _ in 0..key_pages + 3 {
            let done = if matches!(coverage, AlternateProofCoverage::AlreadyAdmittedOneUnit) {
                target.advance_assistant_grant_authority_backfill_for_test(Duration::ZERO, 1).unwrap()
            } else {
                target.advance_assistant_grant_authority_backfill().unwrap()
            };
            if done {
                finished = true;
                break;
            }
        }
        assert!(finished, "dual-signed recheck did not finish within its bounded key pages");
        assert!(!target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)
             OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
            [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap(), "completed dual-signed maintenance retained its original retry obligation");
    }
    // Newly eligible earlier keys have independent retry work; the original
    // search need not restart or survive until that work restores authority.
    assert_windows(&target, &target, &format!("seed={seed} coverage={coverage:?}"));
    let recorded_key: String = target.readers.get().query_row(
        "SELECT auth_member_key FROM assistant_grant_pending WHERE claim_id=?1",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    assert_eq!(recorded_key, alternate.public());
    assert_established_grants_ready(&target, std::slice::from_ref(&grant));
}

#[test]
fn newly_admitted_key_below_mid_page_cursor_reauthenticates_already_admitted_row() {
    for seed in 1..=4 {
        admitted_recheck_finds_alternate_proof(seed, AlternateProofCoverage::MidPage);
    }
}

#[test]
fn same_row_dual_signed_proof_survives_recorded_key_removal_after_mid_page_recheck() {
    for seed in 1..=4 {
        admitted_recheck_finds_alternate_proof(seed, AlternateProofCoverage::AlreadyAdmitted);
    }
}

#[test]
fn dirty_owned_recheck_completes_with_one_unit_zero_duration_slices() {
    for seed in 1..=4 {
        admitted_recheck_finds_alternate_proof(seed,AlternateProofCoverage::AlreadyAdmittedOneUnit);
    }
}

#[test]
fn sustained_future_only_admissions_do_not_restart_mid_page_admitted_recheck() {
    for seed in 1..=4 {
        admitted_recheck_finds_alternate_proof(seed, AlternateProofCoverage::FutureOnly);
    }
}

#[test]
fn newly_admitted_earlier_key_restores_demoted_row_from_existing_signature() {
    for seed in 1..=4 {
        admitted_recheck_finds_alternate_proof(seed, AlternateProofCoverage::AfterDemotion);
    }
}

// Real past-covering admissions must visit the small set of grant rows needing
// authentication, not replay this writer's unrelated (or established) history.
fn past_covering_addition_backlog(unrelated: usize) -> (Store, Vec<ClaimRecord>, Vec<ClaimRecord>, usize) {
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
    for first in (0..unrelated).step_by(32) {
        for index in first..(first + 32).min(unrelated) {
            append(&peer, &format!("custom/unrelated/{index}"), "custom.unrelated.event",
                json!({"index": index}));
        }
        receive(&target, &export(&peer, &target));
    }
    let established: Vec<_> = (0..3).map(|index| {
        let suffix = format!("{:024x}", index + 100);
        append(&peer, &format!("custom/client/assistant-v1-{suffix}"), GRANTED,
            fixture_fields(&suffix, "assistant", 'a'))
    }).collect();
    receive(&target, &export(&peer, &target));
    let needy: Vec<_> = (0..2).map(|index| {
        let suffix = format!("{:024x}", index + 200);
        append(&peer, &format!("custom/client/assistant-v1-{suffix}"), GRANTED,
            fixture_fields(&suffix, "assistant", 'b'))
    }).collect();
    receive_pre_projection_unsigned_history(&target, &export(&peer, &target));
    assert_established_grants_ready(&target, &established);
    for grant in &needy {
        assert_bearer_rejected(&target, grant);
    }
    let receipts: usize = target.readers.get().query_row(
        "SELECT COUNT(*) FROM replica_records WHERE writer='peer'",
        [], |row| row.get(0),
    ).unwrap();
    assert!(receipts >= unrelated + established.len() + needy.len());
    let additions = 24;
    for _ in 0..additions {
        let key = MemberKey::generate().unwrap().0;
        append(&anchor, "host/peer", "fleet.member-admitted", json!({
            "member_key": key.public(), "via": "member", "sponsor": "anchor",
            "writer_floor": 0, "mode": "listening"
        }));
    }
    receive_without_authority_maintenance(&target, &export(&anchor, &target));
    (target, established, needy, additions)
}

#[test]
fn many_past_covering_additions_visit_needy_grants_not_unrelated_receipts() {
    for unrelated in [64, 512] {
        let (target, established, needy, additions) = past_covering_addition_backlog(unrelated);
        target.connection.write().execute_batch(
            /* sqlite-large-value: allow-for-now lv-h8d3c6e9a6220 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE addition_work (unit INTEGER);
             CREATE TEMP TRIGGER count_addition_updates AFTER UPDATE ON assistant_grant_additions
             BEGIN INSERT INTO addition_work VALUES(1); END;
             CREATE TEMP TRIGGER count_addition_completion AFTER DELETE ON assistant_grant_additions
             BEGIN INSERT INTO addition_work VALUES(1); END;",
        ).unwrap();
        authority::take_authentication_attempts();
        authority::take_authentication_claims();
        // Apply only the real membership delta, then use one-unit transactions
        // so no unmeasured production slice can drain the work under test.
        target.persist_assistant_grant_membership_for_test().unwrap();
        let bound = additions * (needy.len() * 4 + 2) + 8;
        let mut completed = false;
        for _ in 0..bound {
            let mut connection = target.connection.write();
            let transaction = connection.transaction().unwrap();
            let done = authority::backfill_slice(&transaction, Duration::ZERO).unwrap();
            transaction.commit().unwrap();
            drop(connection);
            assert_established_grants_ready(&target, &established);
            if done {
                completed = true;
                break;
            }
        }
        assert!(completed,
            "{unrelated} unrelated receipts exhausted a grant-row-proportional work bound");
        let work: usize = target.connection.write().query_row(
            "SELECT COUNT(*) FROM addition_work", [], |row| row.get(0),
        ).unwrap();
        assert!(work >= additions, "fixture did not exercise every real admission");
        assert!(work <= bound,
            "addition work scaled with {unrelated} raw receipts rather than {} needy rows: {work}",
            needy.len());
        assert_eq!(target.readers.get().query_row(
            "SELECT COUNT(*) FROM assistant_grant_additions", [], |row| row.get::<_,usize>(0),
        ).unwrap(), 0);
        let refreshed = authority::take_authentication_claims();
        assert!(refreshed.iter().all(|id| needy.iter().any(|claim| &claim.id == id)),
            "past-covering additions reauthenticated established rows: {refreshed:?}");
        for grant in &needy {
            assert_bearer_rejected(&target, grant);
        }
        assert_windows(&target, &target, "past-covering additions over unrelated history");
    }
}

#[test]
fn ordinary_dirty_subject_is_not_starved_by_past_covering_addition_backlog() {
    let (target, established, _, _) = past_covering_addition_backlog(256);
    // Establish real membership without allowing a production slice to drain
    // the addition backlog before ordinary dirty work joins the scheduler.
    target.persist_assistant_grant_membership_for_test().unwrap();
    let grant = &established[0];
    let mut connection = target.connection.write();
    let queued: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_additions", [], |row| row.get(0),
    ).unwrap();
    assert!(queued > 4, "fixture needs a sustained addition backlog");
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h2483f627e2e6 owner=smallclaims::store::assistant_grant_membership_property_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&grant.id],
    ).unwrap();
    assert!(authority::subject_ready(&connection, &grant.subject).is_err());
    authority::take_authentication_claims();
    let mut serviced = false;
    for _ in 0..4 {
        let transaction = connection.transaction().unwrap();
        assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
        transaction.commit().unwrap();
        let dirty: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)",
            [&grant.id], |row| row.get(0),
        ).unwrap();
        if !dirty {
            serviced = true;
            break;
        }
    }
    assert!(serviced, "ordinary dirty subject starved behind addition tasks");
    assert!(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_additions)",
        [], |row| row.get::<_,bool>(0),
    ).unwrap(), "dirty work only progressed after the addition backlog drained");
    assert!(authority::take_authentication_claims().contains(&grant.id));
    assert!(authority::subject_ready(&connection, &grant.subject).is_ok());
    assert!(authority::contains(&connection, &grant.id).unwrap());
    drop(connection);
    assert_windows(&target, &target, "dirty subject serviced during addition backlog");
}
