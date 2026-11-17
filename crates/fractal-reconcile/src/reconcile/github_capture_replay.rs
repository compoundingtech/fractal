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
        version: VERSION, source: definition.source, authority_grant: definition.authority_grant,
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
}
