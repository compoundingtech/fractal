// Included by assistant_grant_authority_tests; uses its real local/replication fixtures.

#[derive(Debug, Default)]
struct AttachmentSliceWork {
    candidates: usize,
    completions: usize,
    witness_cleanups: usize,
    pair_inserts: usize,
    pair_updates: usize,
    pair_deletes: usize,
}

fn attachment_install_counter(connection: &Connection) {
    connection.execute_batch(
        "CREATE TEMP TABLE attachment_slice_work (
             candidates INTEGER NOT NULL, completions INTEGER NOT NULL,
             pair_inserts INTEGER NOT NULL, pair_updates INTEGER NOT NULL,
             pair_deletes INTEGER NOT NULL, witness_cleanups INTEGER NOT NULL);
         INSERT INTO attachment_slice_work VALUES(0,0,0,0,0,0);
         CREATE TEMP TRIGGER attachment_candidate_work
         AFTER UPDATE OF after_claim_id ON main.assistant_grant_attachments
         WHEN NEW.after_claim_id<>OLD.after_claim_id BEGIN
             SELECT CASE WHEN NEW.after_claim_id<=OLD.after_claim_id
                 THEN RAISE(ABORT,'attachment candidate seek moved backwards') END;
             UPDATE attachment_slice_work SET candidates=candidates+1;
         END;
         CREATE TEMP TRIGGER attachment_completion_work
         AFTER DELETE ON main.assistant_grant_attachments BEGIN
             UPDATE attachment_slice_work SET completions=completions+1;
         END;
         CREATE TEMP TRIGGER attachment_witness_cleanup_work
         AFTER DELETE ON main.assistant_grant_attachment_witnesses BEGIN
             UPDATE attachment_slice_work SET witness_cleanups=witness_cleanups+1;
         END;
         CREATE TEMP TRIGGER attachment_pair_insert_work
         AFTER INSERT ON main.assistant_grant_authority
         WHEN NEW.claim_id<>NEW.base_claim_id BEGIN
             UPDATE attachment_slice_work SET pair_inserts=pair_inserts+1;
         END;
         CREATE TEMP TRIGGER attachment_pair_update_work
         AFTER UPDATE ON main.assistant_grant_authority
         WHEN NEW.claim_id<>NEW.base_claim_id BEGIN
             UPDATE attachment_slice_work SET pair_updates=pair_updates+1;
         END;
         CREATE TEMP TRIGGER attachment_pair_delete_work
         AFTER DELETE ON main.assistant_grant_authority
         WHEN OLD.claim_id<>OLD.base_claim_id BEGIN
             UPDATE attachment_slice_work SET pair_deletes=pair_deletes+1;
         END;",
    ).unwrap();
}

fn attachment_tasks(connection: &Connection) -> usize {
    connection.query_row("SELECT COUNT(*) FROM assistant_grant_attachments", [], |row| row.get(0)).unwrap()
}

fn attachment_connection_slice(connection: &mut Connection, budget: Duration) -> (bool, AttachmentSliceWork) {
    // Enqueue INSERTs belong to authentication, not candidate work. Self-pairs
    // and effective-row writes materialize the same candidate, so do not count
    // them twice. Retiring a saved conflict witness is a separate shared unit.
    connection.execute(
        "UPDATE attachment_slice_work SET candidates=0,completions=0,
         pair_inserts=0,pair_updates=0,pair_deletes=0,witness_cleanups=0", [],
    ).unwrap();
    let transaction = connection.transaction().unwrap();
    let done = authority::backfill_slice(&transaction, budget).unwrap();
    let work = transaction.query_row(
        "SELECT candidates,completions,pair_inserts,pair_updates,pair_deletes,witness_cleanups FROM attachment_slice_work",
        [], |row| Ok(AttachmentSliceWork {
            candidates: row.get(0)?, completions: row.get(1)?,
            pair_inserts: row.get(2)?, pair_updates: row.get(3)?, pair_deletes: row.get(4)?,
            witness_cleanups: row.get(5)?,
        }),
    ).unwrap();
    let limit = if budget.is_zero() { 1 } else { authority::CLAIMS_PER_SLICE };
    assert!(work.candidates+work.completions+work.witness_cleanups<=limit,
        "attachment slice exceeded {limit} units: {work:?}");
    assert!(work.pair_inserts+work.pair_updates+work.pair_deletes<=limit,
        "attachment pair mutation fanout exceeded {limit}: {work:?}");
    transaction.commit().unwrap();
    (done, work)
}

