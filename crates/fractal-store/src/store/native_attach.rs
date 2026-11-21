//! Host-owned native attach admission. Claims are authority; indexes are disposable.
//! These records never declare a launch or convey authority to signal a native process.
use super::*;
use fractal_runtime::native_attach::{AttachEvidence, AttachWitness};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

pub const REQUESTER: &str = "daemon/native-attach";
pub const PREPARED: &str = "custom.native-attach.prepared";
pub const ASK_BOUND: &str = "custom.native-attach.ask-bound";
pub const COMMITTED: &str = "custom.native-attach.committed";
pub const MAX_APPROVAL_MS: u128 = 5 * 60 * 1000;

pub fn protected(kind: &str) -> bool { kind.starts_with("custom.native-attach.") }
pub fn writer_required() -> FractalError {
    FractalError::new("native-attach-writer-required", "native attachment records require verified native attach admission")
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h5d6244f528ee owner=fractal_store::store::native_attach TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS native_attach_requests ON claims(origin,json_extract(body,'$.fields.request_key'))
         WHERE kind='custom.native-attach.prepared';
         CREATE INDEX IF NOT EXISTS native_attach_agents ON claims(json_extract(body,'$.fields.candidate.agent'))
         WHERE kind='custom.native-attach.committed';
         CREATE INDEX IF NOT EXISTS native_attach_answers ON claims(json_extract(body,'$.fields.answer_id'))
         WHERE kind='custom.native-attach.committed';
         CREATE INDEX IF NOT EXISTS native_attach_processes ON claims(json_extract(body,'$.fields.process_key'))
         WHERE kind='custom.native-attach.committed';
         CREATE INDEX IF NOT EXISTS native_attach_sessions ON claims(json_extract(body,'$.fields.native_key'))
         WHERE kind='custom.native-attach.committed';"
    )?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub host: String,
    pub agent: String,
    pub person: String,
    pub evidence: AttachEvidence,
    pub workspace: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preparation {
    pub id: String,
    pub request_key: String,
    pub candidate: Candidate,
    pub candidate_digest: String,
    pub challenge_digest: String,
    pub prepared_at_unix_ms: u128,
    pub expires_at_unix_ms: u128,
}

