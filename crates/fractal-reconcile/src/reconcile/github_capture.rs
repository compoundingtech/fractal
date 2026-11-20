//! Durable GitHub capture: claims are authoritative; this plugin index is entirely rebuildable.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use anyhow::{Context as _, Result, ensure};
use fractal_schema::github_capture::{self as schema, *};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::store::Store;
use super::github_plugin_store::GithubPluginStore;

#[path = "github_capture_writer.rs"]
mod writer;
pub use writer::{GraphGitHubCaptureLedger, fence_source, fence_source_checked};

#[path = "github_capture_replay.rs"]
mod replay;
pub use replay::{capture_replay, CaptureReplayCursor, CaptureReplayPage};

#[path = "github_capture_plugin.rs"]
mod plugin_reads;
pub use plugin_reads::{repository_pull_requests, RepositoryPullRequests};

const PAGE: usize = 128;
// One snapshot continuation reads at most this many merges and their indexed ready proofs.
const SNAPSHOT_PAGE: usize = 128;
const KINDS: &[&str] = &[FACT_KIND, COMMIT_KIND, AUTHORITY_KIND, FENCED_KIND];

type SnapshotKey = (PathBuf, String, String, String);
static SNAPSHOTS: LazyLock<Mutex<SnapshotCache>> = LazyLock::new(Mutex::default);

// Completed reads retain only their shared projection, not a builder-sized map slot.
// Separate typed maps avoid boxing builders or allocating another projection copy.
#[derive(Default)]
struct SnapshotCache {
    building: BTreeMap<SnapshotKey, SnapshotBuilder>,
    complete: BTreeMap<SnapshotKey, MeasurementSnapshot>,
}

/// A coherent committed read and its actual half-open bounds, pinned during paged completion.
#[derive(Clone)]
pub struct MeasurementSnapshot {
    pub projection: Arc<Projection>,
    pub start_ms: u64,
    pub end_ms: u64,
}

struct SnapshotBuilder {
    reader: Option<Connection>,
    start_ms: u64,
    end_ms: u64,
    projection: Projection,
    repositories: Vec<String>,
    repository: usize,
    after: Option<(u64, String)>,
}

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h6b101dd45bbc owner=fractal_reconcile::reconcile::github_capture TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS capture_v1_meta(id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL);
        INSERT OR IGNORE INTO capture_v1_meta VALUES(1,0);
        CREATE TABLE IF NOT EXISTS capture_v1_claims(id TEXT PRIMARY KEY,kind TEXT NOT NULL,origin TEXT NOT NULL,source TEXT NOT NULL,sequence INTEGER,record TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS capture_v1_source_sequence ON capture_v1_claims(source,kind,sequence,id);
        CREATE INDEX IF NOT EXISTS capture_v1_admitted_sequence ON capture_v1_claims(source,kind,origin,json_extract(record,'$.authority_grant'),json_extract(record,'$.epoch'),sequence);
        CREATE TABLE IF NOT EXISTS capture_v1_heads(source TEXT PRIMARY KEY,grant_id TEXT NOT NULL,sequence INTEGER NOT NULL,commit_id TEXT,error TEXT);
        CREATE TABLE IF NOT EXISTS capture_v1_history(source TEXT NOT NULL,sequence INTEGER NOT NULL,commit_id TEXT NOT NULL,PRIMARY KEY(source,sequence));
        CREATE TABLE IF NOT EXISTS capture_v1_conflicts(source TEXT NOT NULL,claim_id TEXT NOT NULL,PRIMARY KEY(source,claim_id));
        CREATE TABLE IF NOT EXISTS capture_v1_entities(source TEXT NOT NULL,family TEXT NOT NULL,repository_id TEXT NOT NULL,entity_id TEXT NOT NULL,digest TEXT NOT NULL,claim_id TEXT NOT NULL,sequence INTEGER NOT NULL,occurred_ms INTEGER,pr_id TEXT,transition_kind TEXT,record TEXT NOT NULL,PRIMARY KEY(source,family,repository_id,entity_id));
        CREATE INDEX IF NOT EXISTS capture_v1_merge_window ON capture_v1_entities(source,family,repository_id,occurred_ms,entity_id);
        CREATE INDEX IF NOT EXISTS capture_v1_lifecycle ON capture_v1_entities(source,family,repository_id,pr_id,transition_kind,occurred_ms,entity_id);
        CREATE INDEX IF NOT EXISTS capture_v1_plugin_pulls ON capture_v1_entities(source,family,repository_id,json_extract(record,'$.image.state'),json_extract(record,'$.image.number') DESC);
        CREATE TABLE IF NOT EXISTS capture_v1_coverage(source TEXT NOT NULL,repository_id TEXT NOT NULL,family TEXT NOT NULL,sequence INTEGER NOT NULL,record TEXT NOT NULL,PRIMARY KEY(source,repository_id,family));
        CREATE TABLE IF NOT EXISTS capture_v1_checkpoints(source TEXT NOT NULL,repository_id TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(source,repository_id));
        CREATE TABLE IF NOT EXISTS capture_v1_receipts(source TEXT NOT NULL,repository_id TEXT NOT NULL,pr_id TEXT NOT NULL,updated_ms INTEGER NOT NULL,pass_ms INTEGER NOT NULL,PRIMARY KEY(source,repository_id,pr_id));")?;
    Ok(())
}