fn attachment_slice(store: &Store, budget: Duration) -> (bool, AttachmentSliceWork) {
    attachment_connection_slice(&mut store.connection.write(), budget)
}

fn attachment_assert_fenced(store: &Store, grants: &[ClaimRecord], old_witnesses: usize) {
    let connection = store.readers.get();
    let tasks = attachment_tasks(&connection);
    let witnesses: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_attachment_witnesses", [], |row| row.get(0),
    ).unwrap();
    if tasks+witnesses==0 { return; }
    assert!(authority::subject_ready(&connection, SUBJECT).is_err());
    for grant in grants {
        assert!(!authority::contains(&connection, &grant.id).unwrap(),
            "queued attachments exposed {}", grant.id);
    }
    let retained: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_effective WHERE grant_subject=?1", [SUBJECT], |row| row.get(0),
    ).unwrap();
    assert!(retained>=old_witnesses, "attachment fencing discarded an established conflict witness");
    if tasks>0 && old_witnesses>0 {
        let candidates = authority::candidates(
            &connection, "assistant/chatgpt", OWNER, Some(9999), None, None,
        ).unwrap();
        for grant in grants {
            assert!(candidates.iter().any(|candidate| candidate.id==grant.id),
                "partial policy/revoke attachment lost baseline conflict witness {}", grant.id);
        }
    }
}

fn attachment_append_group(store: &Store, bases: bool, count: usize) -> Vec<ClaimRecord> {
    (0..count).map(|index| {
        let mut value = fields();
        value["expires_at_unix_ms"] = json!(10000+index);
        let kind = if bases { GRANTED } else if index%2==0 { POLICY } else { REVOKED };
        append(store, SUBJECT, kind, value)
    }).collect()
}

fn attachment_budget_product(remote: bool, bases_first: bool, budget: Duration) {
    const BASES: usize = 9;
    const TRANSITIONS: usize = 19;
    let (issuer, key) = source();
    let target = node("attachment-reader", Some(key.public()));
    let store = if remote { &target } else { &issuer };
    let first = attachment_append_group(&issuer, bases_first, if bases_first { BASES } else { TRANSITIONS });
    drain_authority(&issuer);
    if remote { receive(&target, &export(&issuer, &target)); }
    let old_witnesses = if bases_first { BASES } else { 0 };
    attachment_install_counter(&store.connection.write());
    let second = attachment_append_group(&issuer, !bases_first, if bases_first { TRANSITIONS } else { BASES });
    if remote {
        let exchange = export(&issuer, &target);
        // Admit raw claims before their proofs. No signed maintenance work is
        // hidden inside receive(), which intentionally drains its caller's work.
        receive_pre_projection_unsigned_history(&target, &exchange);
        let connection = target.connection.write();
        for signature in signatures(&exchange).signatures {
            insert_raw_signature(&connection, &signature);
        }
    }
    let grants = if bases_first { &first } else { &second };
    if !remote {
        assert!(attachment_tasks(&store.readers.get())>0, "fanout was performed synchronously");
        attachment_assert_fenced(store, grants, old_witnesses);
    }
    let mut slices = 0;
    let mut pairs = 0;
    let mut candidates = 0;
    let mut completions = 0;
    // Includes authentication/cursor bookkeeping as well as every possible pair
    // and partition completion; ZERO must still terminate without wall-clock aid.
    let finite_bound = 32*(BASES+TRANSITIONS+BASES*TRANSITIONS+1);
    loop {
        attachment_assert_fenced(store, grants, old_witnesses);
        let (done, work) = attachment_slice(store, budget);
        slices += 1;
        pairs += work.pair_inserts;
        candidates += work.candidates;
        completions += work.completions;
        assert!(slices<=finite_bound, "one-unit attachment maintenance failed to finish");
        if done { break; }
    }
    assert_eq!(pairs, BASES*TRANSITIONS, "every compatible base/transition pair must attach exactly once");
    assert!(candidates>=BASES*TRANSITIONS, "pair writes escaped candidate accounting");
    assert!(completions>0, "partition completion must consume a unit");
    let per_slice = if budget.is_zero() { 1 } else { authority::CLAIMS_PER_SLICE };
    assert!(slices>=(BASES*TRANSITIONS).div_ceil(per_slice), "Cartesian attachment work escaped the slice budget");
    let connection = store.readers.get();
    assert_eq!(attachment_tasks(&connection), 0);
    authority::subject_ready(&connection, SUBJECT).unwrap();
    let actual: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_authority WHERE claim_id<>base_claim_id", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(actual, BASES*TRANSITIONS);
    drop(connection);
    assert_reference_and_rebuild(store);
}

