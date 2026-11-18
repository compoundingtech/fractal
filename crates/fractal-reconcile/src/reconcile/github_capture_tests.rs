use super::*;

fn definition() -> SourceDefinition {
    SourceDefinition {
        source: Source { host: "github.com".into(), id: "public-capture".into(), generation: 1 },
        authority_grant: Some("grant-one".into()), repositories: vec!["repo".into()], enabled: true,
        checkpoint_interval_ms: 60_000, replicate_private: false,
    }
}

fn database() -> Result<Connection> {
    let connection = Connection::open_in_memory()?;
    migrate(&connection)?;
    Ok(connection)
}

fn hold(connection: &Connection, id: &str, kind: &str, origin: &str, record: &impl serde::Serialize) -> Result<()> {
    let record = serde_json::to_value(record)?;
    let source: Source = serde_json::from_value(record["source"].clone())?;
    let sequence = record.get("sequence").and_then(Value::as_u64);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h4237074c7d77 owner=fractal_reconcile::reconcile::github_capture_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO capture_v1_claims VALUES(?1,?2,?3,?4,?5,?6)", params![id,kind,origin,source_key(&source)?,sequence,serde_json::to_string(&record)?])?;
    Ok(())
}

fn root(connection: &Connection) -> Result<()> {
    hold(connection, "grant-one", AUTHORITY_KIND, "operator", &AuthorityGrant {
        version: VERSION, source: definition().source, member: "member-one".into(), epoch: 1,
        predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None,
    })
}

fn commit(sequence: u64, predecessor: Option<&str>) -> CaptureCommit {
    CaptureCommit { version: VERSION, source: definition().source, epoch: 1, authority_grant: "grant-one".into(), sequence,
        predecessor: predecessor.map(str::to_owned), facts: Vec::new(), checkpoint: None, coverage: Vec::new(), observed_at_ms: 500 }
}

fn merge(id: &str, at: u64, supersedes: Option<String>) -> ObservedFact {
    ObservedFact { version: VERSION, source: definition().source, observed_at_ms: 500, supersedes,
        fact: FactImage::Merge(MergeFact { pull_request_id: id.into(), repository_id: "repo".into(), merged_at_ms: at }) }
}

fn builder() -> SnapshotBuilder {
    SnapshotBuilder::new(Projection {
        source: definition().source, revision: "head".into(), frontier_ms: 1_000,
        roster: Roster { revision: "roster".into(), members: vec![RosterMember { repository_id: "repo".into(), effective_from_ms: 0, effective_until_ms: None }] },
        pull_requests: Vec::new(), lifecycle: Vec::new(), merges: Vec::new(), coverage: Vec::new(), ready_cycles: BTreeMap::new(),
    })
}

#[test]
fn shared_capture_statements_contribute_to_sqlite_work_samples() -> Result<()> {
    use smallclaims::sqlite::{histogram, work};

    let state = tempfile::tempdir()?;
    let plugin = GithubPluginStore::shared(state.path())?;
    let connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
    migrate(&connection)?;
    histogram::take_on_this_thread();
    let before = work::total();
    let scope = work::SqliteWorkScope::start();
    assert!(held::<AuthorityGrant>(&connection, "absent-grant", AUTHORITY_KIND)?.is_none());
    let scoped = scope.finish();
    let sample = work::total() - before;
    let shapes = histogram::take_on_this_thread();
    assert_eq!(scoped.statements, 1, "the real plugin connection must count capture SQL");
    assert!(scoped.vm_steps > 0, "capture work must include SQLite VM instructions");
    assert!(sample.statements >= scoped.statements && sample.vm_steps >= scoped.vm_steps,
        "capture SQL must contribute to the same process-wide samples as daemon_cost");
    assert!(shapes.iter().any(|(sql, shape)| sql.contains("capture_v1_claims") && shape.count == 1),
        "the measured statement must read a real capture table: {shapes:?}");
    Ok(())
}

#[test]
fn dependencies_replay_out_of_order_without_partial_publication() -> Result<()> {
    let mut db = database()?;
    root(&db)?;
    let fact = merge("pr", 100, None);
    let mut first = commit(1, None);
    first.facts.push(FactReference { claim_id: "fact".into(), digest: fact.digest()? });
    hold(&db, "second", COMMIT_KIND, "member-one", &commit(2, Some("first")))?;
    assert!(!advance(&mut db, &definition())?);
    hold(&db, "first", COMMIT_KIND, "member-one", &first)?;
    assert!(!advance(&mut db, &definition())?);
    let key = projection_key(&definition())?;
    assert_eq!(current_head(&db, &key)?, (0, None));
    hold(&db, "fact", FACT_KIND, "observer", &fact)?;
    assert!(advance(&mut db, &definition())?);
    assert_eq!(current_head(&db, &key)?, (2, Some("second".into())));
    assert!(advance(&mut db, &definition())?);
    assert_eq!(current_head(&db, &key)?.0, 2);
    Ok(())
}

