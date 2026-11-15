//! Private, content-addressed invocation inputs/transcripts. Bytes never enter claims or traces.
use std::{fs, io::{BufRead, BufReader, Write}, path::Path};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::{model::ClaimInput, store::Store};
use fractal_delivery_base::blobs::BlobDir;

const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const RETENTION_MS: u128 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InvocationArtifact {
    pub invocation_id: String,
    pub kind: String,
    pub sha256: String,
    pub source_sha256: String,
    pub bytes: u64,
    pub captured_at_unix_ms: u128,
    pub expires_at_unix_ms: u128,
    pub redacted_bytes: u64,
    pub truncated_characters: u64,
    pub policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_claim: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
}

fn directory(state: &Path) -> BlobDir { BlobDir::at(state.join("invocation-artifacts")) }
fn sanitize(value: &mut Value, redacted: &mut u64, truncated: &mut u64) {
    match value {
        Value::String(text) => {
            let (safe, withheld, shortened) = fractal_runtime::transcript::normalize_conversation_text(text);
            *text = safe;
            *redacted = redacted.saturating_add(withheld);
            *truncated = truncated.saturating_add(shortened);
        }
        Value::Array(values) => for value in values { sanitize(value, redacted, truncated); },
        Value::Object(values) => for value in values.values_mut() { sanitize(value, redacted, truncated); },
        _ => {}
    }
}

/// Capture a stopped provider's JSONL transcript. The caller must supply the exact incarnation's
/// positively stopped file; this routine never discovers a mutable current-session path.
pub fn capture_transcript(state: &Path, invocation: &str, source: &Path, at: u128) -> Result<InvocationArtifact> {
    let input = fs::File::open(source).context("open invocation transcript")?;
    anyhow::ensure!(input.metadata()?.len() <= MAX_BYTES, "invocation transcript exceeds 64 MiB");
    let private = state.join("invocation-artifacts");
    fs::create_dir_all(&private)?;
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))?;
    directory(state).sweep(std::time::Duration::from_millis(RETENTION_MS as u64));
    let temporary = tempfile::Builder::new().prefix(".capture-").tempfile_in(&private)?;
    let mut output = temporary.reopen()?;
    let mut source_hash = Sha256::new();
    let mut safe_hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut redacted_bytes = 0;
    let mut truncated_characters = 0;
    let mut reader = BufReader::new(input);
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line)? != 0 {
        anyhow::ensure!(line.len() as u64 <= MAX_BYTES, "invocation transcript line exceeds 64 MiB");
        source_hash.update(&line);
        if !line.iter().all(u8::is_ascii_whitespace) {
            let mut value: Value = serde_json::from_slice(&line).context("invalid invocation transcript JSONL")?;
            sanitize(&mut value, &mut redacted_bytes, &mut truncated_characters);
            let mut encoded = serde_json::to_vec(&value)?;
            encoded.push(b'\n');
            bytes = bytes.saturating_add(encoded.len() as u64);
            anyhow::ensure!(bytes <= MAX_BYTES, "redacted invocation transcript exceeds 64 MiB");
            safe_hash.update(&encoded);
            output.write_all(&encoded)?;
        }
        line.clear();
    }
    output.sync_all()?;
    drop(output);
    let sha256 = hex::encode(safe_hash.finalize());
    temporary.persist(directory(state).path(&sha256)).map_err(|error| error.error)?;
    Ok(InvocationArtifact { invocation_id: invocation.into(), kind: "transcript".into(), sha256,
        source_sha256: hex::encode(source_hash.finalize()), bytes, captured_at_unix_ms: at,
        expires_at_unix_ms: at.saturating_add(RETENTION_MS), redacted_bytes, truncated_characters,
        policy: "conversation-v1;private;7d".into(), incarnation: None, message_id: None,
        native_session_id: None, binding_claim: None, harness: None })
}