#[test]
fn local_attachment_cartesian_product_shares_the_full_slice_budget_in_both_directions() {
    for bases_first in [true, false] {
        attachment_budget_product(false, bases_first, Duration::from_secs(60));
    }
}

#[test]
fn verified_attachment_cartesian_product_shares_the_full_slice_budget_in_both_directions() {
    for bases_first in [true, false] {
        attachment_budget_product(true, bases_first, Duration::from_secs(60));
    }
}

#[test]
fn local_attachment_zero_duration_is_one_unit_and_finishes_in_both_directions() {
    for bases_first in [true, false] {
        attachment_budget_product(false, bases_first, Duration::ZERO);
    }
}

#[test]
fn verified_attachment_zero_duration_is_one_unit_and_finishes_in_both_directions() {
    for bases_first in [true, false] {
        attachment_budget_product(true, bases_first, Duration::ZERO);
    }
}

#[test]
fn attachment_cursor_and_conflict_witness_survive_file_reopen_and_zero_duration_completion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attachment-resume.sqlite3");
    let grants;
    let saved_cursor;
    let expected;
    {
        let store = Store::open(&path, "attachment-issuer", Arc::new(Plain)).unwrap();
        grants = attachment_append_group(&store, true, 11);
        drain_authority(&store);
        attachment_install_counter(&store.connection.write());
        attachment_append_group(&store, false, 13);
        expected = reference(&store);
        // Stop after actual pair work, not merely an authentication checkpoint.
        for attempt in 0..100 {
            let (_, work) = attachment_slice(&store, Duration::ZERO);
            if work.candidates>0 { break; }
            assert!(attempt<99, "attachment cursor never advanced");
        }
        attachment_assert_fenced(&store, &grants, grants.len());
        let connection = store.readers.get();
        saved_cursor = connection.query_row(
            "SELECT claim_id,candidate_kind,after_claim_id,upper_claim_id
             FROM assistant_grant_attachments WHERE after_claim_id<>'' ORDER BY claim_id,candidate_kind LIMIT 1",
            [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,
                row.get::<_,String>(2)?,row.get::<_,String>(3)?)),
        ).unwrap();
        assert!(attachment_tasks(&connection)>0);
    }
    // Open the durable database directly: Store startup is allowed to advance
    // maintenance and would otherwise conceal whether the saved seek survived.
    {
        let mut connection = Connection::open(&path).unwrap();
        let durable: (String,String) = connection.query_row(
            "SELECT after_claim_id,upper_claim_id FROM assistant_grant_attachments
             WHERE claim_id=?1 AND candidate_kind=?2", params![saved_cursor.0,saved_cursor.1],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(durable, (saved_cursor.2.clone(),saved_cursor.3.clone()));
        assert!(authority::subject_ready(&connection, SUBJECT).is_err());
        for grant in &grants { assert!(!authority::contains(&connection, &grant.id).unwrap()); }
        attachment_install_counter(&connection);
        let mut complete = false;
        for _ in 0..2048 {
            let (done, _) = attachment_connection_slice(&mut connection, Duration::ZERO);
            if done { complete=true; break; }
        }
        assert!(complete, "durable one-unit attachment continuation did not terminate");
        assert_eq!(attachment_tasks(&connection), 0);
        let witnesses: usize = connection.query_row(
            "SELECT COUNT(*) FROM assistant_grant_attachment_witnesses", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(witnesses, 0);
        authority::subject_ready(&connection, SUBJECT).unwrap();
        let actual: Vec<(String,String,bool)> = connection.prepare(
            "SELECT claim_id,expires_at,revoked FROM assistant_grant_effective ORDER BY claim_id",
        ).unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
            .unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(actual, expected, "resumed attachment state disagrees with raw history");
    }
    let reopened = Store::open(&path, "attachment-issuer", Arc::new(Plain)).unwrap();
    assert_reference_and_rebuild(&reopened);
    authority::subject_ready(&reopened.readers.get(), SUBJECT).unwrap();
    let resumed = snapshot(&reopened.readers.get());
    reset_projection(&reopened.connection.write());
    drain_authority(&reopened);
    assert_eq!(snapshot(&reopened.readers.get()), resumed,
        "restart continuation differs from a fresh authority backfill");
}

#[test]
fn invalid_attachment_candidates_each_consume_a_zero_duration_unit() {
    let store = node("attachment-invalid", None);
    for index in 0..23 {
        let mut incompatible = fields();
        incompatible["profile"] = json!("service");
        incompatible["expires_at_unix_ms"] = json!(10000 + index);
        append(&store, SUBJECT, GRANTED, incompatible);
    }
    drain_authority(&store);
    let baseline = snapshot(&store.readers.get());
    attachment_install_counter(&store.connection.write());
    append(&store, SUBJECT, POLICY, fields());
    let mut candidates = 0;
    let mut completions = 0;
    let mut done = false;
    for _ in 0..128 {
        let (complete, work) = attachment_slice(&store, Duration::ZERO);
        assert_eq!(work.pair_inserts, 0, "incompatible profile attached");
        candidates += work.candidates;
        completions += work.completions;
        if complete { done=true; break; }
    }
    assert!(done);
    assert_eq!(candidates, 23, "invalid candidates escaped shared work accounting");
    assert_eq!(completions, 1, "empty partition completion must be its own unit");
    authority::subject_ready(&store.readers.get(), SUBJECT).unwrap();
    assert_eq!(snapshot(&store.readers.get()), baseline,
        "an incompatible policy changed a base's effective projection");
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    authority::rebuild_subject(&transaction, SUBJECT).unwrap();
    assert_eq!(snapshot(&transaction), baseline);
    transaction.commit().unwrap();
}

#[test]
fn late_opposite_claim_attaches_after_an_older_task_captured_its_high_water() {
    let store = node("attachment-late", None);
    attachment_append_group(&store, true, 7);
    drain_authority(&store);
    let policy = append(&store, SUBJECT, POLICY, fields());
    let high_water: String = store.readers.get().query_row(
        "SELECT upper_claim_id FROM assistant_grant_attachments WHERE claim_id=?1 AND candidate_kind=?2",
        params![policy.id,GRANTED], |row| row.get(0),
    ).unwrap();
    let mut late_fields = fields();
    late_fields["expires_at_unix_ms"] = json!(30000);
    let late = append(&store, SUBJECT, GRANTED, late_fields);
    let unchanged: String = store.readers.get().query_row(
        "SELECT upper_claim_id FROM assistant_grant_attachments WHERE claim_id=?1 AND candidate_kind=?2",
        params![policy.id,GRANTED], |row| row.get(0),
    ).unwrap();
    assert_eq!(unchanged, high_water, "late arrival extended an existing fixed-high-water task");
    attachment_install_counter(&store.connection.write());
    let mut done = false;
    for _ in 0..128 {
        if attachment_slice(&store, Duration::ZERO).0 { done=true; break; }
    }
    assert!(done);
    let attached: bool = store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_authority WHERE claim_id=?1 AND base_claim_id=?2)",
        params![policy.id,late.id], |row| row.get(0),
    ).unwrap();
    assert!(attached, "late base was missed by both sides' attachment tasks");
    assert_reference_and_rebuild(&store);
}

