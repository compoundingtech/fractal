//! Tests of `smallclaims::store::checkpoint_agreement` through smalltalk's store.
use super::*;
use smallclaims::store::checkpoint_agreement::*;

const FLEET: &str = "fleet/test";
const CUT: u128 = 20 * DAY_MS;

fn claim(kind: &str, subject: &str, writer: &str, fields: Value) -> CheckpointClaim {
    CheckpointClaim {
        id: format!("{kind}:{subject}:{writer}:{fields}"),
        kind: kind.into(),
        subject: subject.into(),
        writer: writer.into(),
        actor: None,
        fields: fields.as_object().cloned().unwrap_or_default(),
    }
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn seal(writer: &str, participants: &[&str], digest: &str) -> CheckpointClaim {
    claim(
        CHECKPOINT_SEALED,
        &checkpoint_name(CUT),
        writer,
        json!({
            "cut_unix_ms": CUT, "participants": participants, "sealed_digest": digest,
            "sealed_count": 1, "rules_digest": "rules", "checkpoint_protocol": 1,
        }),
    )
}

fn verified(writer: &str, participants: &[&str], drop: &str) -> CheckpointClaim {
    claim(
        CHECKPOINT_VERIFIED,
        &checkpoint_name(CUT),
        writer,
        json!({
            "cut_unix_ms": CUT, "participants": participants, "sealed_digest": "sealed",
            "rules_digest": "rules", "drop_digest": drop, "dropped_envelopes": 1,
            "dropped_claims": 1, "retained_digest": "retained", "graph_digest": "graph",
            "reader_digest": "reader", "checkpoint_protocol": 1,
        }),
    )
}

fn excusal(writer: &str, actor: &str) -> CheckpointClaim {
    CheckpointClaim {
        actor: Some(actor.into()),
        ..claim(
            CHECKPOINT_EXCUSED,
            &format!("checkpoint-excusal/{writer}"),
            "alder",
            json!({"writer": writer, "reason": "gone"}),
        )
    }
}

#[test]
fn participants_are_known_writers_less_leavers_and_excused_writers() {
    let known = names(&["alder", "birch", "cedar", "dogwood"]);
    let left = names(&["dogwood"]);
    // A writer named only by another node's seal is a participant here too.
    let claims = vec![seal("alder", &["alder", "birch", "cedar", "elm"], "d")];
    assert_eq!(
        participants(&known, &left, &claims),
        names(&["alder", "birch", "cedar", "elm"])
    );

    // A person's excusal takes a writer out; an agent's or the system's does nothing.
    let mut claims = claims;
    claims.push(excusal("cedar", "person/operator"));
    claims.push(excusal("birch", "agent/alder.worker"));
    claims.push(CheckpointClaim {
        actor: None,
        ..excusal("elm", "person/operator")
    });
    assert_eq!(
        participants(&known, &left, &claims),
        names(&["alder", "birch", "elm"])
    );
    // The excused writer's own next seal ends its excusal.
    claims.push(seal("cedar", &["alder", "birch", "cedar"], "d"));
    assert_eq!(excused_writers(&claims), BTreeSet::new());
    assert!(participants(&known, &left, &claims).contains("cedar"));
    // A removal is not a leave: a removed writer stays until it is excused.
    assert!(participants(&known, &BTreeSet::new(), &claims).contains("dogwood"));
}

#[test]
fn a_certificate_needs_every_participant_with_identical_terms() {
    let all = ["alder", "birch", "cedar"];
    let complete = all.map(|writer| verified(writer, &all, "drop"));
    let checkpoint = checkpoint_name(CUT);
    assert_eq!(certificates(&complete, &checkpoint).len(), 1);
    let stable = stable_checkpoints(&complete);
    assert_eq!(stable.keys().copied().collect::<Vec<_>>(), [CUT]);

    // One participant missing.
    assert!(certificates(&complete[..2], &checkpoint).is_empty());
    // One digest different.
    let mut different = complete.to_vec();
    different[2] = verified("cedar", &all, "another drop");
    assert!(certificates(&different, &checkpoint).is_empty());
    // The participants disagree.
    let mut disagreeing = complete.to_vec();
    disagreeing[2] = verified("cedar", &["alder", "birch", "cedar", "dogwood"], "drop");
    assert!(certificates(&disagreeing, &checkpoint).is_empty());
    // A second verification from a node is ignored, before or after the certificate.
    let mut second = vec![complete[0].clone(), verified("alder", &all, "other")];
    second.extend_from_slice(&complete[1..]);
    assert_eq!(
        certificates(&second, &checkpoint),
        certificates(&complete, &checkpoint)
    );
    let mut late = complete.to_vec();
    late.push(verified("birch", &all, "other"));
    late.push(seal("alder", &all, "resealed"));
    assert_eq!(
        certificates(&late, &checkpoint),
        certificates(&complete, &checkpoint)
    );
    // A seal is not a verification.
    let seals = all.map(|writer| seal(writer, &all, "sealed"));
    assert!(certificates(&seals, &checkpoint).is_empty());
}

#[test]
fn stability_does_not_depend_on_claim_order() {
    let all = ["alder", "birch", "cedar"];
    let mut claims = all.map(|writer| verified(writer, &all, "drop")).to_vec();
    claims.extend(all.map(|writer| seal(writer, &all, "sealed")));
    claims.push(excusal("dogwood", "person/operator"));
    let expected = stable_checkpoints(&claims);
    for rotation in 0..claims.len() {
        let mut rotated = claims.clone();
        rotated.rotate_left(rotation);
        assert_eq!(stable_checkpoints(&rotated), expected);
        rotated.reverse();
        assert_eq!(stable_checkpoints(&rotated), expected);
    }
}

#[test]
fn both_sides_of_an_excused_partition_can_certify_and_a_node_adopts_both() {
    // People excused each side of a partition. Each side certified the same cut alone.
    let west = ["alder", "birch"];
    let east = ["cedar", "dogwood"];
    let mut claims = west.map(|writer| verified(writer, &west, "west")).to_vec();
    claims.extend(east.map(|writer| verified(writer, &east, "east")));
    let certified = certificates(&claims, &checkpoint_name(CUT));
    assert_eq!(certified.len(), 2);
    claims.reverse();
    assert_eq!(certificates(&claims, &checkpoint_name(CUT)), certified);
    // Every node applies the same one: here the smaller drop digest, since both sides
    // have two participants.
    let chosen = chosen_certificate(&certified).unwrap();
    assert_eq!(chosen.terms.drop_digest, "east");
    let mut reversed = certified.clone();
    reversed.reverse();
    assert_eq!(chosen_certificate(&reversed), Some(chosen));
    // A side with more participants wins whatever its digest.
    let larger = ["cedar", "dogwood", "elm"];
    let mut claims = west.map(|writer| verified(writer, &west, "west")).to_vec();
    claims.extend(larger.map(|writer| verified(writer, &larger, "zzz")));
    let certified = certificates(&claims, &checkpoint_name(CUT));
    assert_eq!(
        chosen_certificate(&certified).unwrap().terms.drop_digest,
        "zzz"
    );
    assert_eq!(chosen_certificate(&[]), None);
}

// Stores exchanging for real.

fn sync(nodes: &[&Store]) {
    for node in nodes {
        node.bind_fleet(FLEET).unwrap();
    }
    for _ in 0..3 {
        for source in nodes {
            for target in nodes {
                if source.origin == target.origin {
                    continue;
                }
                let exchange = source
                    .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
                    .unwrap();
                target
                    .receive_replication_exchange(&source.origin, FLEET, &exchange)
                    .unwrap();
                target.validate_replication_backlog().unwrap();
                target.project_replication_backlog().unwrap();
                target.apply_replication_repairs().unwrap();
            }
        }
    }
}

fn observe(store: &Store, n: usize) {
    for index in 0..n {
        store
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("slow-request")),
                    ("reason".into(), json!(format!("slow {index}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!(
                    "{}-slow-{index}-{}",
                    store.origin,
                    Uuid::now_v7().simple()
                )),
            })
            .unwrap();
    }
}

/// A context two days after tomorrow's cut, so everything written now is before the cut.
fn context(scratch: &Path, days_later: u128) -> CheckpointContext {
    CheckpointContext {
        now_unix_ms: now_ms() + (3 + days_later) * DAY_MS,
        configured_peers: Vec::new(),
        scratch: scratch.to_path_buf(),
        reviewer: "person/operator".into(),
    }
}

fn step(store: &Store, context: &CheckpointContext) -> Vec<CheckpointAction> {
    store.checkpoint_step(context).unwrap()
}

fn kinds(actions: &[CheckpointAction]) -> Vec<&'static str> {
    actions
        .iter()
        .map(|action| match action {
            CheckpointAction::Sealed { .. } => "sealed",
            CheckpointAction::Verified { .. } => "verified",
            CheckpointAction::ProofFailed { .. } => "proof-failed",
            CheckpointAction::AttentionRequested { .. } => "attention",
            CheckpointAction::AttentionWithdrawn { .. } => "withdrawn",
            CheckpointAction::Trimmed { .. } => "trimmed",
            CheckpointAction::ManifestNeeded { .. } => "manifest-needed",
            CheckpointAction::TrimGraphChanged { .. } => "graph-changed",
        })
        .collect()
}