/// Capture an exact native input message, not an assertion about a provider's hidden system prompt.
pub fn capture_input(state: &Path, invocation: &str, input: &str, at: u128) -> Result<InvocationArtifact> {
    anyhow::ensure!(input.len() as u64 <= MAX_BYTES, "invocation input exceeds 64 MiB");
    directory(state).sweep(std::time::Duration::from_millis(RETENTION_MS as u64));
    let (safe, redacted_bytes, truncated_characters) = fractal_runtime::transcript::normalize_conversation_text(input);
    let sha256 = directory(state).put(safe.as_bytes())?;
    Ok(InvocationArtifact { invocation_id: invocation.into(), kind: "input".into(), sha256,
        source_sha256: hex::encode(Sha256::digest(input.as_bytes())), bytes: safe.len() as u64,
        captured_at_unix_ms: at, expires_at_unix_ms: at.saturating_add(RETENTION_MS), redacted_bytes,
        truncated_characters, policy: "conversation-v1;private;7d".into(), incarnation: None,
        message_id: None, native_session_id: None, binding_claim: None, harness: None })
}

pub fn matches_input(artifact: &InvocationArtifact, input: &str) -> bool {
    artifact.source_sha256 == hex::encode(Sha256::digest(input.as_bytes()))
}

/// The first capture owns this immutable reference. Concurrent retries return its original
/// metadata; a different source or identity returns None instead of replacing it.
/// Deletion owns the invocation/kind/hash URL, even when a later message captures the same bytes.
pub fn record_exact(store: &Store, agent: &str, artifact: &InvocationArtifact) -> Result<Option<InvocationArtifact>> {
    let key = format!("invocation-artifact:{}:{}:{}", artifact.invocation_id,
        artifact.kind, artifact.message_id.as_deref().unwrap_or(""));
    let status = if is_deleted(store, &artifact.invocation_id, &artifact.kind, &artifact.sha256)? {
        "deleted"
    } else {
        "retained"
    };
    let input = ClaimInput { subject: agent.into(), kind: "harness.invocation-artifact".into(),
        actor: None, fields: serde_json::from_value(serde_json::json!({"invocation_id":artifact.invocation_id,
            "kind":artifact.kind,"status":status,"artifact":artifact}))?, evidence: vec![],
        expected_subject: None, idempotency_key: Some(key.clone()) };
    let claim = match store.append_claim(&input) {
        Ok(claim) => claim,
        Err(error) if error.code == "idempotency-mismatch" => {
            store.operation_claim(&key)?.context("immutable artifact reference missing")?
        }
        Err(error) => return Err(error.into()),
    };
    let original: InvocationArtifact = serde_json::from_value(claim.body["fields"]["artifact"].clone())?;
    Ok((claim.subject == agent && original.source_sha256 == artifact.source_sha256
        && original.incarnation == artifact.incarnation && original.message_id == artifact.message_id
        && original.native_session_id == artifact.native_session_id
        && original.binding_claim == artifact.binding_claim && original.harness == artifact.harness)
        .then_some(original))
}

pub fn record(store: &Store, agent: &str, artifact: &InvocationArtifact) -> Result<()> {
    anyhow::ensure!(record_exact(store, agent, artifact)?.is_some(), "immutable artifact reference conflicts");
    Ok(())
}

pub fn unavailable(store: &Store, agent: &str, invocation: &str, kind: &str, reason: &str) -> Result<()> {
    let digest = hex::encode(Sha256::digest(reason.as_bytes()));
    let key = format!("invocation-artifact-unavailable:{invocation}:{kind}:{digest}");
    if store.operation_claim(&key)?.is_some() { return Ok(()); }
    // No source path or error payload is published: callers supply a bounded stable reason code.
    anyhow::ensure!(reason.len() <= 128 && reason.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "artifact unavailability reason must be a stable code");
    store.append_claim(&ClaimInput { subject:agent.into(),kind:"harness.invocation-artifact".into(),actor:None,
        fields:serde_json::from_value(serde_json::json!({"invocation_id":invocation,"kind":kind,
            "status":"unavailable","reason":reason}))?,evidence:vec![],expected_subject:None,idempotency_key:Some(key) })?;
    Ok(())
}