#[test]
fn unauthorized_candidates_do_not_override_admitted_member() -> Result<()> {
    let mut db = database()?;
    root(&db)?;
    hold(&db, "rogue", COMMIT_KIND, "other-member", &commit(1, None))?;
    hold(&db, "good", COMMIT_KIND, "member-one", &commit(1, None))?;
    assert!(advance(&mut db, &definition())?);
    assert_eq!(current_head(&db, &projection_key(&definition())?)?.1.as_deref(), Some("good"));
    Ok(())
}

#[test]
fn later_unadmitted_candidates_do_not_make_projection_pending() -> Result<()> {
    let mut db = database()?;
    root(&db)?;
    for index in 0..PAGE + 1 {
        hold(&db, &format!("rogue-{index}"), COMMIT_KIND, "other-member", &commit(3, Some("missing")))?;
    }
    let mut wrong_grant = commit(4, Some("missing"));
    wrong_grant.authority_grant = "unselected".into();
    hold(&db, "wrong-grant", COMMIT_KIND, "member-one", &wrong_grant)?;
    let mut wrong_epoch = commit(5, Some("missing"));
    wrong_epoch.epoch = 2;
    hold(&db, "wrong-epoch", COMMIT_KIND, "member-one", &wrong_epoch)?;
    assert!(advance(&mut db, &definition())?);
    assert_eq!(current_head(&db, &projection_key(&definition())?)?, (0, None));
    Ok(())
}