fn stable_cuts(store: &Store) -> Vec<u128> {
    stable_checkpoints(&store.checkpoint_claims().unwrap())
        .into_keys()
        .collect()
}

#[test]
fn nodes_seal_then_verify_once_every_participant_sealed_the_same_set() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 4);
    observe(&birch, 3);
    sync(&[&alder, &birch]);

    assert_eq!(kinds(&step(&alder, &context)), ["sealed"]);
    // Nothing more until birch has sealed the same set.
    assert!(step(&alder, &context).is_empty());
    assert_eq!(kinds(&step(&birch, &context)), ["sealed"]);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &context)), ["verified"]);
    assert!(stable_cuts(&alder).is_empty());
    assert_eq!(kinds(&step(&birch, &context)), ["verified"]);
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(stable_cuts(node), [cut]);
        assert_eq!(kinds(&step(node, &context)), ["trimmed"]);
        assert!(step(node, &context).is_empty());
    }
    let status = alder.checkpoint_status(context.now_unix_ms, &[]).unwrap();
    let stable = status.newest_stable.unwrap();
    assert_eq!(stable.terms.participants, names(&["alder", "birch"]));
    assert!(status.pending.is_none());
    // Seals and verifications are dated at or after the cut, even though the clock was not.
    for claim in alder
        .claims_page(None, None, 0, None, false, 10_000)
        .unwrap()
        .claims
    {
        if claim.kind.starts_with("checkpoint.") {
            assert!(claim.accepted_at_unix_ms >= cut, "{claim:?}");
        } else {
            assert!(claim.accepted_at_unix_ms < cut, "{claim:?}");
        }
    }
}

#[test]
fn verification_refuses_stale_rule_terms_without_consuming_first_verification() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let checkpoint = checkpoint_name(cut);
    let store = Store::open_memory("alder").unwrap();
    observe(&store, 4);
    let sealed = store.checkpoint_sealed_identities(cut, None).unwrap();
    let terms = SealTerms {
        cut_unix_ms: cut, participants: names(&["alder"]),
        sealed_digest: sealed.digest.clone(), rules_digest: "previous-rules".into(),
    };
    store.publish_seal(&checkpoint, &terms, &sealed, None).unwrap();
    let mut actions = Vec::new();
    store.verify_checkpoint(&checkpoint, &terms, &BTreeSet::new(), sealed.seal_rowid, &context, &mut actions).unwrap();
    assert!(actions.is_empty());
    assert!(first_verifications(&store.checkpoint_claims().unwrap(), &checkpoint).is_empty());
    assert_eq!(kinds(&step(&store, &context)), ["sealed"]);
    assert_eq!(kinds(&step(&store, &context)), ["verified"]);
    let claims = store.checkpoint_claims().unwrap();
    assert_eq!(first_verifications(&claims, &checkpoint).len(), 1);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert!(step(&store, &context).is_empty());
}

#[test]
fn a_late_envelope_before_the_cut_reseals_until_someone_verified() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch, cedar] =
        ["alder", "birch", "cedar"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 2);
    observe(&birch, 2);
    sync(&[&alder, &birch]);
    step(&alder, &context);
    step(&birch, &context);

    // An envelope from before the cut reaches alder late, from a writer it had not heard
    // of. Alder seals again, now with cedar as a participant.
    observe(&cedar, 1);
    sync(&[&alder, &cedar]);
    let actions = step(&alder, &context);
    let CheckpointAction::Sealed { participants, .. } = &actions[0] else {
        panic!("{actions:?}");
    };
    assert_eq!(participants, &names(&["alder", "birch", "cedar"]));
    sync(&[&alder, &birch, &cedar]);
    for node in [&birch, &cedar] {
        assert_eq!(kinds(&step(node, &context)), ["sealed"]);
    }
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(kinds(&step(&alder, &context)), ["verified"]);

    // Once alder verified, a later envelope before the cut never makes it seal again.
    let dogwood = Store::open_memory("dogwood").unwrap();
    observe(&dogwood, 1);
    sync(&[&alder, &dogwood]);
    assert!(step(&alder, &context).is_empty());
    let status = alder.checkpoint_status(context.now_unix_ms, &[]).unwrap();
    assert!(status.pending.unwrap().unsealed.contains("dogwood"));
}

#[test]
fn a_seal_that_returns_to_earlier_terms_is_a_new_seal() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 2);
    observe(&birch, 1);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &context)), ["sealed"]);

    // Birch is excused, so alder seals without it; then birch seals, which ends its
    // excusal, and alder's terms are the first ones again.
    alder
        .excuse_checkpoint_writer(&CheckpointExcuseRequest {
            writer: "birch".into(),
            reason: "away".into(),
            actor: "person/operator".into(),
        })
        .unwrap();
    assert_eq!(kinds(&step(&alder, &context)), ["sealed"]);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&birch, &context)), ["sealed"]);
    sync(&[&alder, &birch]);
    let actions = step(&alder, &context);
    let CheckpointAction::Sealed { participants, .. } = &actions[0] else {
        panic!("{actions:?}");
    };
    assert_eq!(participants, &names(&["alder", "birch"]));
    // The seal took effect, so alder moves on rather than sealing again forever.
    let checkpoint = checkpoint_name(newest_due_cut(context.now_unix_ms));
    assert_eq!(
        newest_seals(&alder.checkpoint_claims().unwrap(), &checkpoint)["alder"].participants,
        names(&["alder", "birch"])
    );
    assert!(!kinds(&step(&alder, &context)).contains(&"sealed"));
}

#[test]
fn a_silent_participant_holds_everything_up_until_a_person_excuses_it() {
    let scratch = tempfile::tempdir().unwrap();
    let first = context(scratch.path(), 0);
    let [alder, birch, cedar] =
        ["alder", "birch", "cedar"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 2);
    observe(&birch, 2);
    // Cedar is an old build, or away: it wrote once and never seals.
    observe(&cedar, 1);
    sync(&[&alder, &birch, &cedar]);
    step(&alder, &first);
    step(&birch, &first);
    sync(&[&alder, &birch]);
    for _ in 0..3 {
        assert!(!kinds(&step(&alder, &first)).contains(&"verified"));
        assert!(!kinds(&step(&birch, &first)).contains(&"verified"));
    }

    // Three days after it became due, the first participant to have sealed asks a person.
    let late = CheckpointContext {
        now_unix_ms: first.now_unix_ms + CHECKPOINT_ATTENTION_AFTER_MS,
        ..first.clone()
    };
    // The newest due checkpoint moved on; seal it first.
    step(&alder, &late);
    step(&birch, &late);
    sync(&[&alder, &birch]);
    step(&alder, &late);
    let items = alder
        .checkpoint_attention_items(Some("person/operator"), late.now_unix_ms)
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].title.contains("cedar"));
    assert_eq!(
        serde_json::to_value(&items).unwrap(),
        serde_json::to_value(
            birch
                .checkpoint_attention_items(Some("person/operator"), late.now_unix_ms)
                .unwrap()
        )
        .unwrap()
    );
    assert!(alder.attention_requests(None, true).unwrap().is_empty());

    // An agent cannot excuse it.
    assert!(
        alder
            .excuse_checkpoint_writer(&CheckpointExcuseRequest {
                writer: "cedar".into(),
                reason: "away".into(),
                actor: "agent/alder.worker".into(),
            })
            .is_err()
    );
    alder
        .excuse_checkpoint_writer(&CheckpointExcuseRequest {
            writer: "cedar".into(),
            reason: "the laptop is in a drawer".into(),
            actor: "person/operator".into(),
        })
        .unwrap();
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &late)), ["sealed"]);
    }
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &late)), ["verified"]);
    }
    sync(&[&alder, &birch]);
    let cut = newest_due_cut(late.now_unix_ms);
    assert_eq!(stable_cuts(&alder), [cut]);
    assert_eq!(kinds(&step(&alder, &late)), ["trimmed"]);
    assert_eq!(kinds(&step(&birch, &late)), ["trimmed"]);

    // Cedar comes back. What it wrote while away replicates. It adopts the checkpoint it
    // missed, and its next seal ends its excusal, so the next checkpoint waits for it.
    observe(&cedar, 1);
    sync(&[&alder, &birch, &cedar]);
    let next = CheckpointContext {
        now_unix_ms: late.now_unix_ms + DAY_MS,
        ..late.clone()
    };
    assert_eq!(kinds(&step(&cedar, &next)), ["manifest-needed"]);
    let need = cedar.checkpoint_manifest_need().unwrap().unwrap();
    let manifest = alder
        .checkpoint_manifest(&need.checkpoint, need.cut_unix_ms)
        .unwrap();
    assert_eq!(
        kinds(&cedar.adopt_checkpoint(&manifest).unwrap()),
        ["trimmed"]
    );
    assert_eq!(claim_ids(&cedar), claim_ids(&alder));
    assert_eq!(kinds(&step(&cedar, &next)), ["sealed"]);
    sync(&[&alder, &birch, &cedar]);
    assert!(
        alder
            .checkpoint_status(next.now_unix_ms, &[])
            .unwrap()
            .excused
            .is_empty()
    );
    let actions = step(&alder, &next);
    let CheckpointAction::Sealed { participants, .. } = &actions[0] else {
        panic!("{actions:?}");
    };
    assert!(participants.contains("cedar"));
}

