use super::*;

const LOCATOR: &str = "acme/garden";

fn fixture() -> (Store, Connection, SourceDefinition, AuthorityGrant) {
    let store = Store::open_memory("node").unwrap();
    store.bind_fleet("018f6f0d-4a5d-7b8c-9d0e-123456789abc").unwrap();
    let source = Source { host: "github.com".into(), id: "example".into(), generation: 1 };
    let grant = AuthorityGrant { version: VERSION, source: source.clone(), member: "node".into(), epoch: 1,
        predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None };
    let claim = append_record(&store, "github-source/example".into(), AUTHORITY_KIND, &grant, "example-grant".into()).unwrap();
    let definition = SourceDefinition { source, authority_grant: claim.id, repositories: vec![LOCATOR.into()],
        enabled: true, checkpoint_interval_ms: 60_000, replicate_private: false };
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&connection).unwrap();
    assert!(ready(&store, &mut connection, &definition).unwrap());
    (store, connection, definition, grant)
}

fn page() -> AcquisitionPage {
    let repository = RepositoryFact { id: "R_example".into(), locator: LOCATOR.into(), is_private: false };
    AcquisitionPage { facts: vec![FactImage::Repository(repository.clone())],
        checkpoint: AcquisitionCheckpoint { repository: Some(repository), checked_ms: Some(100), full_ms: Some(100), ..Default::default() },
        receipt: None, coverage: vec![Coverage { repository_id: "R_example".into(), family: FactFamily::Repository,
            complete: true, authoritative: true, history_from_ms: 0, through_ms: 100, error: None }],
        status: AcquisitionStatus::Complete, error: None }
}

fn cached_work(definition: &SourceDefinition, checkpoint: AcquisitionCheckpoint) -> Work {
    Work { grant: definition.authority_grant.clone(), acquisitions: BTreeMap::from([(LOCATOR.into(),
        CaptureAcquisition::resume(RepositoryTarget { locator: LOCATOR.into(), node_id: Some("R_example".into()) }, checkpoint, 100))]) }
}

#[test]
fn api_identity_accepts_only_the_established_public_origin() {
    let (_, _, definition, _) = fixture();
    for base in ["https://api.github.com", "https://api.github.com/", "https://API.GITHUB.COM:443"] {
        validate_api_identity(&definition.source, base).unwrap();
    }
    for base in ["not a URL", "http://api.github.com", "https://api.github.com:444", "https://github.com",
        "https://api.github.com.example.com", "https://api.github.com@evil.example", "https://user@api.github.com",
        "https://api.github.com/path", "https://api.github.com?route=other", "https://api.github.com/#fragment"] {
        assert!(validate_api_identity(&definition.source, base).is_err(), "{base}");
    }
    let mut enterprise = definition.source;
    enterprise.host = "git.example.com".into();
    for base in ["https://git.example.com", "https://git.example.com/api/v3", "https://api.git.example.com", "https://api.github.com"] {
        assert!(validate_api_identity(&enterprise, base).is_err(), "{base}");
    }
}

#[test]
fn publication_failure_discards_advanced_acquisition_and_replays_durable_frontier() {
    let (store, mut connection, definition, grant) = fixture();
    let page = page();
    let mut work = cached_work(&definition, page.checkpoint.clone());
    // Fail the cache transaction after facts and coverage are durably claimed, at checkpoint application.
    connection.execute_batch("CREATE TRIGGER fail_checkpoint BEFORE INSERT ON capture_v1_checkpoints BEGIN SELECT RAISE(ABORT,'simulated crash'); END;").unwrap();
    let failure = publish_page(&mut work, LOCATOR, &page,
        || publish(&store, &mut connection, &definition, &grant, &page, 100));
    assert!(failure.is_err());
    assert!(!work.acquisitions.contains_key(LOCATOR));
    assert!(checkpoint(&connection, &definition, "R_example").unwrap().is_none());
    assert!(image_revision(&connection, &definition, &page.facts[0]).unwrap().is_some());
    // A new process/cache reconstructs the checkpoint commit from claims, not the lost cursor.
    let mut rebuilt = Connection::open_in_memory().unwrap();
    migrate(&rebuilt).unwrap();
    assert!(ready(&store, &mut rebuilt, &definition).unwrap());
    let restored = checkpoint(&rebuilt, &definition, "R_example").unwrap().unwrap();
    let cursor: AcquisitionCheckpoint = serde_json::from_str(restored.cursor.as_deref().unwrap()).unwrap();
    assert_eq!(cursor.checked_ms, Some(100));
    let head = current_head(&rebuilt, &projection_key(&definition).unwrap()).unwrap();
    assert_eq!(head.0, 3);
    // Retrying an identical normalized page writes no duplicate fact revision or coverage.
    publish(&store, &mut rebuilt, &definition, &grant, &page, 200).unwrap();
    assert_eq!(current_head(&rebuilt, &projection_key(&definition).unwrap()).unwrap().0, 4);
    let facts = store.claims_by_kinds_after(&[FACT_KIND], None, 0, 128).unwrap();
    assert_eq!(facts.claims.len(), 1);
    let committed: ObservedFact = serde_json::from_value(facts.claims[0].body["fields"]["record"].clone()).unwrap();
    assert_eq!(committed.observed_at_ms, 100);
}

