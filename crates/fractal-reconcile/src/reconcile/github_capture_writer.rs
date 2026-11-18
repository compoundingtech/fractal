//! The sole source acquisition owner; no cursor or fact authority lives in this runtime cache.
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;
use crate::model::{ClaimInput, ClaimRecord};
use crate::github_capture::{AcquisitionCheckpoint, AcquisitionPage, AcquisitionStatus, CaptureAcquisition, CaptureProgress, GitHubCaptureLedger, RepositoryTarget};
use crate::github_http::GithubAuth;
use super::*;
use super::super::github_plugin_store::MANIFEST;

#[derive(Default)]
struct Work {
    grant: String,
    acquisitions: BTreeMap<String, CaptureAcquisition>,
}

type WorkHandle = Arc<AsyncMutex<Work>>;
static WORK: LazyLock<Mutex<BTreeMap<(PathBuf, String), WorkHandle>>> = LazyLock::new(Mutex::default);

fn work_handle(state_dir: &Path, source: &Source) -> Result<WorkHandle> {
    let key = (state_dir.to_path_buf(), source_key(source)?);
    let mut handles = WORK.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(Arc::clone(handles.entry(key).or_default()))
}

#[derive(Clone)]
pub struct GraphGitHubCaptureLedger {
    pub store: Arc<Store>,
    pub state_dir: PathBuf,
}

impl std::fmt::Debug for GraphGitHubCaptureLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GraphGitHubCaptureLedger")
    }
}

fn now_ms() -> Result<u64> { Ok(u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?) }

/// Only the public GitHub origin has an established web-host/API-host mapping.
/// Enterprise routing needs an explicit mapping contract; never infer `api.<host>`.
fn validate_api_identity(source: &Source, api_base: &str) -> Result<()> {
    let api = url::Url::parse(api_base).context("invalid GitHub observer API base URL")?;
    ensure!(source.host == "github.com", "GitHub Enterprise API host mapping is not declared");
    ensure!(api.scheme() == "https" && api.host_str() == Some("api.github.com")
        && api.port_or_known_default() == Some(443) && api.username().is_empty()
        && api.password().is_none() && matches!(api.path(), "" | "/")
        && api.query().is_none() && api.fragment().is_none(),
        "GitHub observer API base does not identify the declared source host");
    Ok(())
}

/// A failed durable publication must never leave a request-advanced cursor in memory.
fn publish_page(work: &mut Work, repository: &str, page: &AcquisitionPage,
    publication: impl FnOnce() -> Result<()>) -> Result<()> {
    if page.checkpoint.repository.is_some()
        && let Err(error) = publication() {
        work.acquisitions.remove(repository);
        return Err(error);
    }
    if page.status != AcquisitionStatus::Continue {
        work.acquisitions.remove(repository);
    }
    Ok(())
}

fn append_record(store: &Store, subject: String, kind: &str, record: &impl Serialize, key: String) -> Result<ClaimRecord> {
    let fields = BTreeMap::from([("record".into(),serde_json::to_value(record)?)]);
    MANIFEST.admit(&subject,kind,&fields).map_err(anyhow::Error::msg)?;
    // Identical normalized revisions retain their original observed time after a staging crash.
    if kind == FACT_KIND && let Some(existing) = store.latest_claim(&subject,Some(kind))? { return Ok(existing); }
    Ok(store.append_claim(&ClaimInput { subject,kind:kind.into(),actor:None,fields,
        evidence:Vec::new(),expected_subject:None,idempotency_key:Some(key) })?)
}

#[derive(Default)]
struct CommitContent {
    facts: Vec<FactReference>,
    checkpoint: Option<ScanCheckpoint>,
    coverage: Vec<Coverage>,
}

fn commit(store: &Store, connection: &mut Connection, definition: &SourceDefinition, grant: &AuthorityGrant,
    content: CommitContent, now: u64) -> Result<()> {
    let source = projection_key(definition)?;
    let (sequence, predecessor) = current_head(connection,&source)?;
    let capture = CaptureCommit { version:VERSION,source:definition.source.clone(),epoch:grant.epoch,
        authority_grant:selected_authority(definition)?.to_owned(),
        sequence:sequence.checked_add(1).context("source sequence overflow")?,predecessor,
        facts:content.facts,checkpoint:content.checkpoint,coverage:content.coverage,observed_at_ms:now };
    let digest = capture.digest()?;
    let record = append_record(store,capture.subject()?,COMMIT_KIND,&capture,format!("github-capture:{digest}"))?;
    // The claim is the authority. Local publication can be replayed after any cache failure.
    ingest(store,connection)?;
    ensure!(advance(connection,definition)?, "capture projection has pending predecessors or dependencies");
    let (_, selected) = current_head(connection,&source)?;
    ensure!(selected.as_deref() == Some(record.id.as_str()), "capture was not admitted at the source frontier");
    Ok(())
}