#[test]
fn a_seal_dates_every_later_write_at_or_after_its_cut() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let alder = Store::open_memory("alder").unwrap();
    observe(&alder, 1);
    step(&alder, &context);
    // The clock goes back three days.
    alder.set_write_clock_offset(-(3 * DAY_MS as i64)).unwrap();
    observe(&alder, 3);
    alder
        .put_document("doc/alder-notes", b"invented notes", &None, "alder-notes")
        .unwrap();
    let claims = alder
        .claims_page(None, None, 0, None, false, 10_000)
        .unwrap()
        .claims;
    let after_seal = claims
        .iter()
        .skip_while(|claim| claim.kind != CHECKPOINT_SEALED)
        .collect::<Vec<_>>();
    assert!(after_seal.len() > 3);
    for claim in after_seal {
        assert!(claim.accepted_at_unix_ms >= cut, "{claim:?}");
    }
    // A clock ahead of the cut is used as it is.
    alder.set_write_clock_offset(10 * DAY_MS as i64).unwrap();
    observe(&alder, 1);
    let newest = alder
        .claims_page(None, None, 0, None, false, 10_000)
        .unwrap()
        .claims;
    assert!(newest.last().unwrap().accepted_at_unix_ms > cut + 5 * DAY_MS);
}

/// Folds read a subject's claims in canonical order, which starts with the accepted time. A
/// writer whose clock steps back must still date each new claim at or after its last one,
/// or its new state would sort before its old state and every node would show the old.
#[test]
fn a_writer_never_dates_a_claim_before_its_own_newest() {
    let alder = Store::open_memory("alder").unwrap();
    alder.set_write_clock_offset(2 * DAY_MS as i64).unwrap();
    observe(&alder, 2);
    alder.set_write_clock_offset(0).unwrap();
    observe(&alder, 2);
    let claims = alder
        .claims_page(None, None, 0, None, false, 10_000)
        .unwrap()
        .claims;
    assert_eq!(claims.len(), 4);
    for pair in claims.windows(2) {
        assert!(pair[1].accepted_at_unix_ms >= pair[0].accepted_at_unix_ms);
    }
    let newest = alder
        .latest_claim(
            &format!("daemon/{}", alder.origin),
            Some("daemon.diagnostic"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(newest.id, claims.last().unwrap().id);
}

/// A trim that would change the graph stops, records why, and seals nothing more until a
/// person has looked. A trim that only bumps the graph generation compares the digests and
/// goes on.
#[test]
fn a_trim_that_would_change_the_graph_waits_for_a_person() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    stable_pair(&alder, &birch, &context);
    alder.set_trim_fault(Some(TrimFault::TouchGraph));
    assert_eq!(kinds(&step(&alder, &context)), ["trimmed"]);

    let mut projection_tables = projection_digest::tables(&birch.readers.get()).unwrap();
    projection_tables.remove("claim_sources");
    projection_tables.remove("operations");
    let previous_operations = projection_digest::operation_rows();
    let previous_operations = birch
        .readers
        .get()
        .prepare(&previous_operations)
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .unwrap();
    let held = claim_ids(&birch);
    birch.set_trim_fault(Some(TrimFault::ChangeGraph));
    assert_eq!(kinds(&step(&birch, &context)), ["graph-changed"]);
    let mut current_tables = projection_digest::tables(&birch.readers.get()).unwrap();
    current_tables.remove("claim_sources");
    current_tables.remove("operations");
    let current_operations = birch
        .readers
        .get()
        .prepare(&projection_digest::operation_rows())
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .unwrap();
    assert!(current_operations.is_superset(&previous_operations));
    // The new checkpoint fault, diagnostic claims and their operation rows change digests.
    // Earlier shared rows remain intact when the trim transaction is refused.
    assert_eq!(current_tables, projection_tables);
    assert!(claim_ids(&birch).is_superset(&held));
    let status = birch.checkpoint_status(context.now_unix_ms, &[]).unwrap();
    assert!(status.halted);
    assert_eq!(status.trimmed, None);
    assert!(
        birch
            .claims_for("daemon/birch", Some("daemon.diagnostic"))
            .unwrap()
            .iter()
            .any(|claim| claim.body["fields"]["code"] == "checkpoint-trim-graph-changed")
    );

    // Nothing more happens, even when the next checkpoint is due.
    let next = CheckpointContext {
        now_unix_ms: context.now_unix_ms + DAY_MS,
        ..context.clone()
    };
    observe(&birch, 1);
    assert!(step(&birch, &next).is_empty());
    assert!(
        birch
            .resume_checkpoints("agent/birch.worker", "looked")
            .is_err()
    );
    assert!(birch.resume_checkpoints("person/operator", " ").is_err());
    birch
        .resume_checkpoints("person/operator", "the change came from a test fault")
        .unwrap();
    assert!(
        !birch
            .checkpoint_status(next.now_unix_ms, &[])
            .unwrap()
            .halted
    );
    assert_eq!(kinds(&step(&birch, &next)), ["sealed"]);
}

fn authority(store: &Store) -> (String, String) {
    let status = store.replication_status(true, Some(FLEET), &[]).unwrap();
    (status.authority_digest, status.graph_digest)
}

fn claim_ids(store: &Store) -> BTreeSet<String> {
    store
        .claims_page(None, None, 0, None, false, 100_000)
        .unwrap()
        .claims
        .into_iter()
        .map(|claim| claim.id)
        .collect()
}

/// Two nodes agree on a checkpoint until it is stable, without trimming it.
fn stable_pair(alder: &Store, birch: &Store, context: &CheckpointContext) -> String {
    observe(alder, 8);
    observe(birch, 5);
    sync(&[alder, birch]);
    for round in ["sealed", "verified"] {
        for node in [alder, birch] {
            assert_eq!(kinds(&step(node, context)), [round]);
        }
        sync(&[alder, birch]);
    }
    checkpoint_name(newest_due_cut(context.now_unix_ms))
}

#[test]
fn a_repaired_original_held_by_one_node_does_not_split_the_verifications() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let [alder, birch, cedar] =
        ["alder", "birch", "cedar"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 6);
    observe(&birch, 3);
    observe(&cedar, 2);
    let replacement = alder
        .append_claim(&ClaimInput {
            subject: "daemon/alder".into(),
            kind: "daemon.diagnostic".into(),
            actor: None,
            fields: BTreeMap::from([
                ("severity".into(), json!("warning")),
                ("code".into(), json!("repair-test")),
                ("reason".into(), json!("replacement")),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("replacement".into()),
        })
        .unwrap();
    let original = alder
        .claims_page(None, None, 0, None, false, 10_000)
        .unwrap()
        .claims
        .into_iter()
        .find(|claim| claim.kind == "daemon.diagnostic" && claim.id != replacement.id)
        .unwrap()
        .id;
    sync(&[&alder, &birch, &cedar]);
    // Birch's build rejected the original, so it never held its row, and a person repaired
    // the record there. Alder and cedar admitted the original first and keep its row once the
    // repair reaches them.
    let record_ref = {
        let connection = birch.connection.lock().unwrap();
        let record_ref = connection
            .query_row(
                "SELECT record_ref FROM replica_records WHERE claim_id=?1",
                [&original],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        connection
            .execute(
                "UPDATE replica_records SET state='invalid' WHERE record_ref=?1",
                [&record_ref],
            )
            .unwrap();
        record_ref
    };
    birch
        .repair_replica_record(
            &record_ref,
            &replacement.id,
            "the receiver rejects this record after an upgrade",
            "person/operator",
            "repair-version-skew",
        )
        .unwrap();
    birch
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM claims WHERE id=?1", [&original])
        .unwrap();
    sync(&[&alder, &birch, &cedar]);
    for node in [&alder, &cedar] {
        assert!(claim_ids(node).contains(&original));
        assert_eq!(
            node.replica_record(&record_ref).unwrap().unwrap().state,
            "repaired"
        );
    }
    assert!(!claim_ids(&birch).contains(&original));

    for round in ["sealed", "verified"] {
        for node in [&alder, &birch, &cedar] {
            assert_eq!(kinds(&step(node, &context)), [round]);
        }
        sync(&[&alder, &birch, &cedar]);
    }
    let claims = alder.checkpoint_claims().unwrap();
    let checkpoint = checkpoint_name(cut);
    let verified = first_verifications(&claims, &checkpoint);
    assert_eq!(
        verified
            .values()
            .map(|(_, terms)| terms)
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "{verified:#?}"
    );
    for node in [&alder, &birch, &cedar] {
        assert_eq!(stable_cuts(node), [cut]);
        assert_eq!(kinds(&step(node, &context)), ["trimmed"]);
    }
    assert_eq!(claim_ids(&alder), claim_ids(&cedar));
    assert_eq!(authority(&alder), authority(&birch));
    assert_eq!(authority(&alder), authority(&cedar));
}

#[test]
fn a_blob_that_arrived_after_the_cut_does_not_split_the_verifications() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 4);
    observe(&birch, 3);
    alder
        .put_document("doc/before-the-cut", b"sealed notes", &None, "before")
        .unwrap();
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &context)), ["sealed"]);
    }
    sync(&[&alder, &birch]);
    // Written after alder sealed, so dated after the cut, and birch has not received it when
    // both verify.
    alder
        .put_document("doc/after-the-cut", b"later notes", &None, "after")
        .unwrap();
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &context)), ["verified"]);
    }
    sync(&[&alder, &birch]);
    let pending = alder
        .checkpoint_status(context.now_unix_ms, &[])
        .unwrap()
        .pending;
    assert!(pending.is_none(), "{pending:#?}");
    for node in [&alder, &birch] {
        assert_eq!(stable_cuts(node), [cut]);
    }
}