#[test]
fn staging_crash_reuses_original_fact_claim_before_first_commit() {
    let (store, mut connection, definition, grant) = fixture();
    let page = page();
    let fact = ObservedFact { version: VERSION, source: definition.source.clone(), observed_at_ms: 50,
        supersedes: None, fact: page.facts[0].clone() };
    let digest = fact.digest().unwrap();
    let staged = append_record(&store, fact.subject().unwrap(), FACT_KIND, &fact, format!("github-fact:{digest}")).unwrap();
    assert!(ready(&store, &mut connection, &definition).unwrap());
    assert!(image_revision(&connection, &definition, &page.facts[0]).unwrap().is_none());
    publish(&store, &mut connection, &definition, &grant, &page, 100).unwrap();
    let facts = store.claims_by_kinds_after(&[FACT_KIND], None, 0, 128).unwrap();
    assert_eq!(facts.claims.len(), 1);
    assert_eq!(facts.claims[0].id, staged.id);
    assert!(checkpoint(&connection, &definition, "R_example").unwrap().unwrap().pass_complete);
}

#[test]
fn publication_rejection_before_any_claim_discards_cursor() {
    let (store, mut connection, definition, grant) = fixture();
    let mut page = page();
    page.checkpoint.repository.as_mut().unwrap().is_private = true;
    let mut work = cached_work(&definition, page.checkpoint.clone());
    assert!(publish_page(&mut work, LOCATOR, &page,
        || publish(&store, &mut connection, &definition, &grant, &page, 100)).is_err());
    assert!(work.acquisitions.is_empty());
    assert_eq!(current_head(&connection, &projection_key(&definition).unwrap()).unwrap().0, 0);
    assert!(store.claims_by_kinds_after(&[FACT_KIND, COMMIT_KIND], None, 0, 128).unwrap().claims.is_empty());
}

#[test]
fn writer_requires_exact_grant_source_member_and_fence_identity() {
    let (store, mut connection, definition, grant) = fixture();
    writer_grant(&connection, &definition, "node").unwrap();
    assert!(writer_grant(&connection, &definition, "other").is_err());
    let mut changed = definition.clone();
    changed.source.generation += 1;
    assert!(writer_grant(&connection, &changed, "node").is_err());
    changed = definition.clone();
    changed.authority_grant = "missing".into();
    assert!(writer_grant(&connection, &changed, "node").is_err());
    let exact = FenceReceipt { version: VERSION, source: definition.source.clone(), member: "node".into(),
        epoch: grant.epoch, authority_grant: definition.authority_grant.clone(), final_commit: None, stopped_at_ms: 100 };
    for index in 0..5 {
        let mut receipt = exact.clone();
        match index {
            0 => receipt.authority_grant = "another-grant".into(),
            1 => receipt.epoch += 1,
            2 => receipt.member = "another-member".into(),
            3 => receipt.source.generation += 1,
            _ => receipt.source.host = "git.example.com".into(),
        }
        append_record(&store, "github-source/example".into(), FENCED_KIND, &receipt, format!("unrelated-fence-{index}")).unwrap();
        assert!(ready(&store, &mut connection, &definition).unwrap());
        writer_grant(&connection, &definition, "node").unwrap();
    }
    append_record(&store, "github-source/example".into(), FENCED_KIND, &exact, "exact-fence".into()).unwrap();
    assert!(ready(&store, &mut connection, &definition).unwrap());
    assert!(writer_grant(&connection, &definition, "node").is_err());
    let mut work = cached_work(&definition, page().checkpoint);
    assert!(publish_page(&mut work, LOCATOR, &page(), || {
        writer_grant(&connection, &definition, "node")?;
        publish(&store, &mut connection, &definition, &grant, &page(), 100)
    }).is_err());
    assert!(work.acquisitions.is_empty());
    assert_eq!(current_head(&connection, &projection_key(&definition).unwrap()).unwrap().0, 0);
}

