//! Bounded normalized GitHub evidence. Only a committed capture makes observations visible.
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const FACT_KIND: &str = "github.fact.observed";
pub const COMMIT_KIND: &str = "github.capture.committed";
pub const AUTHORITY_KIND: &str = "github.capture.authority";
pub const FENCED_KIND: &str = "github.capture.fenced";
pub const FACT_FAMILY: &str = "github-fact";
pub const COMMIT_FAMILY: &str = "github-capture";
pub const AUTHORITY_FAMILY: &str = "github-source";
pub const MAX_FACT_BYTES: usize = 1024;
pub const MAX_COMMIT_BYTES: usize = 2048;
pub const MAX_REFERENCES: usize = 4;
pub const VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub host: String,
    pub id: String,
    pub generation: u64,
}

/// Corrected canonical snapshot, shared by capture readers and pure measurements.
/// Only committed projection readers construct this input; raw staged claims are invisible.
#[derive(Clone, Debug)]
pub struct Projection {
    pub source: Source,
    pub revision: String,
    pub frontier_ms: u64,
    pub roster: Roster,
    pub pull_requests: Vec<PullRequestFact>,
    pub lifecycle: Vec<LifecycleFact>,
    pub merges: Vec<MergeFact>,
    /// Indexed final-ready proof by (repository, PR). Present None is unproven;
    /// absent keys require the complete lifecycle vector, never truncated events.
    pub ready_cycles: std::collections::BTreeMap<(String, String), Option<u64>>,
    pub coverage: Vec<Coverage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryFact {
    pub id: String,
    pub locator: String,
    pub is_private: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestFact {
    pub id: String,
    pub repository_id: String,
    pub number: u64,
    pub author_login: Option<String>,
    pub state: PullRequestState,
    pub is_draft: bool,
    pub created_as_draft: Option<bool>,
    pub created_at_ms: u64,
    pub closed_at_ms: Option<u64>,
    pub merged_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleKind {
    Ready,
    Draft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFact {
    pub id: String,
    pub repository_id: String,
    pub pull_request_id: String,
    pub kind: LifecycleKind,
    pub occurred_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeFact {
    pub pull_request_id: String,
    pub repository_id: String,
    pub merged_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactFamily {
    Repository,
    PullRequest,
    Lifecycle,
    Merge,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retraction {
    pub family: FactFamily,
    pub repository_id: String,
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", content = "image", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactImage {
    Repository(RepositoryFact),
    PullRequest(PullRequestFact),
    Lifecycle(LifecycleFact),
    Merge(MergeFact),
    Retraction(Retraction),
}

impl FactImage {
    pub fn identity(&self) -> (FactFamily, &str, &str) {
        match self {
            Self::Repository(fact) => (FactFamily::Repository, &fact.id, &fact.id),
            Self::PullRequest(fact) => (FactFamily::PullRequest, &fact.repository_id, &fact.id),
            Self::Lifecycle(fact) => (FactFamily::Lifecycle, &fact.repository_id, &fact.id),
            Self::Merge(fact) => (FactFamily::Merge, &fact.repository_id, &fact.pull_request_id),
            Self::Retraction(fact) => (fact.family, &fact.repository_id, &fact.id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedFact {
    pub version: u8,
    pub source: Source,
    pub observed_at_ms: u64,
    pub supersedes: Option<String>,
    pub fact: FactImage,
}

impl ObservedFact {
    /// Observation time is provenance, not revision identity: identical polls do not grow history.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        let revision = serde_json::to_vec(&(self.version, &self.source, &self.supersedes, &self.fact))?;
        Ok(hex::encode(Sha256::digest(revision)))
    }

    pub fn subject(&self) -> Result<String, serde_json::Error> {
        Ok(format!("{FACT_FAMILY}/{}", self.digest()?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactReference {
    pub claim_id: String,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub repository_id: String,
    pub family: FactFamily,
    pub complete: bool,
    pub authoritative: bool,
    pub history_from_ms: u64,
    pub through_ms: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanCheckpoint {
    pub repository_id: String,
    pub pass_started_at_ms: u64,
    pub cursor: Option<String>,
    pub pass_complete: bool,
    pub pull_request_id: Option<String>,
    pub upstream_updated_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureCommit {
    pub version: u8,
    pub source: Source,
    pub epoch: u64,
    pub authority_grant: String,
    pub sequence: u64,
    pub predecessor: Option<String>,
    pub facts: Vec<FactReference>,
    pub checkpoint: Option<ScanCheckpoint>,
    pub coverage: Vec<Coverage>,
    pub observed_at_ms: u64,
}

impl CaptureCommit {
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(self)?)))
    }

    pub fn subject(&self) -> Result<String, serde_json::Error> {
        Ok(format!("{COMMIT_FAMILY}/{}", self.digest()?))
    }
}

/// A successor cannot write until a prior writer's stopped frontier is explicitly admitted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityGrant {
    pub version: u8,
    pub source: Source,
    pub member: String,
    pub epoch: u64,
    pub predecessor_grant: Option<String>,
    pub final_predecessor_commit: Option<String>,
    pub fencing_receipt: Option<String>,
}

/// Policy membership, not source identity; unknown historical membership is never backdated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterMember {
    pub repository_id: String,
    pub effective_from_ms: u64,
    pub effective_until_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub revision: String,
    pub members: Vec<RosterMember>,
}

impl Roster {
    pub fn contains(&self, repository_id: &str, at_ms: u64) -> bool {
        self.members.iter().any(|member| {
            member.repository_id == repository_id
                && member.effective_from_ms <= at_ms
                && member.effective_until_ms.is_none_or(|end| at_ms < end)
        })
    }
}

/// A source declaration contains routing, not credentials or an independently writable cursor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDefinition {
    pub source: Source,
    /// Disabled declarations may be published before the operator admits a root grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_grant: Option<String>,
    pub repositories: Vec<String>,
    pub enabled: bool,
    pub checkpoint_interval_ms: u64,
    pub replicate_private: bool,
}

pub fn source_key(source: &Source) -> Result<String, serde_json::Error> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(source)?)))
}

fn valid_source(source: &Source) -> bool {
    !source.id.is_empty()
        && source.id.len() <= 128
        && source.generation > 0
        && source.generation <= i64::MAX as u64
        && !source.host.is_empty()
        && source.host.len() <= 255
        && source.host.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-')
}

fn digest_string(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Validate encoded fields before admission, including replication; byte limits include `record`.
pub fn validate(subject: &str, kind: &str, fields: &std::collections::BTreeMap<String, serde_json::Value>) -> Result<(), super::ValidationError> {
    if !matches!(kind, FACT_KIND | COMMIT_KIND | AUTHORITY_KIND | FENCED_KIND) {
        return Ok(());
    }
    let fail = |message: String| super::error("invalid-github-capture", message);
    let record = fields.get("record").ok_or_else(|| fail("capture claim needs record".into()))?;
    let ceiling = if kind == FACT_KIND { MAX_FACT_BYTES } else { MAX_COMMIT_BYTES };
    struct Size(usize);
    impl std::io::Write for Size {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut size = Size(0);
    serde_json::to_writer(&mut size, fields).map_err(|error| fail(error.to_string()))?;
    if size.0 > ceiling { return Err(fail(format!("capture fields exceed {ceiling} bytes"))); }
    match kind {
        FACT_KIND => {
            let fact = ObservedFact::deserialize(record).map_err(|error| fail(error.to_string()))?;
            let (_, repository, id) = fact.fact.identity();
            if fact.version != VERSION || !valid_source(&fact.source)
                || repository.is_empty() || id.is_empty() || repository.len() > 256 || id.len() > 256
                || fact.supersedes.as_deref().is_some_and(|digest| !digest_string(digest))
                || fact.subject().map_err(|error| fail(error.to_string()))? != subject
            { return Err(fail("invalid fact identity, revision or source".into())); }
        }
        COMMIT_KIND => {
            let commit = CaptureCommit::deserialize(record).map_err(|error| fail(error.to_string()))?;
            let mut references = std::collections::BTreeSet::new();
            if commit.version != VERSION || !valid_source(&commit.source) || commit.epoch == 0
                || commit.sequence == 0 || commit.sequence > i64::MAX as u64
                || commit.authority_grant.is_empty() || commit.authority_grant.len() > 256
                || commit.facts.len() > MAX_REFERENCES
                || commit.subject().map_err(|error| fail(error.to_string()))? != subject
                || commit.facts.iter().any(|reference| {
                    reference.claim_id.is_empty() || !digest_string(&reference.digest)
                        || !references.insert(&reference.claim_id)
                })
                || commit.coverage.iter().any(|coverage| {
                    coverage.repository_id.is_empty() || coverage.through_ms < coverage.history_from_ms
                        || (coverage.complete && coverage.error.is_some())
                })
                || commit.checkpoint.as_ref().is_some_and(|checkpoint| {
                    checkpoint.cursor.as_ref().is_some_and(|cursor| cursor.len() > 768)
                        || checkpoint.pull_request_id.is_some() != checkpoint.upstream_updated_at_ms.is_some()
                })
            { return Err(fail("invalid capture identity, checkpoint, references or coverage".into())); }
        }
        AUTHORITY_KIND => {
            let grant = AuthorityGrant::deserialize(record).map_err(|error| fail(error.to_string()))?;
            let root = grant.epoch == 1;
            if grant.version != VERSION || !valid_source(&grant.source) || grant.epoch == 0
                || grant.member.is_empty() || !subject.starts_with("github-source/")
                || (root && (grant.predecessor_grant.is_some() || grant.final_predecessor_commit.is_some() || grant.fencing_receipt.is_some()))
                || (!root && (grant.predecessor_grant.is_none() || grant.fencing_receipt.as_ref().is_none_or(String::is_empty)))
            { return Err(fail("authority requires a member and an explicit fenced predecessor".into())); }
        }
        FENCED_KIND => {
            let receipt = FenceReceipt::deserialize(record).map_err(|error| fail(error.to_string()))?;
            if receipt.version != VERSION || !valid_source(&receipt.source) || receipt.epoch == 0
                || receipt.member.is_empty() || receipt.authority_grant.is_empty() || !subject.starts_with("github-source/")
            { return Err(fail("fencing receipt needs explicit authority and stopped frontier".into())); }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceReceipt {
    pub version: u8,
    pub source: Source,
    pub member: String,
    pub epoch: u64,
    pub authority_grant: String,
    pub final_commit: Option<String>,
    pub stopped_at_ms: u64,
}

#[path = "github_capture_declarations.rs"]
pub mod declarations;