#[test]
fn status_names_what_differs_between_verifications() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let checkpoint = checkpoint_name(cut);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 4);
    observe(&birch, 3);
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &context)), ["sealed"]);
    }
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &context)), ["verified"]);
    // Birch verifies a different set of kept claims.
    let terms = birch
        .checkpoint_status(context.now_unix_ms, &[])
        .unwrap()
        .pending
        .unwrap()
        .terms;
    let (_, mut plan, proof) = birch
        .plan_checkpoint_through(cut, None, scratch.path())
        .unwrap();
    plan.retained_digest = "another".into();
    birch
        .publish_verification(&checkpoint, &terms, &BTreeSet::new(), &plan, &proof)
        .unwrap();
    sync(&[&alder, &birch]);

    let pending = alder
        .checkpoint_status(context.now_unix_ms, &[])
        .unwrap()
        .pending
        .unwrap();
    assert!(!pending.verifications_agree);
    assert_eq!(
        pending.verifications_differ,
        BTreeMap::from([("birch".to_owned(), "retained claims".to_owned())])
    );
    assert_eq!(pending.sealed, names(&["alder", "birch"]));
    assert!(pending.disagreeing.is_empty());
    assert!(stable_cuts(&alder).is_empty());
}

#[test]
fn participants_trim_the_stable_checkpoint_and_keep_every_identity() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    let checkpoint = stable_pair(&alder, &birch, &context);
    let before = [&alder, &birch].map(authority);
    let held = claim_ids(&alder);
    let index = alder.index().unwrap();
    for node in [&alder, &birch] {
        let reclaim_before = smallclaims::store::checkpoint::allocator_reclaim_stats_for_test();
        assert_eq!(kinds(&step(node, &context)), ["trimmed"]);
        let reclaimed = smallclaims::store::checkpoint::allocator_reclaim_stats_for_test();
        assert_eq!(reclaimed.0 - reclaim_before.0, 1,
            "checkpoint application and its nested finish reclaim once");
        assert!(step(node, &context).is_empty());
        assert_eq!(smallclaims::store::checkpoint::allocator_reclaim_stats_for_test(), reclaimed,
            "periodic completed-checkpoint polling never reclaims");
    }
    // Every identity stays in the inventory, so the authority digest does not move, and
    // the graph is the same.
    assert_eq!([&alder, &birch].map(authority), before);
    assert_eq!(authority(&alder), authority(&birch));
    let kept = claim_ids(&alder);
    assert!(kept.len() < held.len());
    assert_eq!(kept, claim_ids(&birch));
    let manifests =
        [&alder, &birch].map(|node| node.checkpoint_manifest(&checkpoint, cut).unwrap());
    assert!(!manifests[0].claims.is_empty());
    assert_eq!(manifests[0], manifests[1]);
    for claim in &manifests[0].claims {
        assert!(held.contains(&claim.id) && !kept.contains(&claim.id));
    }
    // Snapshots taken before the trim expire.
    assert!(alder.index().unwrap() > index);
    let status = alder.checkpoint_status(context.now_unix_ms, &[]).unwrap();
    assert_eq!(status.trimmed.as_deref(), Some(checkpoint.as_str()));

    // Both keep replicating new writes.
    observe(&alder, 2);
    observe(&birch, 2);
    sync(&[&alder, &birch]);
    assert_eq!(authority(&alder), authority(&birch));
    assert_eq!(claim_ids(&alder), claim_ids(&birch));
}

#[test]
fn a_trim_that_stops_anywhere_finishes_the_same_after_a_restart() {
    for fault in [
        TrimFault::AfterTombstoneChunk(1),
        TrimFault::AfterTombstones,
        TrimFault::AfterChunk(1),
        TrimFault::AfterChunk(2),
        TrimFault::BeforeFinish,
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let path = scratch.path().join("alder.sqlite3");
        let alder = Store::open(&path, "alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        stable_pair(&alder, &birch, &context);
        assert_eq!(kinds(&step(&birch, &context)), ["trimmed"]);
        alder.set_trim_chunk_envelopes(2);
        if fault == TrimFault::AfterTombstoneChunk(1) {
            // Slow enough that recording takes several chunks, so the crash leaves some
            // tombstones recorded and others not.
            alder.set_trim_row_cost(std::time::Duration::from_millis(12));
        }
        alder.set_trim_fault(Some(fault));
        let reclaim_before = smallclaims::store::checkpoint::allocator_reclaim_stats_for_test();
        assert!(alder.checkpoint_step(&context).is_err(), "{fault:?}");
        if fault == TrimFault::AfterTombstoneChunk(1) {
            let count = |sql: &str| -> u64 { alder.readers.get().query_row(sql, [], |row| row.get(0)).unwrap() };
            let recorded = count("SELECT (SELECT COUNT(*) FROM checkpoint_envelopes) + (SELECT COUNT(*) FROM checkpoint_claims)");
            let deleted = count("SELECT COUNT(*) FROM checkpoint_envelopes AS tombstone WHERE NOT EXISTS (
                SELECT 1 FROM replica_envelopes AS held WHERE held.writer=tombstone.writer
                  AND held.sequence=tombstone.sequence AND held.envelope_hash=tombstone.envelope_hash)");
            assert!(recorded > 0 && deleted == 0, "some tombstones are recorded and nothing is deleted");
            let manifest = birch.checkpoint_manifest(&checkpoint_name(newest_due_cut(context.now_unix_ms)), newest_due_cut(context.now_unix_ms)).unwrap();
            assert!((recorded as usize) < manifest.claims.len() + manifest.envelopes.len(), "the crash came part way through");
        }
        assert_eq!(smallclaims::store::checkpoint::allocator_reclaim_stats_for_test().0 - reclaim_before.0, 1);
        drop(alder);

        let alder = Store::open(&path, "alder").unwrap();
        let reclaim_before = smallclaims::store::checkpoint::allocator_reclaim_stats_for_test();
        let actions = step(&alder, &context);
        assert_eq!(kinds(&actions), ["trimmed"], "{fault:?}");
        assert_eq!(smallclaims::store::checkpoint::allocator_reclaim_stats_for_test().0 - reclaim_before.0, 1);
        assert!(step(&alder, &context).is_empty());
        assert_eq!(claim_ids(&alder), claim_ids(&birch), "{fault:?}");
        assert_eq!(authority(&alder), authority(&birch), "{fault:?}");
        let cut = newest_due_cut(context.now_unix_ms);
        let checkpoint = checkpoint_name(cut);
        assert_eq!(
            alder.checkpoint_manifest(&checkpoint, cut).unwrap(),
            birch.checkpoint_manifest(&checkpoint, cut).unwrap(),
            "{fault:?}"
        );
    }
}

#[test]
fn a_node_that_did_not_take_part_adopts_the_manifest_and_nothing_else() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    let checkpoint = stable_pair(&alder, &birch, &context);
    for node in [&alder, &birch] {
        step(node, &context);
    }
    // A new node gets the kept envelopes, never the dropped ones, and the checkpoint claims.
    let cedar = Store::open_memory("cedar").unwrap();
    sync(&[&alder, &cedar]);
    assert_ne!(authority(&cedar).0, authority(&alder).0);
    assert_eq!(kinds(&step(&cedar, &context)), ["manifest-needed"]);
    assert_eq!(
        cedar
            .checkpoint_manifest_need()
            .unwrap()
            .map(|need| need.checkpoint),
        Some(checkpoint.clone())
    );
    let manifest = alder.checkpoint_manifest(&checkpoint, cut).unwrap();

    // A changed tombstone is refused before anything is stored.
    let mut changed = manifest.clone();
    changed.claims[0]
        .predecessors
        .push("an-invented-claim".into());
    assert_eq!(
        cedar.adopt_checkpoint(&changed).unwrap_err().code,
        "checkpoint-manifest-mismatch"
    );
    assert_eq!(cedar.checkpointed_envelopes().unwrap(), 0);

    assert_eq!(
        kinds(&cedar.adopt_checkpoint(&manifest).unwrap()),
        ["trimmed"]
    );
    assert_eq!(cedar.checkpoint_manifest_need().unwrap(), None);
    assert_eq!(authority(&cedar), authority(&alder));
    assert_eq!(claim_ids(&cedar), claim_ids(&alder));
    assert_eq!(
        cedar.checkpoint_manifest(&checkpoint, cut).unwrap(),
        manifest
    );
    assert!(step(&cedar, &context).is_empty());
}