fn image_revision(connection: &Connection, definition: &SourceDefinition, image: &FactImage) -> Result<Option<(String,FactImage)>> {
    let (kind,repository,id) = image.identity();
    let row: Option<(String,String)> = connection.query_row("SELECT digest,record FROM capture_v1_entities WHERE source=?1 AND family=?2 AND repository_id=?3 AND entity_id=?4", params![projection_key(definition)?,family(kind),repository,id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    row.map(|(digest,record)| Ok((digest,serde_json::from_str(&record)?))).transpose()
}

fn writer_grant(connection: &Connection, definition: &SourceDefinition, member: &str) -> Result<AuthorityGrant> {
    let authority = selected_authority(definition)?;
    let Some((grant,_)) = held::<AuthorityGrant>(connection,authority,AUTHORITY_KIND)? else { anyhow::bail!("source authority grant is not present") };
    ensure!(grant.source == definition.source && grant.member == member, "member is not the designated source authority");
    ensure!(authority_chain(connection,definition)?.is_some(), "source authority transfer dependencies are not ready");
    let mut statement = connection.prepare("SELECT record,origin FROM capture_v1_claims WHERE source=?1 AND kind=?2 LIMIT 129")?;
    let mut receipts = 0;
    for row in statement.query_map(params![source_key(&definition.source)?,FENCED_KIND], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))? {
        receipts += 1;
        ensure!(receipts <= 128, "source fencing history exceeds bounded admission read");
        let (record,origin) = row?;
        let receipt: FenceReceipt = serde_json::from_str(&record)?;
        ensure!(!(receipt.source == definition.source && receipt.epoch == grant.epoch
            && receipt.authority_grant == authority
            && receipt.member == grant.member && receipt.member == origin),
            "the selected source writer has been fenced and cannot resume");
    }
    Ok(grant)
}

fn publish_coverage(store:&Store, connection:&mut Connection, definition:&SourceDefinition,
    grant:&AuthorityGrant, rows:&[Coverage], now:u64)->Result<()> {
    for coverage in rows {
        let prior:Option<String>=connection.query_row("SELECT record FROM capture_v1_coverage WHERE source=?1 AND repository_id=?2 AND family=?3",
            params![projection_key(definition)?,coverage.repository_id,family(coverage.family)],|row|row.get(0)).optional()?;
        if prior.map(|row|serde_json::from_str::<Coverage>(&row)).transpose()?.as_ref()==Some(coverage) {continue;}
        commit(store,connection,definition,grant,CommitContent { coverage:vec![coverage.clone()],..Default::default() },now)?;
    }
    Ok(())
}

fn publish(store: &Store, connection: &mut Connection, definition: &SourceDefinition, grant: &AuthorityGrant, locator: &str, page: &AcquisitionPage, now: u64) -> Result<()> {
    let repository = page.checkpoint.repository.as_ref().context("capture has not resolved repository identity")?;
    if let Some(previous) = configured_checkpoint(connection,definition,locator)? {
        ensure!(previous.repository_id == repository.id, "configured repository resolved to another committed node identity");
    }
    ensure!(!repository.is_private || definition.replicate_private, "private source replication has not been explicitly admitted");
    if page.status != AcquisitionStatus::Complete {
        publish_coverage(store,connection,definition,grant,&page.coverage,now)?;
    }
    let mut references = Vec::with_capacity(MAX_REFERENCES);
    for image in &page.facts {
        let prior = image_revision(connection,definition,image)?;
        if prior.as_ref().is_some_and(|(_,prior)| prior == image) { continue; }
        let fact = ObservedFact { version:VERSION,source:definition.source.clone(),observed_at_ms:now,
            supersedes:prior.map(|(digest,_)| digest),fact:image.clone() };
        let digest = fact.digest()?;
        let written = append_record(store,fact.subject()?,FACT_KIND,&fact,format!("github-fact:{digest}"))?;
        references.push(FactReference { claim_id:written.id,digest });
        if references.len() == MAX_REFERENCES {
            commit(store,connection,definition,grant,CommitContent { facts:std::mem::take(&mut references),..Default::default() },now)?;
        }
    }
    if !references.is_empty() { commit(store,connection,definition,grant,CommitContent { facts:references,..Default::default() },now)?; }
    let receipt = page.receipt.as_ref();
    // Bind the authored scope member to the resolved node, not its mutable upstream name.
    // This travels in the committed checkpoint, so cache rebuilds retain rename identity.
    let mut cursor = serde_json::to_value(&page.checkpoint)?;
    cursor["configured_locator"] = Value::String(locator.to_ascii_lowercase());
    let checkpoint = ScanCheckpoint { repository_id:repository.id.clone(),
        pass_started_at_ms:receipt.map_or_else(|| page.checkpoint.pass.map_or(now,|pass|pass.started_ms),|receipt|receipt.pass_started_ms),
        cursor:Some(serde_json::to_string(&cursor)?),pass_complete:page.status == AcquisitionStatus::Complete,
        pull_request_id:receipt.map(|receipt|receipt.pull_request_id.clone()),
        upstream_updated_at_ms:receipt.map(|receipt|receipt.upstream_updated_at_ms) };
    if page.status == AcquisitionStatus::Complete {
        publish_coverage(store,connection,definition,grant,&page.coverage,now)?;
    }
    commit(store,connection,definition,grant,CommitContent { checkpoint:Some(checkpoint),..Default::default() },now)?;
    Ok(())
}

impl GitHubCaptureLedger for GraphGitHubCaptureLedger {
    fn step<'a>(&'a self, api_base: &'a str, repository: &'a str, auth: &'a GithubAuth)
        -> Pin<Box<dyn Future<Output=Result<CaptureProgress>> + Send + 'a>> {
        Box::pin(async move {
            let mut selected = None;
            for definition in self.store.github_source_definitions()? {
                if definition.enabled && definition.repositories.iter().any(|locator| locator.eq_ignore_ascii_case(repository)) {
                    ensure!(selected.is_none(), "repository belongs to two enabled acquisition sources");
                    selected = Some(definition);
                }
            }
            let Some(definition) = selected else { return Ok(CaptureProgress { active:false,continue_after_ms:None }) };
            let authority = selected_authority(&definition)?;
            validate_api_identity(&definition.source, api_base)?;
            let plugin = GithubPluginStore::shared(&self.state_dir)?;
            let source_work = work_handle(&self.state_dir,&definition.source)?;
            let mut work = source_work.lock().await;
            let now = now_ms()?;
            let (grant, previous) = {
                let mut connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
                if !ready(&self.store,&mut connection,&definition)? {
                    return Ok(CaptureProgress { active:true,continue_after_ms:Some(1000) });
                }
                if held::<AuthorityGrant>(&connection,authority,AUTHORITY_KIND)?
                    .is_some_and(|(grant,_)|grant.member != self.store.origin()) {
                    return Ok(CaptureProgress {active:false,continue_after_ms:None});
                }
                let grant = writer_grant(&connection,&definition,self.store.origin())?;
                let previous = configured_checkpoint(&connection,&definition,repository)?;
                (grant,previous)
            };
            if work.grant != authority { work.acquisitions.clear();work.grant=authority.to_owned(); }
            if !work.acquisitions.contains_key(repository) {
                let previous: AcquisitionCheckpoint = previous.and_then(|checkpoint|checkpoint.cursor)
                    .map(|cursor|serde_json::from_str(&cursor)).transpose()?.unwrap_or_default();
                if previous.pass.is_none() && previous.checked_ms.is_some_and(|checked| now.saturating_sub(checked) < definition.checkpoint_interval_ms) {
                    return Ok(CaptureProgress { active:true,continue_after_ms:None });
                }
                let target = RepositoryTarget { locator:repository.to_owned(),node_id:previous.repository.as_ref().map(|repo|repo.id.clone()) };
                work.acquisitions.insert(repository.to_owned(),CaptureAcquisition::resume(target,previous,now));
            }
            let acquisition = work.acquisitions.get_mut(repository).context("source acquisition disappeared")?;
            let mut lookup_error = None;
            let page = acquisition.next_page(api_base,auth,|id,updated,pass| {
                let connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
                // Receipts carry source and repository identity; PR IDs cannot alias another source.
                let repository_id = page_repository(&connection,&definition,repository);
                match repository_id.and_then(|repository|captured_since(&connection,&definition,&repository,id,updated,pass)) {
                    Ok(captured)=>captured,Err(error)=>{lookup_error=Some(error);false}
                }
            }).await;
            if let Some(error)=lookup_error {work.acquisitions.remove(repository);return Err(error);}
            publish_page(&mut work, repository, &page, || -> Result<()> {
                    let mut connection=plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
                    // Recheck the selected grant/fence immediately before every durable publication.
                    ensure!(selected_source(&self.store,&definition.source.id)?.as_ref()==Some(&definition),
                        "source declaration changed while its request was in flight");
                    ensure!(ready(&self.store,&mut connection,&definition)?, "source dependencies changed during acquisition");
                    writer_grant(&connection,&definition,self.store.origin())?;
                    publish(&self.store,&mut connection,&definition,&grant,repository,&page,now)
            })?;
            let status=page.status;
            if let Some(error)=page.error {return Err(error);}
            Ok(CaptureProgress {active:true,continue_after_ms:(status==AcquisitionStatus::Continue).then_some(1000)})
        })
    }
}