#[test]
fn trimming_an_attachment_owner_and_opposite_mid_task_does_not_restore_stale_pairs() {
    let store = node("attachment-trim", None);
    let grants = attachment_append_group(&store, true, 9);
    drain_authority(&store);
    attachment_install_counter(&store.connection.write());
    let transitions = attachment_append_group(&store, false, 5);
    for _ in 0..100 {
        if attachment_slice(&store, Duration::ZERO).1.candidates>0 { break; }
    }
    // Raw trim is a real demotion path: retire an owning transition and an
    // opposite base while other tasks still hold their fixed candidate bounds.
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for claim in [&transitions[0],&grants[0]] {
            transaction.execute("DELETE FROM claims WHERE id=?1", [&claim.id]).unwrap();
            authority::refresh_claim(&transaction, &claim.id).unwrap();
            let own_tasks: usize = transaction.query_row(
                "SELECT COUNT(*) FROM assistant_grant_attachments WHERE claim_id=?1", [&claim.id], |row| row.get(0),
            ).unwrap();
            assert_eq!(own_tasks, 0, "demotion did not cancel the claim's own attachment tasks");
        }
        transaction.commit().unwrap();
    }
    let mut done = false;
    for _ in 0..1024 {
        if attachment_slice(&store, Duration::ZERO).0 { done=true; break; }
    }
    assert!(done, "stale endpoint work did not terminate");
    let stale: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM assistant_grant_authority
         WHERE claim_id=?1 OR base_claim_id=?2", params![transitions[0].id,grants[0].id], |row| row.get(0),
    ).unwrap();
    assert_eq!(stale, 0, "attachment execution resurrected a demoted endpoint");
    authority::subject_ready(&store.readers.get(), SUBJECT).unwrap();
    assert_reference_and_rebuild(&store);
}