fn excuse(store: &Store, writer: &str) {
    store
        .excuse_checkpoint_writer(&CheckpointExcuseRequest {
            writer: writer.into(),
            reason: "cut off by a partition".into(),
            actor: "person/operator".into(),
        })
        .unwrap();
}

/// People on each side of a partition excuse the other side, and each side certifies and
/// trims the same cut alone. Once the partition heals, every node applies the same one of
/// the two certificates, and they end with identical tombstones, inventories and graphs,
/// and trim the next checkpoint together.
#[test]
fn both_sides_of_an_excused_partition_converge_on_one_certificate() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let checkpoint = checkpoint_name(cut);
    let [alder, birch, cedar, dogwood] =
        ["alder", "birch", "cedar", "dogwood"].map(|name| Store::open_memory(name).unwrap());
    let all = [&alder, &birch, &cedar, &dogwood];
    for node in all {
        observe(node, 4);
    }
    sync(&all);
    let west = [&alder, &birch];
    let east = [&cedar, &dogwood];
    for (side, others) in [(west, ["cedar", "dogwood"]), (east, ["alder", "birch"])] {
        for node in side {
            observe(node, 3);
        }
        for writer in others {
            excuse(side[0], writer);
        }
        sync(&side);
        for round in ["sealed", "verified", "trimmed"] {
            for node in side {
                assert_eq!(kinds(&step(node, &context)), [round], "{}", node.origin);
            }
            sync(&side);
        }
    }
    let [west_certificate, east_certificate] =
        [&alder, &cedar].map(|node| node.trimmed_checkpoint().unwrap().unwrap());
    assert_ne!(west_certificate.drop_digest, east_certificate.drop_digest);

    // The partition heals.
    sync(&all);
    let certified = certificates(&alder.checkpoint_claims().unwrap(), &checkpoint);
    assert_eq!(certified.len(), 2);
    let chosen = chosen_certificate(&certified).unwrap().clone();
    let (kept, switching) = if chosen.terms.drop_digest == west_certificate.drop_digest {
        (west, east)
    } else {
        (east, west)
    };
    for node in kept {
        assert_eq!(node.checkpoint_manifest_need().unwrap(), None);
        assert!(!kinds(&step(node, &context)).contains(&"manifest-needed"));
    }
    let manifest = kept[0].checkpoint_manifest(&checkpoint, cut).unwrap();
    for node in switching {
        assert_eq!(kinds(&step(node, &context)), ["manifest-needed"]);
        let need = node.checkpoint_manifest_need().unwrap().unwrap();
        assert_eq!(need.drop_digest, chosen.terms.drop_digest);
        // The other side's manifest does not verify against the chosen certificate.
        let own = node.checkpoint_manifest(&checkpoint, cut).unwrap();
        assert!(node.adopt_checkpoint(&own).is_err());
        assert_eq!(
            kinds(&node.adopt_checkpoint(&manifest).unwrap()),
            ["trimmed"]
        );
        assert_eq!(
            node.checkpoint_manifest(&checkpoint, cut).unwrap(),
            manifest
        );
    }
    sync(&all);
    for node in all {
        assert_eq!(authority(node), authority(&alder), "{}", node.origin);
        assert_eq!(claim_ids(node), claim_ids(&alder), "{}", node.origin);
        assert_eq!(
            node.checkpoint_manifest(&checkpoint, cut).unwrap(),
            manifest,
            "{}",
            node.origin
        );
        assert_eq!(
            node.trimmed_checkpoint().unwrap().unwrap().drop_digest,
            chosen.terms.drop_digest
        );
        assert!(!kinds(&step(node, &context)).contains(&"manifest-needed"));
    }

    // Every writer seals the next checkpoint, which ends its excusal, and all four trim it
    // together.
    let next = CheckpointContext {
        now_unix_ms: context.now_unix_ms + DAY_MS,
        ..context.clone()
    };
    for node in all {
        observe(node, 2);
    }
    sync(&all);
    for round in ["sealed", "verified", "trimmed"] {
        for node in all {
            assert!(
                kinds(&step(node, &next)).contains(&round),
                "{} did not reach {round}",
                node.origin
            );
        }
        sync(&all);
    }
    for node in all {
        assert_eq!(authority(node), authority(&alder), "{}", node.origin);
        assert_eq!(claim_ids(node), claim_ids(&alder), "{}", node.origin);
    }
    // The older checkpoint's manifest lacks the newer drops, so no node adopts it now.
    assert_eq!(
        alder.adopt_checkpoint(&manifest).unwrap_err().code,
        "checkpoint-superseded"
    );
}

#[test]
fn a_node_that_left_the_fleet_is_not_waited_for() {
    let known = names(&["alder", "birch"]);
    let left = names(&["birch"]);
    assert_eq!(participants(&known, &left, &[]), names(&["alder"]));
}

#[test]
fn v9_v10_seals_wait_for_matching_rules_before_verifying() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let checkpoint = checkpoint_name(cut);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 4);
    observe(&birch, 3);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &context)), ["sealed"]);
    sync(&[&alder, &birch]);
    let mut legacy =
        newest_seals(&alder.checkpoint_claims().unwrap(), &checkpoint)["alder"].clone();
    // Exact rules-v9 description/version from main 2e075af9, before native-auth retention.
    legacy.rules_digest = "ba69471dde5e5a9d835a2576a55a9fe9b1c387095ee1dcabdd6abc8e03400780".into();
    assert_ne!(legacy.rules_digest, rules_digest());
    let sealed = birch.checkpoint_sealed_identities(cut, None).unwrap();
    assert_eq!(sealed.digest, legacy.sealed_digest);
    birch
        .publish_seal(&checkpoint, &legacy, &sealed, None)
        .unwrap();
    sync(&[&alder, &birch]);
    assert!(step(&alder, &context).is_empty());
    let pending = alder
        .checkpoint_status(context.now_unix_ms, &[])
        .unwrap()
        .pending
        .unwrap();
    assert_eq!(pending.disagreeing["birch"], "rules");
    assert!(pending.verified.is_empty());
    assert!(stable_cuts(&alder).is_empty());
    assert!(stable_cuts(&birch).is_empty());
    // The updated participant reseals under v10; only then can either side verify.
    assert_eq!(kinds(&step(&birch, &context)), ["sealed"]);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &context)), ["verified"]);
    assert_eq!(kinds(&step(&birch, &context)), ["verified"]);
    sync(&[&alder, &birch]);
    assert_eq!(stable_cuts(&alder), [cut]);
    assert_eq!(stable_cuts(&birch), [cut]);
}

#[test]
fn stale_rule_seals_do_not_end_excusal_until_normal_matching_runtime_reentry() {
    let scratch = tempfile::tempdir().unwrap();
    let first = context(scratch.path(), 0);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 2);
    observe(&birch, 2);
    sync(&[&alder, &birch]);
    alder.excuse_checkpoint_writer(&CheckpointExcuseRequest {
        writer:"birch".into(), reason:"old runtime is away".into(), actor:"person/operator".into(),
    }).unwrap();
    sync(&[&alder, &birch]);
    let cut = newest_due_cut(first.now_unix_ms);
    let sealed = birch.checkpoint_sealed_identities(cut, None).unwrap();
    birch.publish_seal(&checkpoint_name(cut), &SealTerms {
        cut_unix_ms:cut, participants:names(&["alder", "birch"]),
        sealed_digest:sealed.digest.clone(), rules_digest:"rules13".into(),
    }, &sealed, None).unwrap();
    sync(&[&alder, &birch]);
    let status = alder.checkpoint_status(first.now_unix_ms, &[]).unwrap();
    assert_eq!(status.excused, names(&["birch"]));
    assert_eq!(status.participants, names(&["alder"]));
    let actions = step(&alder, &first);
    let CheckpointAction::Sealed { participants, .. } = &actions[0] else { panic!("{actions:?}") };
    assert_eq!(participants, &names(&["alder"]));
    assert_eq!(kinds(&step(&alder, &first)), ["verified"]);
    sync(&[&alder, &birch]);
    assert_eq!(kinds(&step(&alder, &first)), ["trimmed"]);
    let next = CheckpointContext { now_unix_ms:first.now_unix_ms + DAY_MS, ..first };
    let actions = step(&birch, &next);
    if kinds(&actions).contains(&"manifest-needed") {
        let need = birch.checkpoint_manifest_need().unwrap().unwrap();
        let manifest = alder.checkpoint_manifest(&need.checkpoint, need.cut_unix_ms).unwrap();
        assert_eq!(kinds(&birch.adopt_checkpoint(&manifest).unwrap()), ["trimmed"]);
        assert_eq!(kinds(&step(&birch, &next)), ["sealed"]);
    } else {
        assert!(kinds(&actions).contains(&"trimmed"), "{actions:?}");
        assert!(kinds(&actions).contains(&"sealed"), "{actions:?}");
    }
    sync(&[&alder, &birch]);
    let status = alder.checkpoint_status(next.now_unix_ms, &[]).unwrap();
    assert!(status.excused.is_empty());
    assert_eq!(status.participants, names(&["alder", "birch"]));
}

