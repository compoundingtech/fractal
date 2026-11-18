//! Versioned replay over admitted capture commits, never staged observations.
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::*;

/// Opaque wire cursors bind a finite snapshot to the exact source declaration and grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReplayCursor {
    version: u8,
    definition: String,
    after: u64,
    through: u64,
    frontier: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureReplayRecord {
    pub claim_id: String,
    pub commit: CaptureCommit,
    pub facts: Vec<ObservedFact>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureReplayPage {
    pub version: u8,
    pub source: Source,
    pub authority_grant: String,
    pub through_sequence: u64,
    pub frontier: Option<String>,
    pub records: Vec<CaptureReplayRecord>,
    /// Present on the first page only: this is coverage at `through_sequence`.
    /// Subsequent pages retain the same snapshot; clients retain this initial coverage.
    pub coverage: Option<Vec<Coverage>>,
    /// Always returned, including at the snapshot end. Reusing an exhausted cursor starts
    /// a new finite snapshot at the latest admitted frontier without replaying old rows.
    pub cursor: String,
    pub snapshot_complete: bool,
}

fn definition_digest(definition: &SourceDefinition) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(definition)?)))
}

fn decode_cursor(encoded: &str, definition: &str) -> Result<CaptureReplayCursor> {
    ensure!(encoded.len() <= 4096, "capture replay cursor exceeds its bound");
    let cursor: CaptureReplayCursor = serde_json::from_slice(&hex::decode(encoded)?)?;
    ensure!(cursor.version == VERSION && cursor.definition == definition,
        "capture replay cursor belongs to another source, generation, grant or scope");
    ensure!(cursor.after <= cursor.through && (cursor.through == 0) == cursor.frontier.is_none(),
        "capture replay cursor has an invalid frontier");
    Ok(cursor)
}