impl Preparation {
    pub fn subject(&self) -> String { format!("custom/native-attach/{}", self.id) }
    pub fn ask_key(&self) -> String { format!("native-attach/{}/{}", self.id, self.candidate_digest) }
    pub fn request(&self) -> Value {
        let evidence = &self.candidate.evidence;
        json!({"version":1,"type":"decision",
            "question":format!("Attach this running OMP session to {} without restarting it or sending an initial message?",self.candidate.agent),
            "why_person":"Only you may authorize this exact external process to become an addressable agent; restart consent is separate.",
            "reasons":[
                format!("Host: {}; UID: {}; PID: {}; start: {}; boot: {}",self.candidate.host,evidence.process.uid,evidence.process.pid,evidence.process.start_token,evidence.process.boot_id),
                format!("Full native ID: {}",evidence.transcript.native_id),
                format!("Transcript: {}; device/inode: {}/{}",evidence.transcript.path.display(),evidence.transcript.device,evidence.transcript.inode),
                format!("Header cwd: {:?}; observed cwd: {:?}",evidence.transcript.header_cwd,evidence.observed_cwd),
                format!("Durable workspace: {}",self.candidate.workspace.display()),
                format!("Candidate digest: {}",self.candidate_digest)
            ],
            "answers":[
                {"id":"attach","label":"Attach this session","outcome":"accept","consequence":"Authorize this exact external incarnation once, with no startup message and no process ownership."},
                {"id":"decline","label":"Do not attach","outcome":"decline","consequence":"Leave the native process unchanged and grant no attachment authority."}
            ]})
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachedIncarnation {
    pub id: String,
    pub preparation_id: String,
    pub candidate: Candidate,
    pub candidate_digest: String,
    pub answer_id: String,
    pub approval_receipt: String,
    pub request_key: String,
    pub process_key: String,
    pub native_key: String,
    pub committed_at_unix_ms: u128,
    pub ownership: Ownership,
    pub phase: AttachPhase,
    pub epoch: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Ownership { External }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AttachPhase { ChannelPending }

pub fn candidate_digest(candidate: &Candidate) -> Result<String, FractalError> {
    smallclaims::hash::canonical_hash(candidate).map_err(internal)
}

pub fn durable_workspace(path: &Path) -> Result<PathBuf, FractalError> {
    let canonical = fs::canonicalize(path).map_err(internal)?;
    if !canonical.is_dir() { return Err(refused("workspace is not an existing directory")); }
    for root in [PathBuf::from("/tmp"), PathBuf::from("/var/tmp"), std::env::temp_dir()] {
        if let Ok(root) = fs::canonicalize(root) && canonical.starts_with(root) {
            return Err(refused("temporary directories cannot be the declared durable workspace"));
        }
    }
    Ok(canonical)
}
fn refused(message: impl Into<String>) -> FractalError { FractalError::new("native-attach-refused", message) }
fn data<T: serde::de::DeserializeOwned>(claim: &ClaimRecord) -> Result<T, FractalError> {
    serde_json::from_value(claim.body["fields"].clone()).map_err(internal)
}
fn one(connection: &Connection, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    let mut statement = connection.prepare_cached(
        "SELECT id FROM claims WHERE subject=?1 AND kind=?2 AND NOT EXISTS
         (SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired') LIMIT 2"
    ).map_err(internal)?;
    let ids = statement.query_map(params![subject,kind], |row| row.get::<_,String>(0))
        .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
    if ids.len() > 1 { return Err(refused("conflicting native attach authority records")); }
    ids.first().map(|id| claim_by_id_tx(connection,id).map_err(internal)?
        .ok_or_else(|| refused("native attach authority record disappeared"))).transpose()
}
fn append<T: Serialize>(tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str, fields: &T, predecessors: &[String]) -> Result<ClaimRecord, FractalError> {
    append_admitted_claim_tx(tx,origin,subject,kind,Some(REQUESTER),&json!({"fields":fields}),predecessors,None,false)
        .map_err(claim_append_error)
}
fn preparation(connection: &Connection, origin: &str, id: &str) -> Result<(ClaimRecord, Preparation), FractalError> {
    uuid::Uuid::parse_str(id).map_err(|_| refused("invalid native attach preparation ID"))?;
    let claim = one(connection,&format!("custom/native-attach/{id}"),PREPARED)?
        .filter(|claim| claim.origin == origin && claim.actor.as_deref() == Some(REQUESTER))
        .ok_or_else(|| refused("native attach preparation does not belong to this daemon"))?;
    let value = data(&claim)?;
    Ok((claim,value))
}
fn conflict(connection: &Connection, column: &str, value: &str) -> Result<bool, FractalError> {
    match column {
        "agent" => connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.native-attach.committed' AND json_extract(body,'$.fields.candidate.agent')=?1)", [value], |row| row.get(0)),
        "answer" => connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.native-attach.committed' AND json_extract(body,'$.fields.answer_id')=?1)", [value], |row| row.get(0)),
        "process" => connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.native-attach.committed' AND json_extract(body,'$.fields.process_key')=?1)", [value], |row| row.get(0)),
        "native" => connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.native-attach.committed' AND json_extract(body,'$.fields.native_key')=?1)", [value], |row| row.get(0)),
        _ => unreachable!("closed native attach reservation columns"),
    }.map_err(internal)
}

impl Store {
    pub fn prepare_native_attach(&self, candidate: Candidate, request_key: &str, challenge_digest: &str, expires_at: u128, witness: &AttachWitness) -> Result<Preparation, FractalError> {
        let now = now_ms();
        if request_key.is_empty() || request_key.len() > 256 || challenge_digest != witness.challenge_digest()
            || expires_at <= now || expires_at > now + MAX_APPROVAL_MS
            || !candidate.agent.starts_with("agent/") || !candidate.person.starts_with("person/")
            || candidate.host != self.origin()
        { return Err(refused("invalid attach identity, expiry or idempotency key")); }
        witness.revalidate().map_err(internal)?;
        if &candidate.evidence != witness.evidence()
            || durable_workspace(&candidate.workspace)? != candidate.workspace
        { return Err(refused("candidate differs from verified process or durable workspace")); }
        let digest = candidate_digest(&candidate)?;
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        let existing: Option<String> = tx.query_row(
            "SELECT id FROM claims WHERE kind='custom.native-attach.prepared' AND origin=?1 AND json_extract(body,'$.fields.request_key')=?2 LIMIT 1",
            params![self.origin(),request_key],|row|row.get(0)).optional().map_err(internal)?;
        if let Some(id) = existing {
            let record = claim_by_id_tx(&tx,&id).map_err(internal)?.ok_or_else(||refused("preparation disappeared"))?;
            let existing: Preparation = data(&record)?;
            if existing.candidate_digest != digest || existing.challenge_digest != challenge_digest {
                return Err(refused("idempotency key already names another candidate or proof"));
            }
            return Ok(existing);
        }
        let prepared = Preparation { id:uuid::Uuid::new_v4().to_string(),request_key:request_key.into(),candidate,
            candidate_digest:digest,challenge_digest:challenge_digest.into(),prepared_at_unix_ms:now,expires_at_unix_ms:expires_at };
        let subject = prepared.subject();
        append(&tx,self.origin(),&subject,PREPARED,&prepared,&[])?;
        witness.revalidate().map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(prepared)
    }

    pub fn native_attach_preparation(&self, id: &str) -> Result<Preparation, FractalError> {
        preparation(&self.readers.get(),self.origin(),id).map(|(_,value)|value)
    }

    pub fn bind_native_attach_ask(&self, id: &str, ask: &str) -> Result<(), FractalError> {
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        let (prepared_claim, prepared) = preparation(&tx,self.origin(),id)?;
        let asking = one(&tx,ask,"work.person-asked")?.ok_or_else(||refused("native attach ask is absent"))?;
        if asking.origin != self.origin() || asking.actor.as_deref() != Some(REQUESTER)
            || asking.body["fields"]["key"] != prepared.ask_key()
            || asking.body["fields"]["person"] != prepared.candidate.person
            || asking.body["fields"]["request"] != prepared.request()
            || asking.body["fields"].get("default_answer").is_some_and(|value| !value.is_null())
        { return Err(refused("ask does not describe this exact prepared candidate")); }
        let fields = json!({"ask":ask,"episode":asking.id,"candidate_digest":prepared.candidate_digest});
        if let Some(existing) = one(&tx,&prepared.subject(),ASK_BOUND)? {
            if existing.body["fields"] != fields { return Err(refused("preparation already has another approval ask")); }
            return Ok(());
        }
        append(&tx,self.origin(),&prepared.subject(),ASK_BOUND,&fields,&[prepared_claim.id,asking.id])?;
        tx.commit().map_err(internal)
    }

    pub fn native_attach_binding(&self, id: &str) -> Result<Option<AttachedIncarnation>, FractalError> {
        let prepared = self.native_attach_preparation(id)?;
        one(&self.readers.get(),&prepared.subject(),COMMITTED)?.map(|claim|data(&claim)).transpose()
    }

    pub fn commit_native_attach(&self, id: &str, digest: &str, answer_id: &str, request_key: &str, witness: &AttachWitness) -> Result<AttachedIncarnation, FractalError> {
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        let (prepared_claim, prepared) = preparation(&tx,self.origin(),id)?;
        if let Some(existing) = one(&tx,&prepared.subject(),COMMITTED)? {
            let binding: AttachedIncarnation = data(&existing)?;
            if binding.candidate_digest != digest || binding.answer_id != answer_id || binding.request_key != request_key {
                return Err(refused("committed attachment has different idempotent inputs"));
            }
            return Ok(binding);
        }
        let now = now_ms();
        if prepared.candidate_digest != digest || prepared.expires_at_unix_ms <= now || request_key.is_empty() {
            return Err(refused("native attach preparation expired or changed"));
        }
        witness.revalidate().map_err(internal)?;
        if witness.challenge_digest() != prepared.challenge_digest
            || witness.evidence() != &prepared.candidate.evidence
            || durable_workspace(&prepared.candidate.workspace)? != prepared.candidate.workspace
        { return Err(refused("native attach process, transcript or workspace changed")); }
        let bound = one(&tx,&prepared.subject(),ASK_BOUND)?.ok_or_else(||refused("preparation has no bound person ask"))?;
        let answer = claim_by_id_tx(&tx,answer_id).map_err(internal)?.ok_or_else(||refused("person answer is absent"))?;
        let fields = &answer.body["fields"];
        if answer.kind != "work.person-done" || answer.actor.as_deref() != Some(prepared.candidate.person.as_str())
            || Some(answer.subject.as_str()) != bound.body["fields"]["ask"].as_str()
            || fields["episode"] != bound.body["fields"]["episode"]
            || fields["answer"]["id"] != "attach" || fields["answer"]["outcome"] != "accept"
            || fields["resolved_by"].as_str() == Some("deadline")
            || ["delegation","acted_for","sender_class","assistant_grant_id"].iter().any(|key|fields.get(*key).is_some())
            || answer.accepted_at_unix_ms < prepared.prepared_at_unix_ms || answer.accepted_at_unix_ms >= prepared.expires_at_unix_ms
        { return Err(refused("only the exact accepting person answer authorizes attach")); }
        let receipt = one(&tx,&format!("custom/client/native-attach-answer-{answer_id}"),"custom.client.pairing-native-attach-answer-admitted")?
            .ok_or_else(||refused("person answer has no paired-device admission receipt"))?;
        let authority = &receipt.body["fields"];
        if receipt.origin != answer.origin || receipt.actor != answer.actor
            || authority["answer_id"] != answer.id || authority["ask"] != answer.subject
            || authority["episode"] != fields["episode"] || authority["person"] != prepared.candidate.person
            || authority["preparation"] != id || authority["candidate_digest"] != digest
            || !crate::pairing::operation_is_trusted(&tx,&receipt.id,&receipt.origin,receipt.actor.as_deref(),authority).map_err(internal)?
        { return Err(refused("person approval provenance does not bind this candidate")); }
        let grant_id = authority["grant_id"].as_str().ok_or_else(||refused("approval has no paired grant"))?;
        let grant = claim_by_id_tx(&tx,grant_id).map_err(internal)?.ok_or_else(||refused("paired grant is absent"))?;
        let grant_fields = &grant.body["fields"];
        if grant.kind != "custom.client.pairing-completed" || grant.subject != authority["grant_subject"]
            || grant_fields["person_id"] != prepared.candidate.person
            || !grant_fields["scopes"].as_array().is_some_and(|scopes|scopes.iter().any(|scope|scope=="control.work"))
            || !crate::pairing::completion_is_trusted(&tx,&grant.id,&grant.origin,grant.actor.as_deref(),grant_fields,now).map_err(internal)?
            || crate::pairing::completion_is_revoked(&tx,&grant.subject,&grant.id,&grant.origin).map_err(internal)?
        { return Err(refused("paired person authority expired, changed or was revoked")); }
        let process = &prepared.candidate.evidence.process;
        let process_key = smallclaims::hash::canonical_hash(&(&prepared.candidate.host,&process.boot_id,process.pid,process.start_token)).map_err(internal)?;
        let native_key = smallclaims::hash::canonical_hash(&(&prepared.candidate.host,&prepared.candidate.evidence.transcript.native_id)).map_err(internal)?;
        if conflict(&tx,"agent",&prepared.candidate.agent)? || conflict(&tx,"answer",answer_id)?
            || conflict(&tx,"process",&process_key)? || conflict(&tx,"native",&native_key)?
            || tx.query_row("SELECT EXISTS(SELECT 1 FROM desired WHERE subject=?1 AND kind!='stop')",[&prepared.candidate.agent],|row|row.get::<_,bool>(0)).map_err(internal)?
            || retirements::record(&tx,&prepared.candidate.agent,None)?.is_some()
        { return Err(refused("agent, process, native session or person answer is already reserved")); }
        let binding = AttachedIncarnation {id:uuid::Uuid::new_v4().to_string(),preparation_id:prepared.id.clone(),
            candidate:prepared.candidate,candidate_digest:digest.into(),answer_id:answer_id.into(),approval_receipt:receipt.id.clone(),
            request_key:request_key.into(),process_key,native_key,committed_at_unix_ms:now,
            ownership:Ownership::External,phase:AttachPhase::ChannelPending,epoch:1};
        append(&tx,self.origin(),&format!("custom/native-attach/{id}"),COMMITTED,&binding,
            &[prepared_claim.id,bound.id,answer.id,receipt.id])?;
        witness.revalidate().map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(binding)
    }
}