#[test]
fn the_agent_card_fold_matches_per_subject_reductions_after_a_trim() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    for (store, seat) in [(&alder, "agent/alder.keeper"), (&birch, "agent/birch.keeper")] {
        for (kind, fields) in [
            ("runtime.observed", json!({"status": "running", "runtime_id": seat,
                "incarnation_id": "keeper-1"})),
            ("harness.observed", json!({"state": "working", "driver": "omp",
                "incarnation_id": "keeper-1"})),
            ("harness.observed", json!({"state": "idle", "driver": "omp",
                "incarnation_id": "keeper-1"})),
            ("runtime.observed", json!({"status": "stopped", "runtime_id": seat,
                "incarnation_id": "keeper-1"})),
            ("runtime.observed", json!({"status": "running", "runtime_id": seat,
                "incarnation_id": "keeper-2"})),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: kind.into(),
                    actor: Some(seat.into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
    }
    sync(&[&alder, &birch]);
    let mut trimmed = false;
    for _ in 0..4 {
        for node in [&alder, &birch] {
            trimmed |= kinds(&step(node, &context)).contains(&"trimmed");
        }
        sync(&[&alder, &birch]);
    }
    assert!(trimmed, "the fixture reached a trim");
    for node in [&alder, &birch] {
        let indexes = (0..=node.index().unwrap()).collect::<Vec<_>>();
        super::card_fold_tests::assert_card_fold_parity(node, &indexes);
    }
}

/// The rules digest of rules version 14, which this build's rules replace.
const RULES_V14_DIGEST: &str = "6a1426cc23bc9cb8aadc647983d20d79db2b7832b7d846d03e16a80be8412385";
/// The rules digest of rules version 15, which members deployed without the lagging agreement run.
const RULES_V15_DIGEST: &str = "319a5838f0c82b8c52f52ce93ace9a31840f8df17cfd3ffc14917a92c8b7d910";

fn seal_terms(participants: &[&str], sealed: &str, rules: &str) -> SealTerms {
    SealTerms {
        cut_unix_ms: CUT,
        participants: names(participants),
        sealed_digest: sealed.into(),
        rules_digest: rules.into(),
    }
}

fn lagging_verified(writer: &str, participants: &[&str], lagging: Value) -> CheckpointClaim {
    let mut claim = verified(writer, participants, "drop");
    claim.fields.insert("lagging".into(), lagging);
    claim.id = format!("{}:{}", claim.id, claim.fields["lagging"]);
    claim
}

#[test]
fn only_a_minority_sealing_with_older_rules_is_lagging() {
    let all = ["alder", "birch", "cedar", "dogwood"];
    let ours = seal_terms(&all, "sealed", "v15");
    let older = |rules: &str| rules == "v14";
    let seals = |differing: &[(&str, SealTerms)]| {
        let mut seals = all
            .iter()
            .map(|writer| ((*writer).to_owned(), ours.clone()))
            .collect::<BTreeMap<_, _>>();
        for (writer, terms) in differing {
            seals.insert((*writer).to_owned(), terms.clone());
        }
        seals
    };
    // Every seal matches: nobody lags.
    assert_eq!(lagging_participants(&ours, &seals(&[]), older), Some(BTreeSet::new()));
    // One participant seals with rules this build replaces.
    let v14 = seal_terms(&all, "sealed", "v14");
    assert_eq!(
        lagging_participants(&ours, &seals(&[("dogwood", v14.clone())]), older),
        Some(names(&["dogwood"]))
    );
    // Rules this build does not know as older, equal or newer ones alike, are a disagreement.
    let unknown = seal_terms(&all, "sealed", "v16");
    assert_eq!(lagging_participants(&ours, &seals(&[("dogwood", unknown)]), older), None);
    // Older rules with anything else different, or no seal at all, still hold everyone up.
    let other_envelopes = seal_terms(&all, "other sealed", "v14");
    assert_eq!(lagging_participants(&ours, &seals(&[("dogwood", other_envelopes)]), older), None);
    let other_participants = seal_terms(&all[..3], "sealed", "v14");
    assert_eq!(
        lagging_participants(&ours, &seals(&[("dogwood", other_participants)]), older),
        None
    );
    let mut unsealed = seals(&[]);
    unsealed.remove("dogwood");
    assert_eq!(lagging_participants(&ours, &unsealed, older), None);
    // Half the participants lagging leaves too few to verify the new rules.
    assert_eq!(
        lagging_participants(&ours, &seals(&[("cedar", v14.clone()), ("dogwood", v14)]), older),
        None
    );
}

#[test]
fn a_certificate_with_lagging_participants_needs_every_other_one_and_a_majority() {
    let all = ["alder", "birch", "cedar"];
    let checkpoint = checkpoint_name(CUT);
    let lagging = json!(["cedar"]);
    let claims = ["alder", "birch"].map(|writer| lagging_verified(writer, &all, lagging.clone()));
    let certificate = &certificates(&claims, &checkpoint)[0];
    assert_eq!(certificate.terms.lagging, names(&["cedar"]));
    assert_eq!(
        certificate.verifications.keys().cloned().collect::<BTreeSet<_>>(),
        names(&["alder", "birch"])
    );
    // A verifier missing.
    assert!(certificates(&claims[..1], &checkpoint).is_empty());
    // Verifiers that disagree on who lags are two groups, neither complete.
    let mut disagreeing = claims.to_vec();
    disagreeing[1] = verified("birch", &all, "drop");
    assert!(certificates(&disagreeing, &checkpoint).is_empty());
    // The lagging participant's own verification of other terms changes nothing.
    let mut late = claims.to_vec();
    late.push(verified("cedar", &all, "drop"));
    assert_eq!(certificates(&late, &checkpoint), certificates(&claims, &checkpoint));
    // Terms that leave half or fewer of the participants verifying certify nothing.
    let minority = lagging_verified("alder", &all, json!(["birch", "cedar"]));
    assert!(certificates(&[minority], &checkpoint).is_empty());
    // Nor do terms that name a lagging writer that is not a participant.
    let stranger = ["alder", "birch"].map(|writer| {
        lagging_verified(writer, &["alder", "birch"], json!(["cedar"]))
    });
    assert!(certificates(&stranger, &checkpoint).is_empty());
    // A lagging field that is not a list of names is no verification at all.
    assert_eq!(lagging_verified("alder", &all, json!("cedar")).verified_terms(), None);
    // Verifications without the field are the certificates they always were.
    let complete = all.map(|writer| verified(writer, &all, "drop"));
    let certificate = &certificates(&complete, &checkpoint)[0];
    assert!(certificate.terms.lagging.is_empty());
    assert_eq!(certificate.verifications.len(), 3);
}

fn graph(store: &Store) -> String {
    store.export_replication_summary(FLEET).unwrap().graph_digest
}

fn trimmed_drop(store: &Store) -> Option<String> {
    store.trimmed_checkpoint().unwrap().map(|trimmed| trimmed.drop_digest)
}

// Every member here runs current code. A member on an older build is simulated: its rules
// digest is overridden (`set_checkpoint_rules`) and, for a build without the `lagging` field, its
// claim registry loses that field (`set_claim_registry`). Its decoder, sealing and replication
// are this build's, not the old binary's.

#[test]
fn members_on_new_rules_seal_and_trim_while_one_lags_and_it_catches_up_after_upgrading() {
    a_member_lags_a_rules_version(RULES_V14_DIGEST, false);
}

/// Simulates the rules-14 build every member ran before 2026-10-10. It has no `lagging` field, so
/// it keeps such verifications as claims of a newer schema: it sees no certificate, trims nothing
/// and adopts nothing.
#[test]
fn a_member_on_rules_14_lags_without_seeing_the_certificate() {
    a_member_lags_a_rules_version(RULES_V14_DIGEST, true);
}

/// Simulates a rules-15 build without the lagging agreement. Its rules digest differs from this
/// build's, so it is a laggard here, not a peer that would wait for others' seals to match.
#[test]
fn a_member_on_rules_15_without_the_lagging_agreement_lags_the_same_way() {
    a_member_lags_a_rules_version(RULES_V15_DIGEST, true);
}

fn a_member_lags_a_rules_version(rules: &str, before_this_build: bool) {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let first = context(scratch.path(), 0);
    let second = context(scratch.path(), 1);
    let third = context(scratch.path(), 2);
    let path = |name: &str| directory.path().join(format!("{name}.sqlite3"));
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open(&path(name), name).unwrap());
    // Cedar runs an earlier build, which knows no newer rules.
    let cedar = Store::open(&path("cedar"), "cedar").unwrap();
    cedar.set_checkpoint_rules(rules, &[]);
    if before_this_build {
        let mut registry = st3_schema::registry().clone();
        let verified = registry.claims.get_mut(CHECKPOINT_VERIFIED).unwrap();
        assert!(verified.fields.remove("lagging").is_some());
        cedar.set_claim_registry(registry);
    }
    // What cedar sees of a certificate it lags: this build knows it and refuses to apply it;
    // the build before has none to apply.
    let cedar_cuts = |cuts: &[u128]| if before_this_build { Vec::new() } else { cuts.to_vec() };
    let refusal = if before_this_build {
        "checkpoint-not-stable"
    } else {
        "checkpoint-rules-lagging"
    };
    for store in [&alder, &birch, &cedar] {
        observe(store, 3);
    }
    sync(&[&alder, &birch, &cedar]);
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &first)), ["sealed"]);
    }
    sync(&[&alder, &birch, &cedar]);

    // The status names cedar as lagging, not as differing.
    let cut = newest_due_cut(first.now_unix_ms);
    let pending = alder.checkpoint_status(first.now_unix_ms, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.lagging, names(&["cedar"]));
    assert!(pending.disagreeing.is_empty(), "{pending:?}");
    // The members on rules 15 verify without cedar; cedar, on older rules, waits.
    assert_eq!(kinds(&step(&alder, &first)), ["verified"]);
    assert!(step(&cedar, &first).is_empty());
    // A member restarts between verifying and trimming.
    drop(birch);
    let birch = Store::open(&path("birch"), "birch").unwrap();
    assert_eq!(kinds(&step(&birch, &first)), ["verified"]);
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(stable_cuts(&cedar), cedar_cuts(&[cut]));
    let certificate = stable_checkpoints(&alder.checkpoint_claims().unwrap())[&cut][0].clone();
    assert_eq!(certificate.terms.lagging, names(&["cedar"]));
    assert_eq!(certificate.terms.rules_digest, rules_digest());
    assert_eq!(kinds(&step(&alder, &first)), ["trimmed"]);
    assert_eq!(kinds(&step(&birch, &first)), ["trimmed"]);

    // Cedar never applies rules it does not run: it asks for no manifest and refuses one.
    assert!(step(&cedar, &first).is_empty());
    assert_eq!(cedar.checkpoint_manifest_need().unwrap(), None);
    assert_eq!(trimmed_drop(&cedar), None);
    let manifest = alder.checkpoint_manifest(&checkpoint_name(cut), cut).unwrap();
    assert_eq!(cedar.adopt_checkpoint(&manifest).unwrap_err().code, refusal);

    // Cedar goes on writing, dated after the cut it sealed, and its writes reach the others,
    // which project the same graph.
    observe(&cedar, 2);
    observe(&alder, 1);
    sync(&[&alder, &birch, &cedar]);
    let cedar_writes = |store: &Store| {
        store
            .claims_page(None, None, 0, None, false, 10_000)
            .unwrap()
            .claims
            .into_iter()
            .filter(|claim| claim.origin == "cedar" && claim.kind == "daemon.diagnostic")
            .filter(|claim| claim.accepted_at_unix_ms >= cut)
            .map(|claim| claim.id)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(cedar_writes(&cedar).len(), 2);
    assert_eq!(cedar_writes(&alder), cedar_writes(&cedar));
    assert_eq!(cedar_writes(&birch), cedar_writes(&cedar));
    assert_eq!(graph(&alder), graph(&birch));

    // The next checkpoint: cedar, untrimmed, seals the same envelopes as the trimmed members,
    // so they go on without it again.
    let cut2 = newest_due_cut(second.now_unix_ms);
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &second)), ["sealed"]);
    }
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(kinds(&step(&alder, &second)), ["verified"]);
    assert_eq!(kinds(&step(&birch, &second)), ["verified"]);
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(kinds(&step(&alder, &second)), ["trimmed"]);
    assert_eq!(kinds(&step(&birch, &second)), ["trimmed"]);
    assert!(step(&cedar, &second).is_empty());
    assert_eq!(stable_cuts(&cedar), cedar_cuts(&[cut, cut2]));
    // The build before keeps the four verifications for an upgrade, and finds none invalid.
    let status = cedar.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(status.invalid_records, 0);
    assert_eq!(status.waiting_claims, if before_this_build { 4 } else { 0 });
    assert_eq!(trimmed_drop(&alder), trimmed_drop(&birch));

    // Cedar upgrades: it restarts on this build, adopts the newest checkpoint from a peer, and
    // then takes part in the next one like the others.
    drop(cedar);
    let cedar = Store::open(&path("cedar"), "cedar").unwrap();
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(stable_cuts(&cedar), [cut, cut2]);
    assert_eq!(kinds(&step(&cedar, &second)), ["manifest-needed"]);
    let need = cedar.checkpoint_manifest_need().unwrap().unwrap();
    assert_eq!(need.cut_unix_ms, cut2);
    let manifest = alder.checkpoint_manifest(&need.checkpoint, need.cut_unix_ms).unwrap();
    assert_eq!(kinds(&cedar.adopt_checkpoint(&manifest).unwrap()), ["trimmed"]);
    assert_eq!(trimmed_drop(&cedar), trimmed_drop(&alder));
    sync(&[&alder, &birch, &cedar]);
    assert_eq!(graph(&cedar), graph(&alder));
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &third)), ["sealed"]);
    }
    sync(&[&alder, &birch, &cedar]);
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &third)), ["verified"]);
    }
    sync(&[&alder, &birch, &cedar]);
    let cut3 = newest_due_cut(third.now_unix_ms);
    let certificate = stable_checkpoints(&cedar.checkpoint_claims().unwrap())[&cut3][0].clone();
    assert!(certificate.terms.lagging.is_empty());
    assert_eq!(certificate.verifications.len(), 3);
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &third)), ["trimmed"]);
    }
    assert_eq!(trimmed_drop(&cedar), trimmed_drop(&alder));
    assert_eq!(trimmed_drop(&birch), trimmed_drop(&alder));
    assert_eq!(graph(&cedar), graph(&alder));
    assert_eq!(graph(&birch), graph(&alder));
}