#[test]
fn admitted_forks_fail_before_and_after_application() -> Result<()> {
    let mut db = database()?;
    root(&db)?;
    hold(&db, "first", COMMIT_KIND, "member-one", &commit(1, None))?;
    assert!(advance(&mut db, &definition())?);
    hold(&db, "fork", COMMIT_KIND, "member-one", &commit(1, None))?;
    let key = projection_key(&definition())?;
    db.execute(/* sqlite-large-value: allow-for-now lv-h5478e08a3895 owner=fractal_reconcile::reconcile::github_capture_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO capture_v1_conflicts VALUES(?1,'fork')", [&key])?;
    assert!(advance(&mut db, &definition()).is_err());
    // The conflict persists: an existing cached snapshot cannot become trusted next time.
    assert!(advance(&mut db, &definition()).is_err());
    let mut fresh = database()?;
    root(&fresh)?;
    hold(&fresh, "first", COMMIT_KIND, "member-one", &commit(1, None))?;
    hold(&fresh, "fork", COMMIT_KIND, "member-one", &commit(1, None))?;
    assert!(advance(&mut fresh, &definition()).is_err());
    assert_eq!(current_head(&fresh, &key)?.0, 0);
    Ok(())
}

#[test]
fn fenced_successor_rebuilds_and_rejects_stale_epoch_tail() -> Result<()> {
    let mut db = database()?;
    root(&db)?;
    hold(&db, "first", COMMIT_KIND, "member-one", &commit(1, None))?;
    hold(&db, "stale", COMMIT_KIND, "member-one", &commit(2, Some("first")))?;
    hold(&db, "stop", FENCED_KIND, "member-one", &FenceReceipt {
        version: VERSION, source: definition().source, member: "member-one".into(), epoch: 1,
        authority_grant: "grant-one".into(), final_commit: Some("first".into()), stopped_at_ms: 600,
    })?;
    hold(&db, "grant-two", AUTHORITY_KIND, "operator", &AuthorityGrant {
        version: VERSION, source: definition().source, member: "member-two".into(), epoch: 2,
        predecessor_grant: Some("grant-one".into()), final_predecessor_commit: Some("first".into()), fencing_receipt: Some("stop".into()),
    })?;
    let mut next = definition();
    next.authority_grant = Some("grant-two".into());
    let mut second = commit(2, Some("first"));
    second.epoch = 2;
    second.authority_grant = "grant-two".into();
    hold(&db, "second", COMMIT_KIND, "member-two", &second)?;
    assert!(advance(&mut db, &next)?);
    assert_eq!(current_head(&db, &projection_key(&next)?)?.1.as_deref(), Some("second"));
    assert_ne!(projection_key(&definition())?, projection_key(&next)?);
    Ok(())
}

#[test]
fn missing_or_forged_fence_never_admits_successor() -> Result<()> {
    let db = database()?;
    root(&db)?;
    let mut next = definition();
    next.authority_grant = Some("grant-two".into());
    hold(&db, "grant-two", AUTHORITY_KIND, "operator", &AuthorityGrant {
        version: VERSION, source: definition().source, member: "member-two".into(), epoch: 2,
        predecessor_grant: Some("grant-one".into()), final_predecessor_commit: None, fencing_receipt: Some("stop".into()),
    })?;
    assert!(authority_chain(&db, &next)?.is_none());
    hold(&db, "stop", FENCED_KIND, "other-member", &FenceReceipt {
        version: VERSION, source: definition().source, member: "member-one".into(), epoch: 1,
        authority_grant: "grant-one".into(), final_commit: None, stopped_at_ms: 600,
    })?;
    assert!(authority_chain(&db, &next).is_err());
    Ok(())
}

#[test]
fn corrections_retractions_and_reinstatement_replace_old_windows() -> Result<()> {
    let db = database()?;
    let original = merge("pr", 100, None);
    apply_fact(&db, "source", 1, "original", &original, &original.digest()?)?;
    let corrected = merge("pr", 200, Some(original.digest()?));
    apply_fact(&db, "source", 2, "corrected", &corrected, &corrected.digest()?)?;
    let mut old = builder();
    assert!(old.advance(&db, "source", 0, 150)?);
    assert!(old.projection.merges.is_empty());
    let retraction = ObservedFact { version: VERSION, source: definition().source, observed_at_ms: 600,
        supersedes: Some(corrected.digest()?), fact: FactImage::Retraction(Retraction { family: FactFamily::Merge, repository_id: "repo".into(), id: "pr".into(), reason: "upstream correction".into() }) };
    apply_fact(&db, "source", 3, "retracted", &retraction, &retraction.digest()?)?;
    let mut current = builder();
    assert!(current.advance(&db, "source", 0, 300)?);
    assert!(current.projection.merges.is_empty());
    assert!(apply_fact(&db, "source", 4, "bad", &original, &original.digest()?).is_err());
    let restored = merge("pr", 250, Some(retraction.digest()?));
    apply_fact(&db, "source", 4, "restored", &restored, &restored.digest()?)?;
    let mut current = builder();
    assert!(current.advance(&db, "source", 0, 300)?);
    assert_eq!(current.projection.merges[0].merged_at_ms, 250);
    Ok(())
}

#[test]
fn paging_crosses_old_cliff_with_equal_timestamps_and_exclusive_end() -> Result<()> {
    let mut db = database()?;
    let tx = db.transaction()?;
    for index in 0..20_001 {
        let fact = merge(&format!("pr-{index:05}"), 100, None);
        apply_fact(&tx, "source", 1, "fact", &fact, "digest")?;
    }
    let excluded = merge("end", 200, None);
    apply_fact(&tx, "source", 1, "end", &excluded, "digest")?;
    tx.commit()?;
    let mut snapshot = builder();
    let mut calls = 0;
    loop {
        let before = snapshot.projection.merges.len();
        let complete = snapshot.advance(&db, "source", 100, 200)?;
        assert!(snapshot.projection.merges.len() - before <= SNAPSHOT_PAGE);
        calls += 1;
        if complete { break; }
        assert!(calls < 200);
    }
    assert_eq!(snapshot.projection.merges.len(), 20_001);
    assert!(calls > 1);
    assert_eq!(snapshot.projection.ready_cycles.len(), 20_001);
    Ok(())
}

#[test]
fn pinned_wal_read_survives_correction_between_pages() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("projection.sqlite3");
    let db = Connection::open(&path)?;
    db.execute_batch("PRAGMA journal_mode=WAL")?;
    migrate(&db)?;
    for index in 0..SNAPSHOT_PAGE + 1 {
        let fact = merge(&format!("pr-{index:05}"), 100, None);
        apply_fact(&db, "source", 1, "first", &fact, &fact.digest()?)?;
    }
    let reader = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-hc21aef8d0305 owner=fractal_reconcile::reconcile::github_capture_tests TODO=trace-statement-and-value-representation */ "BEGIN DEFERRED")?;
    let mut snapshot = builder();
    assert!(!snapshot.advance(&reader, "source", 0, 200)?);
    let original = merge(&format!("pr-{:05}", SNAPSHOT_PAGE), 100, None);
    let corrected = merge(&format!("pr-{:05}", SNAPSHOT_PAGE), 300, Some(original.digest()?));
    apply_fact(&db, "source", 2, "corrected", &corrected, &corrected.digest()?)?;
    assert!(snapshot.advance(&reader, "source", 0, 200)?);
    assert_eq!(snapshot.projection.merges.len(), SNAPSHOT_PAGE + 1);
    assert!(snapshot.projection.merges.iter().all(|fact| fact.merged_at_ms == 100));
    drop(reader);
    let mut fresh = builder();
    while !fresh.advance(&db, "source", 0, 200)? {}
    assert_eq!(fresh.projection.merges.len(), SNAPSHOT_PAGE);
    Ok(())
}