fn family(family: FactFamily) -> &'static str {
    match family { FactFamily::Repository => "repository", FactFamily::PullRequest => "pull_request", FactFamily::Lifecycle => "lifecycle", FactFamily::Merge => "merge" }
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T> { Ok(T::deserialize(value)?) }

fn selected_source(store: &Store, id: &str) -> Result<Option<SourceDefinition>> {
    let subject = if id.starts_with("github-source/") { id.to_owned() } else { format!("github-source/{id}") };
    store.desired_subject_with_writer(&subject)?.map(|(desired, _)| {
        ensure!(desired.kind == "github-source", "source reference is not a github-source declaration");
        schema::declarations::parse_source(&desired.desired).map_err(anyhow::Error::msg)
    }).transpose()
}

fn selected_roster(store: &Store, id: &str) -> Result<Option<Roster>> {
    let subject = if id.starts_with("github-scope/") { id.to_owned() } else { format!("github-scope/{id}") };
    store.desired_subject_with_writer(&subject)?.map(|(desired, _)| {
        ensure!(desired.kind == "github-scope", "roster reference is not a github-scope declaration");
        schema::declarations::parse_roster(&desired.desired).map_err(anyhow::Error::msg)
    }).transpose()
}

/// One bounded indexed claim page. The local cursor is only a rebuild bookmark, never source truth.
fn ingest(store: &Store, connection: &mut Connection) -> Result<bool> {
    let after: u64 = connection.query_row("SELECT cursor FROM capture_v1_meta WHERE id=1", [], |row| row.get(0))?;
    let page = store.claims_by_kinds_after(KINDS, None, after, PAGE)?;
    let transaction = connection.transaction()?;
    let mut through = after;
    for claim in &page.claims {
        through = through.max(claim.store_index);
        let record = claim.body.pointer("/fields/record").context("capture claim has no record")?;
        let (source, sequence) = match claim.kind.as_str() {
            FACT_KIND => (decode::<ObservedFact>(record)?.source, None),
            COMMIT_KIND => { let commit: CaptureCommit = decode(record)?; (commit.source, Some(commit.sequence)) }
            AUTHORITY_KIND => (decode::<AuthorityGrant>(record)?.source, None),
            FENCED_KIND => (decode::<FenceReceipt>(record)?.source, None),
            _ => unreachable!(),
        };
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h8b8be878a0a1 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO capture_v1_claims(id,kind,origin,source,sequence,record) VALUES(?1,?2,?3,?4,?5,?6)",
            params![claim.id,claim.kind,claim.origin,source_key(&source)?,sequence,serde_json::to_string(record)?])?;
        if let Some(sequence)=sequence {
            let source=source_key(&source)?;
            let mut statement=transaction.prepare_cached("SELECT source FROM capture_v1_heads WHERE source>=?1 AND source<?2 ORDER BY source LIMIT 129")?;
            let heads=statement.query_map(params![format!("{source}:"),format!("{source};")],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            ensure!(heads.len()<=128,"source projection namespace population exceeds bounded admission");
            for head in heads {
                let divergent:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM capture_v1_history WHERE source=?1 AND sequence=?2 AND commit_id<>?3)",params![head,sequence,claim.id],|row|row.get(0))?;
                if divergent {
                    transaction.execute(/* sqlite-large-value: allow-for-now lv-hd1bc48ebaa1b owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO capture_v1_conflicts(source,claim_id) VALUES(?1,?2)",params![head,claim.id])?;
                }
            }
        }
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h822e800d8a42 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "UPDATE capture_v1_meta SET cursor=?1 WHERE id=1", [through])?;
    transaction.commit()?;
    Ok(page.next_cursor.is_none())
}

fn held<T: DeserializeOwned>(connection: &Connection, id: &str, kind: &str) -> Result<Option<(T, String)>> {
    let value: Option<(String, String)> = connection.query_row("SELECT record,origin FROM capture_v1_claims WHERE id=?1 AND kind=?2", params![id,kind], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    value.map(|(record, origin)| Ok((serde_json::from_str(&record)?,origin))).transpose()
}

struct Epoch { grant_id: String, member: String, final_sequence: Option<u64> }

/// An admitted grant is an explicit chain with receipts from each stopped predecessor member.
fn authority_chain(connection: &Connection, definition: &SourceDefinition) -> Result<Option<BTreeMap<u64, Epoch>>> {
    let Some(mut id) = definition.authority_grant.clone() else { return Ok(None) };
    let mut epochs = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut cutoff = None;
    let mut successor_epoch = None;
    for _ in 0..32 {
        ensure!(seen.insert(id.clone()), "authority grant chain is cyclic");
        let Some((grant, _)) = held::<AuthorityGrant>(connection, &id, AUTHORITY_KIND)? else { return Ok(None) };
        ensure!(grant.source == definition.source, "authority grant is for another source generation");
        if let Some(successor) = successor_epoch { ensure!(grant.epoch.checked_add(1) == Some(successor), "authority epochs are not consecutive"); }
        epochs.insert(grant.epoch, Epoch { grant_id:id.clone(),member:grant.member.clone(),final_sequence:cutoff });
        let Some(predecessor) = grant.predecessor_grant else {
            ensure!(grant.epoch == 1, "root authority epoch must be one");
            return Ok(Some(epochs));
        };
        let receipt_id = grant.fencing_receipt.context("successor has no stopped-writer receipt")?;
        let Some((receipt, origin)) = held::<FenceReceipt>(connection, &receipt_id, FENCED_KIND)? else { return Ok(None) };
        ensure!(receipt.source == definition.source && receipt.authority_grant == predecessor
            && receipt.epoch.checked_add(1) == Some(grant.epoch) && receipt.member == origin
            && receipt.final_commit == grant.final_predecessor_commit,
            "fencing receipt does not bind the predecessor epoch and frontier");
        cutoff = match receipt.final_commit {
            Some(final_id) => {
                let Some((commit, commit_origin)) = held::<CaptureCommit>(connection, &final_id, COMMIT_KIND)? else { return Ok(None) };
                ensure!(commit.source == definition.source && commit.epoch == receipt.epoch
                    && commit.authority_grant == predecessor && commit_origin == receipt.member,
                    "fenced frontier is not a predecessor capture");
                Some(commit.sequence)
            }
            None => Some(0),
        };
        let Some((prior, _)) = held::<AuthorityGrant>(connection, &predecessor, AUTHORITY_KIND)? else { return Ok(None) };
        ensure!(prior.member == receipt.member, "fencing receipt was not emitted by the granted member");
        successor_epoch = Some(grant.epoch);
        id = predecessor;
    }
    anyhow::bail!("authority grant chain exceeds its bounded depth")
}

fn selected_authority(definition: &SourceDefinition) -> Result<&str> {
    definition.authority_grant.as_deref().context("source has no selected authority grant")
}

fn projection_key(definition: &SourceDefinition) -> Result<String> {
    Ok(format!("{}:{}",source_key(&definition.source)?,selected_authority(definition)?))
}

/// A changed authority gets a fresh rebuild namespace, never a synchronous history deletion.
fn prepare_source(connection: &mut Connection, definition: &SourceDefinition) -> Result<String> {
    let key = projection_key(definition)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h6a2ba291f8fd owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO capture_v1_heads(source,grant_id,sequence,commit_id,error) VALUES(?1,?2,0,NULL,NULL)", params![key,selected_authority(definition)?])?;
    Ok(key)
}

fn current_head(connection: &Connection, source: &str) -> Result<(u64, Option<String>)> {
    Ok(connection.query_row("SELECT sequence,commit_id FROM capture_v1_heads WHERE source=?1", [source], |row| Ok((row.get(0)?,row.get(1)?)))?)
}

fn apply_fact(connection: &Connection, source: &str, sequence: u64, claim_id: &str, fact: &ObservedFact, digest: &str) -> Result<()> {
    let (kind, repository, id) = fact.fact.identity();
    let previous: Option<String> = connection.query_row("SELECT digest FROM capture_v1_entities WHERE source=?1 AND family=?2 AND repository_id=?3 AND entity_id=?4", params![source,family(kind),repository,id], |row| row.get(0)).optional()?;
    if previous.as_deref() == Some(digest) { return Ok(()); }
    ensure!(fact.supersedes.as_deref() == previous.as_deref(), "fact revision does not supersede the current source image");
    let (occurred, pr_id, transition) = match &fact.fact {
        FactImage::Lifecycle(event) => (Some(event.occurred_at_ms), Some(event.pull_request_id.as_str()), Some(match event.kind { LifecycleKind::Ready => "ready", LifecycleKind::Draft => "draft" })),
        FactImage::Merge(merge) => (Some(merge.merged_at_ms), Some(merge.pull_request_id.as_str()), None),
        FactImage::PullRequest(pr) => (pr.merged_at_ms, Some(pr.id.as_str()), None),
        _ => (None, None, None),
    };
    // Keep retraction revisions so a later correction must supersede the tombstone, not absence.
    connection.execute(/* sqlite-large-value: allow-for-now lv-hdf91a9ae9fee owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO capture_v1_entities(source,family,repository_id,entity_id,digest,claim_id,sequence,occurred_ms,pr_id,transition_kind,record) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![source,family(kind),repository,id,digest,claim_id,sequence,occurred,pr_id,transition,serde_json::to_string(&fact.fact)?])?;
    Ok(())
}

fn apply_one(connection: &mut Connection, key: &str, id: &str, commit: &CaptureCommit) -> Result<bool> {
    let (sequence, predecessor) = current_head(connection, key)?;
    ensure!(commit.sequence == sequence + 1 && commit.predecessor == predecessor, "capture predecessor or sequence diverges");
    let mut facts = Vec::with_capacity(commit.facts.len());
    for reference in &commit.facts {
        let Some((fact, _)) = held::<ObservedFact>(connection, &reference.claim_id, FACT_KIND)? else { return Ok(false) };
        ensure!(fact.source == commit.source && fact.digest()? == reference.digest, "capture fact has another source or digest");
        facts.push((reference, fact));
    }
    let transaction = connection.transaction()?;
    for (reference, fact) in facts { apply_fact(&transaction, key, commit.sequence, &reference.claim_id, &fact, &reference.digest)?; }
    for coverage in &commit.coverage {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h0e8f2194bce1 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO capture_v1_coverage(source,repository_id,family,sequence,record) VALUES(?1,?2,?3,?4,?5)",
            params![key,coverage.repository_id,family(coverage.family),commit.sequence,serde_json::to_string(coverage)?])?;
    }
    if let Some(checkpoint) = &commit.checkpoint {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hf86388bd4ff5 owner=fractal_reconcile::reconcile::github_capture TODO=stage-generation-and-keyed-reclamation */ "INSERT OR REPLACE INTO capture_v1_checkpoints(source,repository_id,record) VALUES(?1,?2,?3)", params![key,checkpoint.repository_id,serde_json::to_string(checkpoint)?])?;
        if let (Some(pr), Some(updated)) = (&checkpoint.pull_request_id, checkpoint.upstream_updated_at_ms) {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h4ed2b30a6d8e owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO capture_v1_receipts(source,repository_id,pr_id,updated_ms,pass_ms) VALUES(?1,?2,?3,?4,?5)", params![key,checkpoint.repository_id,pr,updated,checkpoint.pass_started_at_ms])?;
        }
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hd74cfac88e82 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO capture_v1_history(source,sequence,commit_id) VALUES(?1,?2,?3)", params![key,commit.sequence,id])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hbd54d59b8187 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "UPDATE capture_v1_heads SET sequence=?2,commit_id=?3,error=NULL WHERE source=?1", params![key,commit.sequence,id])?;
    transaction.commit()?;
    Ok(true)
}

/// New arrivals at an already applied sequence are checked once, never silently first-wins.
fn audit_late_candidates(connection:&Connection,key:&str,epochs:&BTreeMap<u64,Epoch>)->Result<bool> {
    let candidates={
        let mut statement=connection.prepare_cached("SELECT q.claim_id,c.record,c.origin FROM capture_v1_conflicts q JOIN capture_v1_claims c ON c.id=q.claim_id WHERE q.source=?1 ORDER BY q.claim_id LIMIT 129")?;
        statement.query_map([key],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let complete=candidates.len()<=PAGE;
    for (id,record,origin) in candidates.into_iter().take(PAGE) {
        let commit:CaptureCommit=serde_json::from_str(&record)?;
        if let Some(epoch)=epochs.get(&commit.epoch) {
            ensure!(epoch.grant_id!=commit.authority_grant || epoch.member!=origin
                || epoch.final_sequence.is_some_and(|last|commit.sequence>last),
                "a late admitted capture diverges from an already applied source sequence");
        }
        connection.execute(/* sqlite-large-value: allow-for-now lv-h0fde81c700c9 owner=fractal_reconcile::reconcile::github_capture TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM capture_v1_conflicts WHERE source=?1 AND claim_id=?2",params![key,id])?;
    }
    Ok(complete)
}

/// At most PAGE commits advance per call; incomplete dependency chains remain explicit not-ready.
fn advance(connection: &mut Connection, definition: &SourceDefinition) -> Result<bool> {
    let key = prepare_source(connection, definition)?;
    let Some(epochs) = authority_chain(connection, definition)? else { return Ok(false) };
    if !audit_late_candidates(connection,&key,&epochs)? {return Ok(false);}
    let acquisition_source = source_key(&definition.source)?;
    for _ in 0..PAGE {
        let (sequence, _) = current_head(connection, &key)?;
        let candidates = {
            let mut statement = connection.prepare("SELECT id,record,origin FROM capture_v1_claims WHERE source=?1 AND kind=?2 AND sequence=?3 ORDER BY id LIMIT 129")?;
            let rows = statement.query_map(params![acquisition_source,COMMIT_KIND,sequence+1], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        ensure!(candidates.len() <= PAGE, "too many divergent capture candidates");
        let mut admitted = None;
        for (id, record, origin) in candidates {
            let commit: CaptureCommit = serde_json::from_str(&record)?;
            let Some(epoch) = epochs.get(&commit.epoch) else { continue };
            if epoch.grant_id != commit.authority_grant || epoch.member != origin || epoch.final_sequence.is_some_and(|last| commit.sequence > last) { continue; }
            ensure!(admitted.is_none(), "two admitted captures claim the same source sequence");
            admitted = Some((id, commit));
        }
        let Some((id, commit)) = admitted else {
            // A gap is pending only when the selected authority has a later capture.
            // Each epoch probes an indexed range; unrelated candidates cannot consume
            // the replay budget or masquerade as admitted progress.
            for (number, epoch) in &epochs {
                let later: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM capture_v1_claims WHERE source=?1 AND kind=?2 AND origin=?3 AND json_extract(record,'$.authority_grant')=?4 AND json_extract(record,'$.epoch')=?5 AND sequence>?6 AND sequence<=coalesce(?7,9223372036854775807))",
                    params![acquisition_source,COMMIT_KIND,epoch.member,epoch.grant_id,number,sequence,epoch.final_sequence],
                    |row| row.get(0),
                )?;
                if later { return Ok(false); }
            }
            return Ok(true);
        };
        if !apply_one(connection, &key, &id, &commit)? { return Ok(false); }
    }
    Ok(false)
}

/// Refresh only bounded pages; callers never report a partially replayed index as covered.
fn ready(store: &Store, connection: &mut Connection, definition: &SourceDefinition) -> Result<bool> {
    if definition.authority_grant.is_none() { return Ok(false); }
    let caught_up = ingest(store, connection)?;
    let advanced = advance(connection, definition)?;
    Ok(caught_up && advanced)
}

fn coverage(connection: &Connection, source: &str, roster: &Roster) -> Result<Vec<Coverage>> {
    let mut result = Vec::new();
    let repositories: BTreeSet<_> = roster.members.iter().map(|member| &member.repository_id).collect();
    for repository in repositories {
        let mut statement = connection.prepare_cached("SELECT record FROM capture_v1_coverage WHERE source=?1 AND repository_id=?2 ORDER BY family")?;
        for value in statement.query_map(params![source,repository], |row| row.get::<_,String>(0))? { result.push(serde_json::from_str(&value?)?); }
    }
    Ok(result)
}

/// Finite evidence horizon, not a completeness verdict. Missing required coverage
/// remains visible to each measurement window instead of erasing covered history.
fn measurement_horizon(records: &[Coverage], roster: &Roster, required: &[FactFamily]) -> Option<u64> {
    records.iter().filter(|record| required.contains(&record.family)
        && record.complete && record.authoritative && record.error.is_none())
        .flat_map(|record| roster.members.iter().filter_map(move |member| {
            let end = record.through_ms.min(member.effective_until_ms.unwrap_or(u64::MAX));
            (member.repository_id == record.repository_id
                && end > record.history_from_ms.max(member.effective_from_ms)).then_some(end)
        })).max()
}

pub fn measurement_frontier(store: &Store, state_dir: &Path, source_id: &str, roster_id: &str, required: &[FactFamily]) -> Result<Option<u64>> {
    let (Some(definition), Some(roster)) = (selected_source(store, source_id)?, selected_roster(store, roster_id)?) else { return Ok(None) };
    let plugin = GithubPluginStore::shared(state_dir)?;
    let mut connection = plugin.connection.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !ready(store, &mut connection, &definition)? { return Ok(None); }
    let records = coverage(&connection, &projection_key(&definition)?, &roster)?;
    Ok(measurement_horizon(&records, &roster, required))
}

/// Each retry advances a bounded keyset page. Pending reads pin a SQLite read transaction,
/// revision and bounds, so regular acquisition and a moving Now cut cannot starve completion.
/// A completed read reports its actual cut; subsequent requests refresh newer revisions.
pub fn measurement_snapshot(store: &Store, state_dir: &Path, source_id: &str, roster_id: &str, window: std::ops::Range<u64>, view_id: &str, required: &[FactFamily]) -> Result<Option<MeasurementSnapshot>> {
    let std::ops::Range { start: start_ms, end: end_ms } = window;
    let (Some(definition), Some(roster)) = (selected_source(store, source_id)?, selected_roster(store, roster_id)?) else { return Ok(None) };
    let plugin = GithubPluginStore::shared(state_dir)?;
    let mut connection = plugin.connection.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !ready(store, &mut connection, &definition)? { return Ok(None); }
    let source = projection_key(&definition)?;
    let (_, head) = current_head(&connection, &source)?;
    let Some(head) = head else { return Ok(None) };
    // Independent view definitions (civil timezone and Now/watermark cut) must not consume
    // each other's differently bounded reads. The caller supplies that stable view identity.
    let cache_key = (state_dir.to_path_buf(), source.clone(), serde_json::to_string(&roster)?, format!("{view_id}:{required:?}"));
    let mut snapshots = SNAPSHOTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(snapshot) = snapshots.complete.get(&cache_key) {
        if snapshot.projection.revision == head && snapshot.start_ms == start_ms && snapshot.end_ms == end_ms {
            return Ok(Some(snapshot.clone()));
        }
        snapshots.complete.remove(&cache_key);
    }
    if !snapshots.building.contains_key(&cache_key) {
        let records = coverage(&connection, &source, &roster)?;
        let frontier_ms = measurement_horizon(&records, &roster, required).unwrap_or(0);
        let projection = Projection { source: definition.source,revision:head,frontier_ms,roster,pull_requests:Vec::new(),lifecycle:Vec::new(),merges:Vec::new(),coverage:records,ready_cycles:BTreeMap::new() };
        // Keep a bounded number of independent requests, rather than an unbounded query cache.
        if snapshots.building.len() + snapshots.complete.len() >= 8 {
            snapshots.building.clear();
            snapshots.complete.clear();
        }
        // A separate WAL read transaction preserves corrected images across continuations.
        let mut reader = Connection::open_with_flags(connection.path().context("capture plugin has no database path")?, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        smallclaims::sqlite::observe(&mut reader);
        reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h5234cc786356 owner=fractal_reconcile::reconcile::github_capture TODO=trace-statement-and-value-representation */ "BEGIN DEFERRED")?;
        ensure!(current_head(&reader, &source)?.1.as_deref() == Some(projection.revision.as_str()), "capture head changed while pinning snapshot");
        let mut builder = SnapshotBuilder::new(projection);
        builder.reader = Some(reader);
        builder.start_ms = start_ms;
        builder.end_ms = end_ms;
        snapshots.building.insert(cache_key.clone(), builder);
    }
    let builder = snapshots.building.get_mut(&cache_key).context("snapshot cache entry disappeared")?;
    let reader = builder.reader.take().context("snapshot reader is absent")?;
    let advanced = builder.advance(&reader, &source, builder.start_ms, builder.end_ms);
    builder.reader = Some(reader);
    if !advanced? { return Ok(None); }
    let builder = snapshots.building.remove(&cache_key).context("snapshot cache entry disappeared")?;
    let snapshot = MeasurementSnapshot { projection: Arc::new(builder.projection), start_ms: builder.start_ms, end_ms: builder.end_ms };
    snapshots.complete.insert(cache_key, snapshot.clone());
    Ok(Some(snapshot))
}

impl SnapshotBuilder {
    fn new(projection: Projection) -> Self {
        let repositories = projection.roster.members.iter().map(|member| member.repository_id.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        Self { projection, repositories, repository: 0, after: None, reader: None, start_ms: 0, end_ms: 0 }
    }

    /// Repository probes and merge enrichment both consume the shared per-call budget.
    /// The tuple cursor preserves equal-timestamp merges without OFFSET scans.
    fn advance(&mut self, connection: &Connection, source: &str, start_ms: u64, end_ms: u64) -> Result<bool> {
        let mut budget = SNAPSHOT_PAGE;
        while self.repository < self.repositories.len() && budget > 0 {
            let repository = &self.repositories[self.repository];
            let (after_ms, after_id) = self.after.as_ref().map_or((start_ms, ""), |(ms, id)| (*ms, id.as_str()));
            let rows = {
                let mut statement = connection.prepare_cached("SELECT occurred_ms,entity_id,record FROM capture_v1_entities WHERE source=?1 AND family='merge' AND repository_id=?2 AND (occurred_ms,entity_id)>(?3,?4) AND occurred_ms<?5 ORDER BY occurred_ms,entity_id LIMIT ?6")?;
                let rows = statement.query_map(params![source,repository,after_ms,after_id,end_ms,budget],
                    |row| Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            if rows.is_empty() {
                budget -= 1;
                self.repository += 1;
                self.after = None;
                continue;
            }
            budget -= rows.len();
            for (occurred, id, record) in rows {
                if let FactImage::Merge(merge) = serde_json::from_str(&record)? {
                    let pr: Option<String> = connection.query_row("SELECT record FROM capture_v1_entities WHERE source=?1 AND family='pull_request' AND repository_id=?2 AND entity_id=?3", params![source,repository,merge.pull_request_id], |row| row.get(0)).optional()?;
                    let mut ready_at = None;
                    if let Some(record) = pr && let FactImage::PullRequest(pr) = serde_json::from_str(&record)? {
                        let draft: Option<u64> = connection.query_row("SELECT occurred_ms FROM capture_v1_entities WHERE source=?1 AND family='lifecycle' AND repository_id=?2 AND pr_id=?3 AND transition_kind='draft' AND occurred_ms<=?4 ORDER BY occurred_ms DESC,entity_id DESC LIMIT 1", params![source,repository,merge.pull_request_id,merge.merged_at_ms], |row| row.get(0)).optional()?;
                        let first_ready: Option<u64> = connection.query_row("SELECT occurred_ms FROM capture_v1_entities WHERE source=?1 AND family='lifecycle' AND repository_id=?2 AND pr_id=?3 AND transition_kind='ready' AND occurred_ms>=?4 AND occurred_ms<=?5 ORDER BY occurred_ms,entity_id LIMIT 1", params![source,repository,merge.pull_request_id,draft.unwrap_or(pr.created_at_ms),merge.merged_at_ms], |row| row.get(0)).optional()?;
                        ready_at = if draft.is_none() && pr.created_as_draft == Some(false) { Some(pr.created_at_ms) }
                            else if first_ready.is_some() && first_ready != draft { first_ready } else { None };
                        self.projection.pull_requests.push(pr);
                    }
                    self.projection.ready_cycles.insert((merge.repository_id.clone(),merge.pull_request_id.clone()),ready_at);
                    self.projection.merges.push(merge);
                }
                self.after = Some((occurred, id));
            }
        }
        Ok(self.repository == self.repositories.len())
    }
}

#[cfg(test)]
#[path = "github_capture_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn checkpoint(connection: &Connection, definition: &SourceDefinition, repository: &str) -> Result<Option<ScanCheckpoint>> {
    let row: Option<String> = connection.query_row("SELECT record FROM capture_v1_checkpoints WHERE source=?1 AND repository_id=?2", params![projection_key(definition)?,repository], |row| row.get(0)).optional()?;
    row.map(|value| Ok(serde_json::from_str(&value)?)).transpose()
}

pub(super) fn captured_since(connection: &Connection, definition: &SourceDefinition, repository: &str, pr: &str, updated: u64, pass: u64) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM capture_v1_receipts WHERE source=?1 AND repository_id=?2 AND pr_id=?3 AND updated_ms>=?4 AND pass_ms>=?5)", params![projection_key(definition)?,repository,pr,updated,pass], |row| row.get(0))?)
}