#[test]
fn a_member_that_upgraded_first_does_not_certify_its_new_rules_alone() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch, cedar] =
        ["alder", "birch", "cedar"].map(|name| Store::open_memory(name).unwrap());
    // Alder runs rules that replace this build's; birch and cedar run this build.
    alder.set_checkpoint_rules("rules-16", &[&rules_digest()]);
    for store in [&alder, &birch, &cedar] {
        observe(store, 2);
    }
    sync(&[&alder, &birch, &cedar]);
    for store in [&alder, &birch, &cedar] {
        assert_eq!(kinds(&step(store, &context)), ["sealed"]);
    }
    sync(&[&alder, &birch, &cedar]);
    // Two of three lag alder, too many for it to verify alone; to birch and cedar, alder's
    // newer rules are a disagreement. Nobody verifies until the deploy finishes.
    for store in [&alder, &birch, &cedar] {
        assert!(step(store, &context).is_empty());
    }
    let pending = alder.checkpoint_status(context.now_unix_ms, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.lagging, names(&["birch", "cedar"]));
    let pending = birch.checkpoint_status(context.now_unix_ms, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.disagreeing["alder"], "rules");
    assert!(pending.lagging.is_empty());
    for store in [&alder, &birch, &cedar] {
        assert!(stable_cuts(store).is_empty());
    }
}

#[test]
fn only_rules_14_and_15_lag_this_build() {
    let checkpoint_rules = super::checkpoint_rules::supersedes;
    assert!(checkpoint_rules(RULES_V14_DIGEST));
    assert!(checkpoint_rules(RULES_V15_DIGEST));
    assert!(!checkpoint_rules(&rules_digest()));
    // Rules 9, from the test above, and anything unknown are not known as older.
    assert!(!checkpoint_rules("ba69471dde5e5a9d835a2576a55a9fe9b1c387095ee1dcabdd6abc8e03400780"));
    assert!(!checkpoint_rules("rules13"));
}

#[test]
fn an_earlier_verification_decides_who_lags_while_its_laggards_still_lag_or_have_upgraded() {
    let all = ["alder", "birch", "cedar"];
    let checkpoint = checkpoint_name(CUT);
    let ours = SealTerms {
        sealed_digest: "sealed".into(),
        rules_digest: "rules".into(),
        ..seal_terms(&all, "sealed", "rules")
    };
    let older = |rules: &str| rules == "v14";
    let seals = |cedar: SealTerms| {
        let mut seals = all
            .iter()
            .map(|writer| ((*writer).to_owned(), ours.clone()))
            .collect::<BTreeMap<_, _>>();
        seals.insert("cedar".into(), cedar);
        seals
    };
    let anchor = lagging_verified("alder", &all, json!(["cedar"]));
    // Cedar upgraded and seals our terms since alder verified without it: the set stands.
    assert_eq!(
        lagging_for_verification(&ours, &seals(ours.clone()), &[anchor.clone()], &checkpoint, older),
        Some(names(&["cedar"]))
    );
    // Cedar still lags: the set stands too.
    let v14 = seal_terms(&all, "sealed", "v14");
    assert_eq!(
        lagging_for_verification(&ours, &seals(v14.clone()), &[anchor.clone()], &checkpoint, older),
        Some(names(&["cedar"]))
    );
    // Without an earlier verification, an upgraded cedar is no laggard.
    assert_eq!(
        lagging_for_verification(&ours, &seals(ours.clone()), &[], &checkpoint, older),
        Some(BTreeSet::new())
    );
    // Cedar now differs in its envelopes: the earlier set no longer holds, and neither does any.
    let moved = seal_terms(&all, "other sealed", "v14");
    assert_eq!(
        lagging_for_verification(&ours, &seals(moved), &[anchor.clone()], &checkpoint, older),
        None
    );
    // A verification by a writer its own terms name as lagging decides nothing.
    let own = lagging_verified("cedar", &all, json!(["cedar"]));
    assert_eq!(
        lagging_for_verification(&ours, &seals(ours.clone()), &[own], &checkpoint, older),
        Some(BTreeSet::new())
    );
    // Nor does a second verification by a writer, or one of other seal terms.
    let first = verified("alder", &all, "drop");
    assert_eq!(
        lagging_for_verification(&ours, &seals(ours.clone()), &[first, anchor], &checkpoint, older),
        Some(BTreeSet::new())
    );
    let other_terms = lagging_verified("birch", &["alder", "birch", "cedar", "dogwood"], json!(["cedar"]));
    assert_eq!(
        lagging_for_verification(&ours, &seals(v14), &[other_terms], &checkpoint, older),
        Some(names(&["cedar"]))
    );
}