/// Metadata remains durable after expiry/deletion; access checks happen before calling this.
pub fn read(state: &Path, artifact: &InvocationArtifact, at: u128) -> Result<Option<Vec<u8>>> {
    anyhow::ensure!(artifact.sha256.len()==64 && artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid invocation artifact digest");
    if at >= artifact.expires_at_unix_ms { return Ok(None); }
    let bytes = directory(state).read(&artifact.sha256)?;
    if let Some(bytes) = &bytes {
        anyhow::ensure!(hex::encode(Sha256::digest(bytes)) == artifact.sha256, "invocation artifact digest mismatch");
    }
    Ok(bytes)
}

/// The URL's tombstone is permanent: a newer retained claim cannot restore read authority.
/// Readers must check this receipt independently of the latest capture's status.
pub fn is_deleted(store: &Store, invocation: &str, kind: &str, hash: &str) -> Result<bool> {
    let key = format!("invocation-artifact-delete:{invocation}:{kind}:{hash}");
    Ok(store.operation_claim(&key)?.is_some())
}

pub fn delete(state: &Path, store: &Store, agent: &str, artifact: &InvocationArtifact, at: u128) -> Result<()> {
    anyhow::ensure!(artifact.sha256.len()==64 && artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid invocation artifact digest");
    let key = format!("invocation-artifact-delete:{}:{}:{}",artifact.invocation_id,artifact.kind,artifact.sha256);
    if store.operation_claim(&key)?.is_some() { return Ok(()); }
    // A content hash may be referenced by other invocations; physical retention handles shared
    // bytes. This invocation/kind/hash URL stays revoked across later message captures.
    store.append_claim(&ClaimInput { subject:agent.into(),kind:"harness.invocation-artifact".into(),actor:None,
        fields:serde_json::from_value(serde_json::json!({"invocation_id":artifact.invocation_id,
            "kind":artifact.kind,"status":"deleted","sha256":artifact.sha256,"deleted_at_unix_ms":at}))?,
        evidence:vec![],expected_subject:None,
        idempotency_key:Some(key) })?;
    directory(state).sweep(std::time::Duration::from_millis(RETENTION_MS as u64));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_input_is_private_redacted_content_addressed_and_expires() {
        let root = tempfile::tempdir().unwrap();
        let artifact = capture_input(root.path(), "invocation/example", "hello token=fixture-secret", 100).unwrap();
        let bytes = read(root.path(), &artifact, 101).unwrap().unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "hello token=[REDACTED]");
        assert!(artifact.redacted_bytes > 0);
        assert!(read(root.path(), &artifact, artifact.expires_at_unix_ms).unwrap().is_none());
        let mut invalid = artifact;
        invalid.sha256 = "../outside".into();
        assert!(read(root.path(), &invalid, 101).is_err());
    }
    #[test]
    fn immutable_reference_keeps_first_metadata_and_rejects_changed_source_or_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let mut first = capture_input(root.path(), "invocation/example", "first envelope", 100).unwrap();
        first.incarnation = Some("exact".into());
        first.message_id = Some("message/exact".into());
        assert_eq!(record_exact(&store, "agent/example", &first).unwrap(), Some(first.clone()));
        let mut retry = first.clone();
        retry.captured_at_unix_ms = 200;
        retry.expires_at_unix_ms += 100;
        assert_eq!(record_exact(&store, "agent/example", &retry).unwrap(), Some(first.clone()));
        retry.source_sha256 = "different-source".into();
        assert!(record_exact(&store, "agent/example", &retry).unwrap().is_none());
        retry = first.clone();
        retry.incarnation = Some("other".into());
        assert!(record_exact(&store, "agent/example", &retry).unwrap().is_none());
        assert_eq!(store.claims_for("agent/example", Some("harness.invocation-artifact")).unwrap().len(), 1);
    }

    #[test]
    fn deletion_stays_authoritative_when_a_later_message_captures_the_same_redacted_bytes() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let agent = "agent/example";
        let mut first = capture_input(root.path(), "invocation/example", "hello token=first-secret", 100).unwrap();
        first.message_id = Some("message/a".into());
        record(&store, agent, &first).unwrap();
        delete(root.path(), &store, agent, &first, 200).unwrap();

        let mut second = capture_input(root.path(), "invocation/example", "hello token=second-secret", 300).unwrap();
        second.message_id = Some("message/b".into());
        assert_eq!(first.sha256, second.sha256);
        assert_ne!(first.source_sha256, second.source_sha256);
        assert_eq!(record_exact(&store, agent, &second).unwrap(), Some(second.clone()));
        // Shared bytes still exist; denial must come from the durable reference, not unlinking.
        assert!(read(root.path(), &second, 301).unwrap().is_some());
        let assert_url_revoked = || {
            // Match the API's invocation/kind/hash lookup, including its newest-reference ordering.
            let latest = store.claims_for(agent, Some("harness.invocation-artifact")).unwrap()
                .into_iter().rev().find(|claim| {
                    let fields = &claim.body["fields"];
                    fields["invocation_id"] == second.invocation_id && fields["kind"] == second.kind
                        && (fields["sha256"] == second.sha256 || fields["artifact"]["sha256"] == second.sha256)
                }).unwrap();
            assert_eq!(latest.body["fields"]["status"], "deleted");
        };
        assert_url_revoked();
        delete(root.path(), &store, agent, &second, 400).unwrap();
        assert_url_revoked();
        assert_eq!(record_exact(&store, agent, &second).unwrap(), Some(second.clone()));
        assert_url_revoked();
        // Simulate a capture that read "not deleted" before deletion but committed afterward.
        store.append_claim(&ClaimInput {
            subject: agent.into(), kind: "harness.invocation-artifact".into(), actor: None,
            fields: serde_json::from_value(serde_json::json!({
                "invocation_id":second.invocation_id,"kind":second.kind,
                "status":"retained","artifact":second,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(is_deleted(&store, &second.invocation_id, &second.kind, &second.sha256).unwrap());
        delete(root.path(), &store, agent, &second, 500).unwrap();
        assert!(is_deleted(&store, &second.invocation_id, &second.kind, &second.sha256).unwrap());
    }

    #[test]
    fn capture_sweeps_expired_bytes_but_shared_hash_refresh_preserves_new_reference() {
        let root = tempfile::tempdir().unwrap();
        let old = capture_input(root.path(), "invocation/old", "old bytes", 100).unwrap();
        let stale = std::time::SystemTime::now()
            - std::time::Duration::from_millis((RETENTION_MS + 60_000) as u64);
        fs::File::open(directory(root.path()).path(&old.sha256)).unwrap()
            .set_times(fs::FileTimes::new().set_modified(stale)).unwrap();
        let first = capture_input(root.path(), "invocation/first", "shared bytes", 200).unwrap();
        assert!(!directory(root.path()).path(&old.sha256).exists());
        let second = capture_input(root.path(), "invocation/second", "shared bytes", 300).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert!(read(root.path(), &first, first.expires_at_unix_ms).unwrap().is_none());
        assert!(read(root.path(), &second, first.expires_at_unix_ms).unwrap().is_some());
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(fs::metadata(directory(root.path()).path(&second.sha256)).unwrap()
            .permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(root.path().join("invocation-artifacts")).unwrap()
            .permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn jsonl_capture_reports_redaction_without_republishing_provider_bytes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("session.jsonl");
        fs::write(&source, "{\"message\":{\"role\":\"user\",\"content\":\"token=fixture-secret\"}}\n").unwrap();
        let artifact = capture_transcript(root.path(), "invocation/example", &source, 100).unwrap();
        let bytes = read(root.path(), &artifact, 101).unwrap().unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("fixture-secret"));
        assert_ne!(artifact.sha256, artifact.source_sha256);
    }
}