#[test]
fn first_local_base_without_opposite_history_is_immediately_usable_without_attachment_work() {
    let store = node("attachment-immediate", None);
    drain_authority(&store);
    attachment_install_counter(&store.connection.write());
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let connection = store.readers.get();
    assert_eq!(attachment_tasks(&connection), 0);
    authority::subject_ready(&connection, SUBJECT).unwrap();
    assert!(authority::contains(&connection, &grant.id).unwrap());
    let witnesses: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_attachment_witnesses", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(witnesses, 0);
    drop(connection);
    let connection = store.connection.write();
    let charged: usize = connection.query_row(
        "SELECT candidates+completions+pair_inserts+pair_updates+pair_deletes+witness_cleanups
         FROM attachment_slice_work", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(charged, 0, "constant self-materialization was mistaken for pair attachment");
    drop(connection);
    assert_reference_and_rebuild(&store);
}

#[test]
fn completed_subject_cleanup_lifts_fence_despite_unrelated_continuous_attachment_arrivals() {
    let store = node("attachment-cleanup-fairness", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    drain_authority(&store);
    attachment_install_counter(&store.connection.write());
    let mut policy = fields();
    policy["expires_at_unix_ms"] = json!(20000);
    append(&store, SUBJECT, POLICY, policy);
    for attempt in 0..32 {
        if attachment_tasks(&store.readers.get())==0 { break; }
        attachment_slice(&store, Duration::ZERO);
        assert!(attempt<31, "subject A's attachment task did not finish");
    }
    // Stop precisely after task completion, before the separately charged
    // witness cleanup. A is still fenced and has one eligible witness.
    assert!(authority::subject_ready(&store.readers.get(), SUBJECT).is_err());
    let witnesses: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM assistant_grant_attachment_witnesses WHERE grant_subject=?1",
        [SUBJECT], |row| row.get(0),
    ).unwrap();
    assert_eq!(witnesses, 1);

    let other = "custom/client/assistant-v1-fedcba9876543210fedcba98";
    let other_fields = || {
        let mut value = fields();
        value["grant_id"] = json!("assistant-grant/fedcba9876543210fedcba98");
        value["session_actor"] = json!("assistant/chatgpt/session/assistant-grant/fedcba9876543210fedcba98");
        value
    };
    for index in 0..40 {
        let mut value = other_fields();
        value["expires_at_unix_ms"] = json!(30000+index);
        append(&store, other, GRANTED, value);
    }
    for index in 0..40 {
        let mut value = other_fields();
        value["expires_at_unix_ms"] = json!(40000+index);
        append(&store, other, POLICY, value);
    }
    let mut lifted = false;
    for index in 0..8 {
        // Keep growing B's 40-by-40 backlog while A waits for cleanup.
        let mut value = other_fields();
        value["expires_at_unix_ms"] = json!(50000+index);
        append(&store, other, POLICY, value);
        let (_, work) = attachment_slice(&store, Duration::ZERO);
        assert!(work.candidates+work.completions+work.witness_cleanups<=1);
        assert!(attachment_tasks(&store.readers.get())>0,
            "regression requires B's unrelated backlog to remain");
        assert!(authority::subject_ready(&store.readers.get(), other).is_err());
        if authority::subject_ready(&store.readers.get(), SUBJECT).is_ok() {
            assert!(authority::contains(&store.readers.get(), &grant.id).unwrap());
            lifted = true;
            break;
        }
    }
    assert!(lifted, "unrelated continuous arrivals starved completed-subject cleanup");
}

#[test]
fn ineligible_witness_prefix_consumes_bounded_visits_in_zero_and_full_slices() {
    const PREFIX: usize = authority::CLAIMS_PER_SLICE * 3 + 1;
    for budget in [Duration::ZERO, Duration::from_secs(60)] {
        let store = node("attachment-witness-prefix", None);
        let grant = append(&store, SUBJECT, GRANTED, fields());
        drain_authority(&store);
        let mut connection = store.connection.write();
        let expected = snapshot(&connection);
        // Clone a real witness payload only to exercise durable cleanup seeks.
        // Synthetic IDs never enter authenticated pending/authority rows, and
        // the unfinished subject's tasks intentionally have missing owners.
        for index in 0..PREFIX {
            connection.execute(
                "INSERT INTO assistant_grant_attachment_witnesses
                 SELECT ?1,'prefix/unfinished',principal,owner,profile,expires_at,revoked,grant_record
                 FROM assistant_grant_effective WHERE claim_id=?2",
                params![format!("prefix/{index:08}"),grant.id],
            ).unwrap();
        }
        connection.execute(
            "INSERT INTO assistant_grant_attachment_witnesses
             SELECT 'tail/eligible','tail/finished',principal,owner,profile,expires_at,revoked,grant_record
             FROM assistant_grant_effective WHERE claim_id=?1", [&grant.id],
        ).unwrap();
        // More tasks than all visits needed below: attachment turns cannot
        // accidentally make the prefix eligible before the tail is reached.
        for index in 0..(PREFIX * 4) {
            connection.execute(
                "INSERT INTO assistant_grant_attachments
                 VALUES(?1,?2,'prefix/unfinished','missing','missing','','')",
                params![format!("prefix-task/{index:08}"),GRANTED],
            ).unwrap();
        }
        connection.execute_batch(
            "DELETE FROM meta WHERE key IN ('assistant_grant_owner_cursor/witness','assistant_grant_owner_upper/witness');
             CREATE TEMP TABLE witness_visits (id INTEGER PRIMARY KEY, base_claim_id TEXT NOT NULL);
             CREATE TEMP TRIGGER witness_visit_counter AFTER INSERT ON main.meta
             WHEN NEW.key='assistant_grant_owner_cursor/witness' BEGIN
                 INSERT INTO witness_visits(base_claim_id)
                 SELECT owner FROM assistant_grant_queue_owners
                 WHERE queue='witness' AND sequence=CAST(NEW.value AS INTEGER);
             END;",
        ).unwrap();
        let limit = if budget.is_zero() { 1 } else { authority::CLAIMS_PER_SLICE };
        let mut visited = 0;
        let mut finished = false;
        for _ in 0..(8 * (PREFIX + 1)) {
            connection.execute("DELETE FROM witness_visits", []).unwrap();
            let transaction = connection.transaction().unwrap();
            assert!(!authority::backfill_slice(&transaction, budget).unwrap());
            let visits: Vec<String> = transaction.prepare(
                "SELECT base_claim_id FROM witness_visits ORDER BY id",
            ).unwrap().query_map([], |row| row.get(0)).unwrap()
                .collect::<rusqlite::Result<_>>().unwrap();
            assert!(visits.len()<=limit,
                "ineligible prefix escaped the {limit}-visit slice budget: {visits:?}");
            for base in visits {
                if visited<PREFIX {
                    assert_eq!(base, format!("prefix/{visited:08}"),
                        "cleanup searched past an uncharged ineligible witness");
                } else if visited==PREFIX {
                    assert_eq!(base, "tail/eligible");
                } else {
                    // A full slice can wrap after retiring the eligible tail.
                    assert_eq!(base, format!("prefix/{:08}", (visited-PREFIX-1)%PREFIX));
                }
                visited += 1;
            }
            let prefix_left: usize = transaction.query_row(
                "SELECT COUNT(*) FROM assistant_grant_attachment_witnesses
                 WHERE grant_subject='prefix/unfinished'", [], |row| row.get(0),
            ).unwrap();
            assert_eq!(prefix_left, PREFIX, "unfinished witnesses were retired");
            assert!(attachment_tasks(&transaction)>0, "ineligible-prefix fixture ran out of tasks");
            let tail_left: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachment_witnesses
                 WHERE base_claim_id='tail/eligible')", [], |row| row.get(0),
            ).unwrap();
            assert_eq!(tail_left, visited<=PREFIX,
                "eligible tail was reached without charging every preceding witness");
            assert_eq!(snapshot(&transaction), expected, "cleanup fixtures changed authority");
            transaction.commit().unwrap();
            if !tail_left {
                finished = true;
                break;
            }
        }
        assert!(finished, "bounded cleanup never reached the eligible tail");
    }
}