/// Opens `names` on files in `directory`; the last one runs rules 14 until it is reopened.
fn lagging_fleet(directory: &Path, names: &[&str]) -> Vec<Store> {
    let stores = names
        .iter()
        .map(|name| Store::open(&directory.join(format!("{name}.sqlite3")), *name).unwrap())
        .collect::<Vec<_>>();
    stores.last().unwrap().set_checkpoint_rules(RULES_V14_DIGEST, &[]);
    for store in &stores {
        observe(store, 2);
    }
    stores
}

fn everyone(fleet: &[Store]) -> Vec<&Store> {
    fleet.iter().collect()
}

fn upgrade(directory: &Path, store: Store) -> Store {
    let name = store.origin.clone();
    drop(store);
    Store::open(&directory.join(format!("{name}.sqlite3")), name).unwrap()
}

fn verified_lagging(store: &Store, writer: &str, checkpoint: &str) -> BTreeSet<String> {
    first_verifications(&store.checkpoint_claims().unwrap(), checkpoint)[writer].1.lagging.clone()
}

#[test]
fn a_member_that_upgrades_between_two_verifications_is_still_verified_without() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let checkpoint = checkpoint_name(newest_due_cut(context.now_unix_ms));
    let mut fleet = lagging_fleet(directory.path(), &["alder", "birch", "cedar"]);
    sync(&everyone(&fleet));
    for store in &fleet {
        assert_eq!(kinds(&step(store, &context)), ["sealed"]);
    }
    sync(&everyone(&fleet));
    assert_eq!(kinds(&step(&fleet[0], &context)), ["verified"]);
    // Cedar upgrades and seals again, now matching, before birch verifies.
    let cedar = upgrade(directory.path(), fleet.pop().unwrap());
    fleet.push(cedar);
    sync(&everyone(&fleet));
    assert_eq!(kinds(&step(&fleet[2], &context)), ["sealed"]);
    sync(&everyone(&fleet));
    // Birch verifies without cedar, as alder did, so their verifications form one group.
    assert_eq!(kinds(&step(&fleet[1], &context)), ["verified"]);
    assert_eq!(verified_lagging(&fleet[1], "birch", &checkpoint), names(&["cedar"]));
    // Cedar, named lagging, does not verify.
    assert!(step(&fleet[2], &context).is_empty());
    sync(&everyone(&fleet));
    for store in &fleet[..2] {
        assert_eq!(kinds(&step(store, &context)), ["trimmed"]);
    }
    // Cedar runs the certified rules now, so it adopts the manifest.
    assert_eq!(kinds(&step(&fleet[2], &context)), ["manifest-needed"]);
    let need = fleet[2].checkpoint_manifest_need().unwrap().unwrap();
    let manifest = fleet[0].checkpoint_manifest(&need.checkpoint, need.cut_unix_ms).unwrap();
    assert_eq!(kinds(&fleet[2].adopt_checkpoint(&manifest).unwrap()), ["trimmed"]);
    sync(&everyone(&fleet));
    assert_eq!(trimmed_drop(&fleet[2]), trimmed_drop(&fleet[0]));
    assert_eq!(graph(&fleet[2]), graph(&fleet[0]));
    assert_eq!(graph(&fleet[1]), graph(&fleet[0]));
}

#[test]
fn a_member_that_upgrades_after_one_verification_waits_for_the_certificate_and_adopts() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let checkpoint = checkpoint_name(cut);
    let mut fleet = lagging_fleet(directory.path(), &["alder", "birch", "cedar", "dogwood"]);
    sync(&everyone(&fleet));
    for store in &fleet {
        assert_eq!(kinds(&step(store, &context)), ["sealed"]);
    }
    sync(&everyone(&fleet));
    assert_eq!(kinds(&step(&fleet[0], &context)), ["verified"]);
    sync(&everyone(&fleet));
    // Dogwood upgrades after alder verified without it, before the certificate is complete.
    let dogwood = upgrade(directory.path(), fleet.pop().unwrap());
    fleet.push(dogwood);
    assert_eq!(kinds(&step(&fleet[3], &context)), ["sealed"]);
    sync(&everyone(&fleet));
    // Its status says it lags this checkpoint, and it does not verify.
    let pending = fleet[3].checkpoint_status(context.now_unix_ms, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.lagging, names(&["dogwood"]));
    assert!(pending.disagreeing.is_empty(), "{pending:?}");
    assert!(step(&fleet[3], &context).is_empty());
    for store in &fleet[1..3] {
        assert_eq!(kinds(&step(store, &context)), ["verified"]);
    }
    sync(&everyone(&fleet));
    for writer in ["alder", "birch", "cedar"] {
        assert_eq!(verified_lagging(&fleet[0], writer, &checkpoint), names(&["dogwood"]));
    }
    assert_eq!(stable_cuts(&fleet[3]), [cut]);
    for store in &fleet[..3] {
        assert_eq!(kinds(&step(store, &context)), ["trimmed"]);
    }
    assert_eq!(kinds(&step(&fleet[3], &context)), ["manifest-needed"]);
    let manifest = fleet[0].checkpoint_manifest(&checkpoint, cut).unwrap();
    assert_eq!(kinds(&fleet[3].adopt_checkpoint(&manifest).unwrap()), ["trimmed"]);
    sync(&everyone(&fleet));
    for store in &fleet[1..] {
        assert_eq!(trimmed_drop(store), trimmed_drop(&fleet[0]));
        assert_eq!(graph(store), graph(&fleet[0]));
    }
}

/// The boundary that stays open: two verifiers that each publish their one verification before
/// they hold the other's, one before and one after a laggard upgraded, name different laggards.
/// Each verifies once, so neither group completes, and the checkpoint is never stable. Nothing is
/// trimmed, and the next checkpoint goes on.
#[test]
fn two_verifiers_that_disagree_on_who_lags_leave_the_checkpoint_uncertified() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let first = context(scratch.path(), 0);
    let second = context(scratch.path(), 1);
    let checkpoint = checkpoint_name(newest_due_cut(first.now_unix_ms));
    let mut fleet = lagging_fleet(directory.path(), &["alder", "birch", "cedar", "dogwood"]);
    sync(&everyone(&fleet));
    for store in &fleet {
        assert_eq!(kinds(&step(store, &first)), ["sealed"]);
    }
    sync(&everyone(&fleet));
    let dogwood = upgrade(directory.path(), fleet.pop().unwrap());
    fleet.push(dogwood);
    assert_eq!(kinds(&step(&fleet[3], &first)), ["sealed"]);
    // Only birch holds dogwood's new seal when both verify.
    sync(&[&fleet[1], &fleet[3]]);
    assert_eq!(kinds(&step(&fleet[0], &first)), ["verified"]);
    assert_eq!(kinds(&step(&fleet[1], &first)), ["verified"]);
    sync(&everyone(&fleet));
    assert_eq!(verified_lagging(&fleet[0], "alder", &checkpoint), names(&["dogwood"]));
    assert_eq!(verified_lagging(&fleet[0], "birch", &checkpoint), BTreeSet::new());
    // Cedar joins alder's group; dogwood, named lagging there, does not verify.
    assert_eq!(kinds(&step(&fleet[2], &first)), ["verified"]);
    assert!(step(&fleet[3], &first).is_empty());
    sync(&everyone(&fleet));
    for store in &fleet {
        assert!(stable_cuts(store).is_empty());
        assert!(step(store, &first).is_empty());
        assert_eq!(trimmed_drop(store), None);
    }
    // The next checkpoint has no laggard: all four verify it and trim.
    for store in &fleet {
        assert_eq!(kinds(&step(store, &second)), ["sealed"]);
    }
    sync(&everyone(&fleet));
    for store in &fleet {
        assert_eq!(kinds(&step(store, &second)), ["verified"]);
    }
    sync(&everyone(&fleet));
    for store in &fleet {
        assert_eq!(kinds(&step(store, &second)), ["trimmed"]);
    }
    for store in &fleet[1..] {
        assert_eq!(trimmed_drop(store), trimmed_drop(&fleet[0]));
        assert_eq!(graph(store), graph(&fleet[0]));
    }
}