#[test]
fn stopped_receipt_retry_returns_original_claim_and_requires_exact_frontier() {
    let (store, mut connection, definition, grant) = fixture();
    publish(&store, &mut connection, &definition, &grant, &page(), 100).unwrap();
    let (_, frontier) = current_head(&connection, &projection_key(&definition).unwrap()).unwrap();
    assert!(existing_fence(&store, &connection, &definition, &grant, frontier.as_deref()).unwrap().is_none());
    let receipt = FenceReceipt { version: VERSION, source: definition.source.clone(), member: "node".into(),
        epoch: grant.epoch, authority_grant: definition.authority_grant.clone(),
        final_commit: frontier.clone(), stopped_at_ms: 100 };
    let original = append_record(&store, "github-source/example".into(), FENCED_KIND, &receipt, "stopped-example".into()).unwrap();
    assert!(ready(&store, &mut connection, &definition).unwrap());
    for _ in 0..2 {
        let retry = existing_fence(&store, &connection, &definition, &grant, frontier.as_deref()).unwrap().unwrap();
        assert_eq!(retry.id, original.id);
        assert_eq!(retry.body, original.body);
    }
    assert!(existing_fence(&store, &connection, &definition, &grant, None).is_err());
    let mut other_grant = definition.clone();
    other_grant.authority_grant = "another-root".into();
    assert!(existing_fence(&store, &connection, &other_grant, &grant, frontier.as_deref()).unwrap().is_none());
    let receipts = store.claims_by_kinds_after(&[FENCED_KIND], None, 0, 128).unwrap();
    assert_eq!(receipts.claims.len(), 1);
}

#[test]
fn continuing_publication_retains_acquisition_only_after_checkpoint_is_committed() {
    let (store, mut connection, definition, grant) = fixture();
    let mut page = page();
    page.status = AcquisitionStatus::Continue;
    page.coverage[0].complete = false;
    page.coverage[0].through_ms = 0;
    let mut work = cached_work(&definition, page.checkpoint.clone());
    publish_page(&mut work, LOCATOR, &page,
        || publish(&store, &mut connection, &definition, &grant, &page, 100)).unwrap();
    assert!(work.acquisitions.contains_key(LOCATOR));
    assert!(!checkpoint(&connection, &definition, "R_example").unwrap().unwrap().pass_complete);
    let commits = store.claims_by_kinds_after(&[COMMIT_KIND], None, 0, 128).unwrap();
    let records: Vec<CaptureCommit> = commits.claims.iter()
        .map(|claim| serde_json::from_value(claim.body["fields"]["record"].clone()).unwrap()).collect();
    assert_eq!(records.len(), 3);
    assert!(!records[0].coverage[0].complete);
    assert_eq!(records[1].facts.len(), 1);
    assert!(records[2].checkpoint.is_some());
}

#[tokio::test]
async fn fence_operation_stops_cached_worker_and_retries_exact_receipt() {
    let (store, _, definition, _) = fixture();
    let state = tempfile::tempdir().unwrap();
    let source = format!(r#"version 2
github-source "example" {{
    host "github.com"
    generation 1
    authority-grant "{}"
    repository "acme/garden"
    enabled true
    checkpoint-every-ms 60000
    replicate-private false
}}
"#, definition.authority_grant);
    crate::reconcile::tests::apply_source(&store, &source, "declare-example");
    let handle = work_handle(state.path(), &definition.source).unwrap();
    *handle.lock().await = cached_work(&definition, page().checkpoint);
    assert!(fence_source_checked(&store, state.path(), "example", &definition.source, "stale-grant").await.is_err());
    assert!(handle.lock().await.acquisitions.contains_key(LOCATOR));
    let first = fence_source_checked(&store, state.path(), "example", &definition.source, &definition.authority_grant).await.unwrap();
    assert!(handle.lock().await.acquisitions.is_empty());
    let retry = fence_source(&store, state.path(), "example").await.unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(retry.body, first.body);
    let plugin = GithubPluginStore::shared(state.path()).unwrap();
    let connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(writer_grant(&connection, &definition, "node").is_err());
    let receipt: FenceReceipt = serde_json::from_value(first.body["fields"]["record"].clone()).unwrap();
    assert_eq!(receipt.authority_grant, definition.authority_grant);
    assert_eq!(receipt.source, definition.source);
    assert_eq!(receipt.epoch, 1);
    assert_eq!(receipt.member, "node");
    assert_eq!(receipt.final_commit, None);
}

#[test]
fn another_origin_cannot_fence_the_designated_member_by_naming_it() {
    let (store, mut connection, definition, grant) = fixture();
    let other = Store::open_memory("other").unwrap();
    other.bind_fleet("018f6f0d-4a5d-7b8c-9d0e-123456789abc").unwrap();
    let receipt = FenceReceipt { version: VERSION, source: definition.source.clone(), member: "node".into(),
        epoch: grant.epoch, authority_grant: definition.authority_grant.clone(), final_commit: None, stopped_at_ms: 100 };
    append_record(&other, "github-source/example".into(), FENCED_KIND, &receipt, "foreign-fence".into()).unwrap();
    store.import_replication("other", &other.export_replication(0).unwrap()).unwrap();
    assert!(ready(&store, &mut connection, &definition).unwrap());
    writer_grant(&connection, &definition, "node").unwrap();
    assert!(existing_fence(&store, &connection, &definition, &grant, None).unwrap().is_none());
}