fn configured_checkpoint(connection:&Connection,definition:&SourceDefinition,locator:&str)->Result<Option<ScanCheckpoint>> {
    let mut statement=connection.prepare_cached("SELECT record FROM capture_v1_checkpoints WHERE source=?1 LIMIT 129")?;
    let rows=statement.query_map([projection_key(definition)?],|row|row.get::<_,String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(rows.len()<=128,"source repository population exceeds bounded lookup");
    for row in rows {
        let checkpoint:ScanCheckpoint=serde_json::from_str(&row)?;
        let cursor:Value=serde_json::from_str(checkpoint.cursor.as_deref().context("capture checkpoint has no acquisition cursor")?)?;
        let configured=cursor.get("configured_locator").and_then(Value::as_str)
            .context("committed checkpoint has no authored repository binding; refusing to infer membership from its current name")?;
        if configured.eq_ignore_ascii_case(locator) {
            return Ok(Some(checkpoint));
        }
    }
    Ok(None)
}

fn page_repository(connection:&Connection,definition:&SourceDefinition,locator:&str)->Result<String> {
    Ok(configured_checkpoint(connection,definition,locator)?
        .context("repository identity is not yet committed")?.repository_id)
}

/// Serialize with acquisition, then durably stop the exact authority before a successor is admitted.
pub async fn fence_source(store:&Store,state_dir:&Path,source_id:&str)->Result<ClaimRecord> {
    let definition=selected_source(store,source_id)?.context("source is not declared")?;
    fence_source_checked(store,state_dir,source_id,&definition.source,selected_authority(&definition)?).await
}

/// Fence only the generation and grant the operator inspected, including on retries.
pub async fn fence_source_checked(store:&Store,state_dir:&Path,source_id:&str,
    expected_source:&Source,expected_grant:&str)->Result<ClaimRecord> {
    let definition=selected_source(store,source_id)?.context("source is not declared")?;
    ensure!(&definition.source==expected_source && definition.authority_grant.as_deref()==Some(expected_grant),
        "source authority changed before fencing");
    let handle=work_handle(state_dir,&definition.source)?;
    let mut work=handle.lock().await;
    ensure!(selected_source(store,source_id)?.as_ref()==Some(&definition),
        "source declaration changed while waiting to fence its writer");
    let plugin=GithubPluginStore::shared(state_dir)?;
    let mut connection=plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
    ensure!(ready(store,&mut connection,&definition)?,"source projection is catching up");
    let grant=held::<AuthorityGrant>(&connection,expected_grant,AUTHORITY_KIND)?
        .context("source authority grant is not present")?.0;
    ensure!(grant.source==definition.source && grant.member==store.origin(),
        "member is not the designated source authority");
    let (_,frontier)=current_head(&connection,&projection_key(&definition)?)?;
    if let Some(receipt)=existing_fence(store,&connection,&definition,&grant,frontier.as_deref())? {
        work.acquisitions.clear();
        return Ok(receipt);
    }
    writer_grant(&connection,&definition,store.origin())?;
    let receipt=FenceReceipt {version:VERSION,source:definition.source.clone(),member:store.origin().to_owned(),epoch:grant.epoch,
        authority_grant:expected_grant.to_owned(),final_commit:frontier,stopped_at_ms:now_ms()?};
    work.acquisitions.clear();
    let subject=format!("github-source/{}",definition.source.id);
    let key=format!("github-fenced:{}:{}:{}",source_key(&definition.source)?,expected_grant,grant.epoch);
    let claim=append_record(store,subject,FENCED_KIND,&receipt,key)?;
    ingest(store,&mut connection)?;
    Ok(claim)
}

fn existing_fence(store:&Store,connection:&Connection,definition:&SourceDefinition,
    grant:&AuthorityGrant,frontier:Option<&str>)->Result<Option<ClaimRecord>> {
    let mut statement=connection.prepare("SELECT id,record,origin FROM capture_v1_claims WHERE source=?1 AND kind=?2 LIMIT 129")?;
    let mut count=0;
    for row in statement.query_map(params![source_key(&definition.source)?,FENCED_KIND],
        |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))? {
        count+=1;
        ensure!(count<=128,"source fencing history exceeds bounded admission read");
        let (id,record,origin)=row?;
        let receipt:FenceReceipt=serde_json::from_str(&record)?;
        if receipt.source==definition.source && Some(receipt.authority_grant.as_str())==definition.authority_grant.as_deref()
            && receipt.epoch==grant.epoch && receipt.member==grant.member && receipt.member==origin {
            ensure!(receipt.final_commit.as_deref()==frontier,"fenced frontier changed after writer stopped");
            return crate::store::claim_by_id_tx(&store.readers.get(),&id);
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "github_capture_writer_tests.rs"]
mod tests;