/// A page scans at most 128 admitted commits and their four bounded references each.
/// `None` means the rebuildable projection has not caught up; it is not empty coverage.
pub fn capture_replay(
    store: &Store,
    state_dir: &Path,
    source_id: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Option<CaptureReplayPage>> {
    ensure!((1..=PAGE).contains(&limit), "capture replay limit is 1 to {PAGE}");
    let definition = selected_source(store, source_id)?.context("source is not declared")?;
    let digest = definition_digest(&definition)?;
    let previous = cursor.map(|cursor| decode_cursor(cursor, &digest)).transpose()?;
    let plugin = GithubPluginStore::shared(state_dir)?;
    let mut connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
    if !ready(store, &mut connection, &definition)? { return Ok(None); }
    let key = projection_key(&definition)?;
    let (head_sequence, head_id) = current_head(&connection, &key)?;
    if let Some(previous) = &previous {
        ensure!(head_sequence >= previous.through, "capture replay snapshot frontier was invalidated");
        if let Some(frontier) = &previous.frontier {
            let retained: Option<String> = connection.query_row(
                "SELECT commit_id FROM capture_v1_history WHERE source=?1 AND sequence=?2",
                params![key, previous.through], |row| row.get(0),
            ).optional()?;
            ensure!(retained.as_ref() == Some(frontier), "capture replay snapshot frontier was replaced");
        }
    }
    let new_snapshot = previous.as_ref().is_none_or(|previous| previous.after == previous.through);
    let mut next = match previous {
        Some(previous) if !new_snapshot => previous,
        previous => CaptureReplayCursor {
            version: VERSION, definition: digest,
            after: previous.map_or(0, |previous| previous.after),
            through: head_sequence, frontier: head_id,
        },
    };
    let coverage = if new_snapshot {
        let mut statement = connection.prepare_cached(
            "SELECT record FROM capture_v1_coverage WHERE source=?1 ORDER BY repository_id,family LIMIT 513",
        )?;
        let records = statement.query_map([&key], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(records.len() <= 512, "capture coverage exceeds the declared source bound");
        Some(records.into_iter().map(|record| serde_json::from_str(&record))
            .collect::<serde_json::Result<Vec<Coverage>>>()?)
    } else { None };
    let commits = {
        let mut statement = connection.prepare_cached(
            "SELECT h.commit_id,c.record FROM capture_v1_history h JOIN capture_v1_claims c ON c.id=h.commit_id WHERE h.source=?1 AND h.sequence>?2 AND h.sequence<=?3 ORDER BY h.sequence LIMIT ?4",
        )?;
        statement.query_map(params![key,next.after,next.through,limit], |row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut records = Vec::with_capacity(commits.len());
    for (claim_id, record) in commits {
        let commit: CaptureCommit = serde_json::from_str(&record)?;
        ensure!(commit.sequence == next.after + 1, "capture replay has a missing admitted sequence");
        let mut facts = Vec::with_capacity(commit.facts.len());
        for reference in &commit.facts {
            let (fact, _) = held::<ObservedFact>(&connection, &reference.claim_id, FACT_KIND)?
                .context("admitted capture lost its fact dependency")?;
            ensure!(fact.source == definition.source && fact.digest()? == reference.digest,
                "admitted replay fact has another source or digest");
            facts.push(fact);
        }
        next.after = commit.sequence;
        records.push(CaptureReplayRecord { claim_id, commit, facts });
    }
    ensure!(!records.is_empty() || next.after == next.through, "capture replay frontier has no admitted rows");
    Ok(Some(CaptureReplayPage {
        version: VERSION, source: definition.source,
        authority_grant: definition.authority_grant.context("source has no selected authority grant")?,
        through_sequence: next.through, frontier: next.frontier.clone(), records, coverage,
        cursor: hex::encode(serde_json::to_vec(&next)?), snapshot_complete: next.after == next.through,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_binds_version_scope_and_finite_frontier() {
        let cursor = CaptureReplayCursor {
            version: VERSION, definition: "definition-a".into(), after: 2,
            through: 3, frontier: Some("commit-3".into()),
        };
        let encoded = hex::encode(serde_json::to_vec(&cursor).unwrap());
        assert_eq!(decode_cursor(&encoded, "definition-a").unwrap(), cursor);
        assert!(decode_cursor(&encoded, "definition-b").is_err());
        let invalid = CaptureReplayCursor { after: 4, ..cursor.clone() };
        assert!(decode_cursor(&hex::encode(serde_json::to_vec(&invalid).unwrap()), "definition-a").is_err());
        let invalid = CaptureReplayCursor { frontier: None, ..cursor };
        assert!(decode_cursor(&hex::encode(serde_json::to_vec(&invalid).unwrap()), "definition-a").is_err());
        assert!(decode_cursor(&"0".repeat(4097), "definition-a").is_err());
    }

    fn append(store: &Store, subject: String, kind: &str, record: &impl Serialize) -> crate::model::ClaimRecord {
        store.append_claim(&crate::model::ClaimInput {
            subject, kind: kind.into(), actor: None,
            fields: BTreeMap::from([("record".into(), serde_json::to_value(record).unwrap())]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    #[test]
    fn replay_and_plugin_remain_pending_with_a_known_later_capture() {
        let store = Store::open_memory("node").unwrap();
        store.bind_fleet("018f6f0d-4a5d-7b8c-9d0e-123456789abc").unwrap();
        let state = tempfile::tempdir().unwrap();
        let source = Source { host: "github.com".into(), id: "example".into(), generation: 1 };
        let grant = AuthorityGrant { version: VERSION, source: source.clone(), member: "node".into(),
            epoch: 1, predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None };
        let grant = append(&store, "github-source/example".into(), AUTHORITY_KIND, &grant);
        crate::reconcile::tests::apply_source(&store, &format!(r#"version 2
github-source "example" {{
    host "github.com"
    generation 1
    authority-grant "{}"
    repository "acme/garden"
    enabled #true
    checkpoint-every-ms 60000
    replicate-private #false
}}"#, grant.id), "declare-delayed-replay");
        // An admitted writer can publish a malformed/out-of-order source sequence even
        // though private replication preserves its own independent batch sequence.
        let commit = CaptureCommit { version: VERSION, source, epoch: 1,
            authority_grant: grant.id, sequence: 2, predecessor: Some("missing-first-capture".into()),
            facts: vec![], checkpoint: None, coverage: vec![], observed_at_ms: 100 };
        append(&store, commit.subject().unwrap(), COMMIT_KIND, &commit);
        assert!(capture_replay(&store, state.path(), "example", None, 128).unwrap().is_none());
        assert!(capture_replay(&store, state.path(), "example", None, 128).unwrap().is_none());
        let pulls = super::super::repository_pull_requests(
            &store, state.path(), "acme/garden", None, None, 128,
        ).unwrap().unwrap();
        assert!(!pulls.complete);
        assert!(pulls.pulls.is_empty());
    }

    #[test]
    fn replay_pins_coverage_and_frontier_and_ignores_staged_facts() {
        let store = Store::open_memory("node").unwrap();
        store.bind_fleet("018f6f0d-4a5d-7b8c-9d0e-123456789abc").unwrap();
        let state = tempfile::tempdir().unwrap();
        let source = Source { host: "github.com".into(), id: "example".into(), generation: 1 };
        let grant = AuthorityGrant { version: VERSION, source: source.clone(), member: "node".into(),
            epoch: 1, predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None };
        let grant = append(&store, "github-source/example".into(), AUTHORITY_KIND, &grant);
        crate::reconcile::tests::apply_source(&store, &format!(r#"version 2
github-source "example" {{
    host "github.com"
    generation 1
    authority-grant "{}"
    repository "acme/garden"
    enabled #true
    checkpoint-every-ms 60000
    replicate-private #false
}}"#, grant.id), "declare-replay");
        let fact = ObservedFact { version: VERSION, source: source.clone(), observed_at_ms: 100,
            supersedes: None, fact: FactImage::Repository(RepositoryFact {
                id: "R_example".into(), locator: "acme/garden".into(), is_private: false,
            }) };
        let staged = append(&store, fact.subject().unwrap(), FACT_KIND, &fact);
        let second_fact = ObservedFact { fact: FactImage::Repository(RepositoryFact {
            id: "R_second".into(), locator: "acme/orchard".into(), is_private: false,
        }), ..fact.clone() };
        let second_staged = append(&store, second_fact.subject().unwrap(), FACT_KIND, &second_fact);
        let empty = capture_replay(&store, state.path(), "example", None, 1).unwrap().unwrap();
        assert!(empty.records.is_empty());
        assert!(empty.snapshot_complete);
        let mut commit = CaptureCommit { version: VERSION, source, epoch: 1,
            authority_grant: grant.id, sequence: 1, predecessor: None,
            // Replay preserves the commit's reference order, not staging/store order.
            facts: vec![
                FactReference { claim_id: second_staged.id, digest: second_fact.digest().unwrap() },
                FactReference { claim_id: staged.id, digest: fact.digest().unwrap() },
            ],
            checkpoint: None, coverage: vec![], observed_at_ms: 100 };
        let first = append(&store, commit.subject().unwrap(), COMMIT_KIND, &commit);
        commit.sequence = 2;
        commit.predecessor = Some(first.id);
        commit.facts.clear();
        commit.coverage = vec![Coverage { repository_id: "R_example".into(), family: FactFamily::Repository,
            complete: true, authoritative: true, history_from_ms: 0, through_ms: 100, error: None }];
        let second = append(&store, commit.subject().unwrap(), COMMIT_KIND, &commit);
        let page = capture_replay(&store, state.path(), "example", Some(&empty.cursor), 1).unwrap().unwrap();
        assert_eq!(page.through_sequence, 2);
        assert_eq!(page.coverage.as_ref().unwrap()[0].through_ms, 100);
        assert_eq!(page.records[0].facts, vec![second_fact, fact]);
        assert!(!page.snapshot_complete);
        commit.sequence = 3;
        commit.predecessor = Some(second.id);
        commit.coverage[0].through_ms = 200;
        append(&store, commit.subject().unwrap(), COMMIT_KIND, &commit);
        let continuation = capture_replay(&store, state.path(), "example", Some(&page.cursor), 1).unwrap().unwrap();
        assert_eq!(continuation.through_sequence, 2);
        assert_eq!(continuation.records[0].commit.sequence, 2);
        assert!(continuation.coverage.is_none());
        assert!(continuation.snapshot_complete);
        let next = capture_replay(&store, state.path(), "example", Some(&continuation.cursor), 1).unwrap().unwrap();
        assert_eq!(next.through_sequence, 3);
        assert_eq!(next.records[0].commit.sequence, 3);
        assert_eq!(next.coverage.unwrap()[0].through_ms, 200);
        assert!(next.snapshot_complete);
    }
}
