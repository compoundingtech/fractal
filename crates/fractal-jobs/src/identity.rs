//! Inactive execution identity. References must be resolved before submission, never at launch.
use crate::JobRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Input from a modified (dirty) tree MUST use `Content` with its content digest,
/// never `Git { repo, commit }`: the same HEAD is not equal dirty-tree input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SourceReference {
    Git { repo: String, commit: ExactCommit },
    Content { sha256: [u8; 32] },
}

/// Full object IDs only: branch names and abbreviated commits cannot be represented.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algorithm", content = "digest", rename_all = "kebab-case")]
pub enum ExactCommit {
    Sha1([u8; 20]),
    Sha256([u8; 32]),
}

/// The complete environment, with no inherited host variables. Digest resolution is external.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum EnvironmentReference {
    Exact(BTreeMap<String, String>),
    Sha256([u8; 32]),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReference {
    /// Passed directly to exec, without shell parsing or Unicode normalization.
    pub argv: Vec<String>,
    /// POSIX path relative to the source root; `.` denotes that root.
    pub cwd: RelativeCwd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    EmptyArgv,
    EmptyExecutable,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CommandError {}

impl CommandReference {
    /// Validate before submission; arguments after the executable may be empty.
    pub fn validate(&self) -> Result<(), CommandError> {
        match self.argv.first() {
            None => Err(CommandError::EmptyArgv),
            Some(executable) if executable.is_empty() => Err(CommandError::EmptyExecutable),
            Some(_) => Ok(()),
        }
    }
}

/// A source-relative POSIX path; parent traversal is not an execution identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RelativeCwd(String);

impl TryFrom<String> for RelativeCwd {
    type Error = &'static str;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\0')
            || path.split('/').any(|part| part == "..")
        {
            return Err("cwd must be relative to the source without parent traversal");
        }
        Ok(Self(path))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    pub source: SourceReference,
    pub environment: EnvironmentReference,
    pub command: CommandReference,
    /// Resource and policy declarations are part of request identity too.
    pub resources: JobRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestDigest(pub [u8; 32]);

impl ExecutionRequest {
    /// Version 1 canonical UTF-8 JSON: fixed struct-field order, sorted environment
    /// keys, compact encoding, exact string/codepoint and argv order preservation.
    /// The version prefix domain-separates this contract from other SHA-256 uses.
    /// No floats, arbitrary JSON maps or fallible custom serializers occur here.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = b"fractal-jobs-request-v1\n".to_vec();
        serde_json::to_writer(&mut bytes, self).expect("execution request is JSON serializable");
        bytes
    }

    pub fn digest(&self) -> RequestDigest {
        RequestDigest(Sha256::digest(self.canonical_bytes()).into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyKey {
    pub requester: String,
    pub key: String,
    pub request_digest: RequestDigest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdempotencyDecision<J> {
    NewJob,
    ExistingJob(J),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum IdempotencyRefusal {
    IdempotencyDigestMismatch,
}

/// `existing` must be the lookup for exactly `(requester, key)`, not a global key.
/// This pure decision does not reserve a key: atomic persistent receipts, including
/// racing submissions and retention, belong to unit 4 and are NOT implemented here.
pub fn decide_idempotency<J>(
    request: &IdempotencyKey,
    existing: Option<(RequestDigest, J)>,
) -> Result<IdempotencyDecision<J>, IdempotencyRefusal> {
    match existing {
        None => Ok(IdempotencyDecision::NewJob),
        Some((digest, job)) if digest == request.request_digest => {
            Ok(IdempotencyDecision::ExistingJob(job))
        }
        Some(_) => Err(IdempotencyRefusal::IdempotencyDigestMismatch),
    }
}