#[test]
fn final_ready_proof_uses_last_draft_and_corrected_lifecycle() -> Result<()> {
    let db = database()?;
    let image = merge("pr", 500, None);
    apply_fact(&db, "source", 1, "merge", &image, &image.digest()?)?;
    let pr = ObservedFact {
        version: VERSION, source: definition().source, observed_at_ms: 600, supersedes: None,
        fact: FactImage::PullRequest(PullRequestFact {
            id: "pr".into(), repository_id: "repo".into(), number: 1, author_login: None,
            state: PullRequestState::Merged, is_draft: false, created_as_draft: Some(false),
            created_at_ms: 10, closed_at_ms: Some(500), merged_at_ms: Some(500),
        }),
    };
    apply_fact(&db, "source", 1, "pr", &pr, &pr.digest()?)?;
    for (id, kind, at) in [("ready-one", LifecycleKind::Ready, 100), ("draft", LifecycleKind::Draft, 200), ("ready-two", LifecycleKind::Ready, 300)] {
        let event = ObservedFact {
            version: VERSION, source: definition().source, observed_at_ms: 600, supersedes: None,
            fact: FactImage::Lifecycle(LifecycleFact { id: id.into(), repository_id: "repo".into(), pull_request_id: "pr".into(), kind, occurred_at_ms: at }),
        };
        apply_fact(&db, "source", 1, id, &event, &event.digest()?)?;
    }
    let mut snapshot = builder();
    assert!(snapshot.advance(&db, "source", 0, 600)?);
    let identity = ("repo".to_owned(), "pr".to_owned());
    assert_eq!(snapshot.projection.ready_cycles.get(&identity), Some(&Some(300)));
    let old = ObservedFact {
        version: VERSION, source: definition().source, observed_at_ms: 600, supersedes: None,
        fact: FactImage::Lifecycle(LifecycleFact { id: "ready-two".into(), repository_id: "repo".into(), pull_request_id: "pr".into(), kind: LifecycleKind::Ready, occurred_at_ms: 300 }),
    };
    let retracted = ObservedFact {
        version: VERSION, source: definition().source, observed_at_ms: 700, supersedes: Some(old.digest()?),
        fact: FactImage::Retraction(Retraction { family: FactFamily::Lifecycle, repository_id: "repo".into(), id: "ready-two".into(), reason: "upstream correction".into() }),
    };
    apply_fact(&db, "source", 2, "retracted-ready", &retracted, &retracted.digest()?)?;
    let mut snapshot = builder();
    assert!(snapshot.advance(&db, "source", 0, 600)?);
    assert_eq!(snapshot.projection.ready_cycles.get(&identity), Some(&None));
    Ok(())
}

#[test]
fn empty_repository_probes_share_the_page_budget() -> Result<()> {
    let db = database()?;
    let mut snapshot = builder();
    snapshot.repositories = (0..SNAPSHOT_PAGE + 1).map(|index| format!("repo-{index}")).collect();
    assert!(!snapshot.advance(&db, "source", 0, 1_000)?);
    assert_eq!(snapshot.repository, SNAPSHOT_PAGE);
    assert!(snapshot.advance(&db, "source", 0, 1_000)?);
    Ok(())
}

#[test]
fn indicator_horizons_ignore_unrequired_families_and_do_not_pin_to_retired_members() {
    let roster = Roster { revision: "scope".into(), members: vec![
        RosterMember { repository_id: "active".into(), effective_from_ms: 100, effective_until_ms: None },
        RosterMember { repository_id: "retired".into(), effective_from_ms: 0, effective_until_ms: Some(100) },
    ] };
    let record = |repository: &str, family, through_ms| Coverage {
        repository_id: repository.into(), family, complete: true, authoritative: true,
        history_from_ms: 0, through_ms, error: None,
    };
    let records = vec![
        record("active", FactFamily::PullRequest, 1_000),
        record("active", FactFamily::Merge, 900),
        record("active", FactFamily::Lifecycle, 500),
        record("retired", FactFamily::Merge, 50),
    ];
    let merged = [FactFamily::PullRequest, FactFamily::Merge];
    assert_eq!(measurement_horizon(&records, &roster, &merged), Some(1_000));
    assert_eq!(measurement_horizon(&records[2..3], &roster, &merged), None);
    let without_lifecycle = &records[..2];
    assert_eq!(measurement_horizon(without_lifecycle, &roster, &merged), Some(1_000));
    let ready = [FactFamily::PullRequest, FactFamily::Merge, FactFamily::Lifecycle];
    assert_eq!(measurement_horizon(without_lifecycle, &roster, &ready), Some(1_000));
    // A retired member's newer upstream scan cannot move the horizon past its membership.
    assert_eq!(measurement_horizon(&[record("retired", FactFamily::Merge, 2_000)], &roster, &merged), Some(100));
}