#[test]
fn attachment_owner_turn_survives_reopen_and_lower_and_between_id_arrivals() {
    const BOUND: usize = 256;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attachment-owner-fairness.sqlite3");
    let grant;
    let policy;
    {
        let store = Store::open(&path, "attachment-owner-fairness", Arc::new(Plain)).unwrap();
        grant = append(&store, SUBJECT, GRANTED, fields());
        drain_authority(&store);
        // Only queue fixtures have controlled IDs. The victim and its base are
        // real local appends, with their ordinary content-derived claim IDs.
        store.connection.write().execute(
            "INSERT INTO assistant_grant_attachments
             VALUES('!m/initial',?1,'attacker/subject','missing','missing','','')",
            [GRANTED],
        ).unwrap();
        let mut value = fields();
        value["expires_at_unix_ms"] = json!(20000);
        policy = append(&store, SUBJECT, POLICY, value);
        assert!(authority::subject_ready(&store.readers.get(), SUBJECT).is_err());
    }
    let mut connection = Connection::open(&path).unwrap();
    attachment_install_counter(&connection);
    let mut reopened = false;
    let mut lifted = false;
    for slice in 0..BOUND {
        // New owners arrive both below the already visited attacker and in the
        // hash interval before the victim. They must join a later arrival round,
        // rather than continually extending the victim's current round.
        for prefix in ["!a/lower", "!t/between"] {
            let id = format!("{prefix}/{slice:08}");
            assert!(id.as_str()<policy.id.as_str());
            connection.execute(
                "INSERT INTO assistant_grant_attachments
                 VALUES(?1,?2,'attacker/subject','missing','missing','','')",
                params![id,GRANTED],
            ).unwrap();
        }
        let (_, work) = attachment_connection_slice(&mut connection, Duration::ZERO);
        assert!(attachment_tasks(&connection)>0,
            "victim must finish without draining continuously arriving unrelated owners");
        if !reopened && work.candidates>0 {
            let after: String = connection.query_row(
                "SELECT after_claim_id FROM assistant_grant_attachments
                 WHERE claim_id=?1 AND candidate_kind=?2",
                params![policy.id,GRANTED], |row| row.get(0),
            ).unwrap();
            assert_eq!(after, grant.id);
            assert!(authority::subject_ready(&connection, SUBJECT).is_err());
            // Reopening Store would itself perform maintenance. A raw reopen
            // isolates the durable owner turn/frontier and the unfinished page.
            drop(connection);
            connection = Connection::open(&path).unwrap();
            attachment_install_counter(&connection);
            reopened = true;
        }
        if authority::subject_ready(&connection, SUBJECT).is_ok() {
            assert!(authority::contains(&connection, &grant.id).unwrap());
            assert!(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_authority
                 WHERE claim_id=?1 AND base_claim_id=?2)",
                params![policy.id,grant.id], |row| row.get::<_,bool>(0),
            ).unwrap(), "the victim's actual policy attachment was skipped");
            lifted = true;
            break;
        }
    }
    assert!(reopened, "regression never exercised an unfinished victim across reopen");
    assert!(lifted, "lower and between-ID arrivals starved the attachment victim beyond {BOUND} ZERO slices");
    assert!(!connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE claim_id='!m/initial')",
        [], |row| row.get::<_,bool>(0),
    ).unwrap(), "stale attacker owners must retire normally");
}
