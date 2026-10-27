//! A parent session's subagent transcripts (client contract `conversation-blocks.v1` §1b, q2).
//! OMP stores each subagent's transcript beside its parent's, in a directory named after the
//! parent file: `<parent>.jsonl` -> `<parent>/<task id>.jsonl`. A child session id encodes its
//! native parent, task, and root routing session, so gateways route it to the parent's owner
//! without storing an index. Open or bookmarked links can outlive their generating gateway.
//! The boundary reader accepts the old two-field encoding; writers emit only three fields.
use crate::external_sessions::ExternalSession;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Child ids extend the external session prefix, so existing `session/external-` routing keeps
/// working, while staying distinct from every inventoried session id (a plain hash).
const CHILD_PREFIX: &str = "session/external-child-";

/// The reversible id of a subagent conversation, including its root owner's routing key.
pub fn child_session_id(
    parent: &ExternalSession,
    task_id: &str,
    routing_parent_session_id: &str,
) -> String {
    let routing_parent = child_parts(routing_parent_session_id)
        .map(|parts| parts.routing_parent_session_id)
        .unwrap_or_else(|| routing_parent_session_id.to_owned());
    format!(
        "{CHILD_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(json!([parent.id, task_id, routing_parent]).to_string())
    )
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChildParts {
    pub external_parent_id: String,
    pub task_id: String,
    pub routing_parent_session_id: String,
}

/// Decode a child link at the boundary, normalizing legacy two-field links to a root routing
/// key. A routing parent is native or managed, never another child.
pub fn child_parts(session_id: &str) -> Option<ChildParts> {
    let encoded = session_id.strip_prefix(CHILD_PREFIX)?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    let fields = serde_json::from_slice::<Vec<String>>(&decoded).ok()?;
    let (parent, task, routing_parent) = match fields.len() {
        3 => {
            let [parent, task, routing_parent]: [String; 3] = fields.try_into().ok()?;
            (parent, task, Some(routing_parent))
        }
        2 => {
            let [parent, task]: [String; 2] = fields.try_into().ok()?;
            (parent, task, None)
        }
        _ => return None,
    };
    // Nested encodings grow geometrically in bytes. Decode each parent once so work stays
    // proportional to the link size, without imposing a depth limit absent from the writer.
    let parent_parts = child_parts(&parent);
    let routing_parent = routing_parent.unwrap_or_else(|| {
        parent_parts.as_ref().map_or_else(
            || parent.clone(),
            |parts| parts.routing_parent_session_id.clone(),
        )
    });
    let external = |id: &str| {
        id.strip_prefix("session/external-")
            .is_some_and(|hash| hash.len() == 24 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let managed = routing_parent.strip_prefix("session/").is_some_and(|hash| {
        hash.len() == 24 && hash.bytes().all(|b| b.is_ascii_hexdigit())
    });
    if !(external(&parent) || parent_parts.is_some())
        || !valid_task_id(&task)
        || !(external(&routing_parent) || managed)
    {
        return None;
    }
    if let Some(parent_parts) = parent_parts
        && parent_parts.routing_parent_session_id != routing_parent
    {
        return None;
    }
    Some(ChildParts {
        external_parent_id: parent,
        task_id: task,
        routing_parent_session_id: routing_parent,
    })
}

/// A task id must stay one path component: no separators, no traversal, no dot-prefixed name.
fn valid_task_id(task_id: &str) -> bool {
    !task_id.is_empty()
        && task_id.len() <= 256
        && !task_id.starts_with('.')
        && !task_id.contains(['/', '\\', '\0'])
}

/// The child transcript the harness writes for `task_id`, when it exists as a regular file.
/// The entry itself is never followed through a symlink: the harness writes it as a file, so a
/// link there could only point a client at a file outside the session. The task directory may
/// be a link, as st's managed seats link the companion directory to the original transcript's.
pub fn child_transcript(parent: &ExternalSession, task_id: &str) -> Option<PathBuf> {
    if !valid_task_id(task_id) {
        return None;
    }
    let path = parent
        .transcript
        .with_extension("")
        .join(format!("{task_id}.jsonl"));
    std::fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|_| path)
}

/// Map a child session id back to its session. The parent must still resolve through normal
/// discovery, with the same owner rules as any other native session, and the child transcript
/// must exist beside it; otherwise the id names no conversation.
pub fn resolve_child(
    home: Option<&Path>,
    session_id: &str,
) -> anyhow::Result<Option<ExternalSession>> {
    let Some(parts) = child_parts(session_id) else {
        return Ok(None);
    };
    // Managed roots require the owner's binding, not local native discovery.
    if !parts.routing_parent_session_id.starts_with("session/external-") {
        return Ok(None);
    }
    let parent = if child_parts(&parts.external_parent_id).is_some() {
        resolve_child(home, &parts.external_parent_id)?
    } else {
        crate::external_sessions::find(home, &parts.external_parent_id)?
    };
    let Some(parent) = parent else {
        return Ok(None);
    };
    resolve_child_of(&parent, session_id, &parts.task_id)
}

/// Build a child source after the caller has resolved and verified its parent binding.
pub fn resolve_child_of(
    parent: &ExternalSession,
    session_id: &str,
    task_id: &str,
) -> anyhow::Result<Option<ExternalSession>> {
    let Some(transcript) = child_transcript(parent, task_id) else {
        return Ok(None);
    };
    let metadata = std::fs::symlink_metadata(&transcript)?;
    let updated = metadata.modified().ok();
    let started = metadata.created().ok().or(updated);
    let unix = |time: Option<std::time::SystemTime>| {
        time.map(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        })
        .unwrap_or_default()
    };
    Ok(Some(ExternalSession {
        id: session_id.to_owned(),
        revision: String::new(),
        driver: parent.driver,
        native_id: format!("{}/{task_id}", parent.native_id),
        transcript,
        codex_home: parent.codex_home.clone(),
        cwd: parent.cwd.clone(),
        title: Some(format!(
            "{} · {task_id}",
            parent.title.as_deref().unwrap_or(&parent.native_id)
        )),
        started_at_unix_ms: unix(started),
        updated_at_unix_ms: unix(updated),
        process: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_sessions::ExternalDriver;
    use sha2::{Digest as _, Sha256};

    fn parent(root: &Path) -> ExternalSession {
        let transcript = root.join(".omp/agent/sessions/example/native-test.jsonl");
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::write(
            &transcript,
            concat!(
                "{\"type\":\"session\",\"id\":\"native-test\",\"cwd\":\"/work/example\",\"timestamp\":\"2026-10-06T12:00:00Z\"}\n",
                "{\"type\":\"message\",\"id\":\"message-test\",\"timestamp\":\"2026-10-06T12:00:01Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"parent only\"}]}}\n",
            ),
        )
        .unwrap();
        ExternalSession {
            id: format!(
                "session/external-{}",
                &hex::encode(Sha256::digest(b"omp:native-test"))[..24]
            ),
            revision: "revision-test".into(),
            driver: ExternalDriver::Omp,
            native_id: "native-test".into(),
            transcript,
            codex_home: None,
            cwd: None,
            title: None,
            started_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            process: None,
        }
    }

    fn child_file(parent: &ExternalSession, task_id: &str) -> PathBuf {
        let path = child_transcript_path(parent, task_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"child\",\"cwd\":\"/work/example\",\"timestamp\":\"2026-10-06T12:01:00Z\"}\n",
                "{\"type\":\"message\",\"id\":\"child-message\",\"timestamp\":\"2026-10-06T12:01:01Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"child only\"}]}}\n",
            ),
        )
        .unwrap();
        path
    }

    fn child_transcript_path(parent: &ExternalSession, task_id: &str) -> PathBuf {
        parent
            .transcript
            .with_extension("")
            .join(format!("{task_id}.jsonl"))
    }

    #[test]
    fn traversal_and_hidden_task_ids_never_leave_the_parent_directory() {
        let root = tempfile::tempdir().unwrap();
        let parent = parent(root.path());
        for task in [
            "",
            "..",
            "../escape",
            "sub/dir",
            "back\\slash",
            ".hidden",
            ".",
        ] {
            assert!(child_transcript(&parent, task).is_none(), "{task:?}");
        }
        // A directory in the child slot is not a transcript, even with a valid id.
        std::fs::create_dir_all(child_transcript_path(&parent, "directory")).unwrap();
        assert!(child_transcript(&parent, "directory").is_none());
        assert!(child_file(&parent, "task-1").is_file());
        assert_eq!(
            child_transcript(&parent, "task-1"),
            Some(child_transcript_path(&parent, "task-1"))
        );
    }

    #[test]
    fn symlinked_child_transcripts_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let parent = parent(root.path());
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let target = outside.join("elsewhere.jsonl");
        std::fs::write(&target, "{\"type\":\"session\",\"id\":\"elsewhere\"}\n").unwrap();
        // A symlinked transcript in the task directory is not the child's transcript.
        let link = child_transcript_path(&parent, "linked");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(link.is_file(), "the link resolves");
        assert!(child_transcript(&parent, "linked").is_none());
        let id = child_session_id(&parent, "linked", &parent.id);
        assert!(resolve_child(Some(root.path()), &id).unwrap().is_none());
        // A regular child file beside it still resolves.
        assert!(child_transcript(&parent, "task-1").is_none());
        let file = child_file(&parent, "task-1");
        assert_eq!(child_transcript(&parent, "task-1"), Some(file));
    }

    #[test]
    fn child_ids_round_trip_through_resolution_without_stored_state() {
        let root = tempfile::tempdir().unwrap();
        let parent = parent(root.path());
        let expected = child_file(&parent, "task-1");
        let id = child_session_id(&parent, "task-1", &parent.id);
        assert!(id.starts_with("session/external-child-"));
        assert_eq!(
            child_parts(&id),
            Some(ChildParts {
                external_parent_id: parent.id.clone(),
                task_id: "task-1".to_owned(),
                routing_parent_session_id: parent.id.clone(),
            })
        );
        // The id holds no path separator, so it stays one route segment.
        assert!(!id.trim_start_matches("session/").contains('/'));
        let resolved = resolve_child(Some(root.path()), &id).unwrap().unwrap();
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.transcript, expected);
        assert_eq!(resolved.driver, ExternalDriver::Omp);
        assert_eq!(resolved.native_id, "native-test/task-1");
        // Without the file, or for a task that never ran, the id names nothing.
        std::fs::remove_file(&expected).unwrap();
        assert!(resolve_child(Some(root.path()), &id).unwrap().is_none());
        let missing = child_session_id(&parent, "never-ran", &parent.id);
        assert!(
            resolve_child(Some(root.path()), &missing)
                .unwrap()
                .is_none()
        );
        // Unknown shapes are not child ids at all.
        assert!(child_parts("session/external-notachild").is_none());
        assert!(child_parts("session/external-child-not-base64!").is_none());
    }

    #[test]
    fn managed_roots_and_descendants_keep_the_routing_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut parent = parent(root.path());
        let routing = "session/0123456789abcdef01234567";
        let child = child_session_id(&parent, "task-1", routing);
        let parts = child_parts(&child).unwrap();
        assert_eq!(parts.external_parent_id, parent.id);
        assert_eq!(parts.routing_parent_session_id, routing);
        assert!(resolve_child(Some(root.path()), &child).unwrap().is_none());
        parent.id = child.clone();
        let grandchild = child_session_id(&parent, "task-2", &child);
        let parts = child_parts(&grandchild).unwrap();
        assert_eq!(parts.external_parent_id, child);
        assert_eq!(parts.routing_parent_session_id, routing);
    }

    #[test]
    fn legacy_child_links_decode_and_resolve_but_writers_emit_three_fields() {
        let root = tempfile::tempdir().unwrap();
        let parent = parent(root.path());
        let expected = child_file(&parent, "task-1");
        let legacy = format!("{CHILD_PREFIX}{}", URL_SAFE_NO_PAD.encode(
            json!([parent.id, "task-1"]).to_string(),
        ));
        let parts = child_parts(&legacy).unwrap();
        assert_eq!(parts.routing_parent_session_id, parent.id);
        assert_eq!(resolve_child(Some(root.path()), &legacy).unwrap().unwrap().transcript, expected);
        let descendant = format!("{CHILD_PREFIX}{}", URL_SAFE_NO_PAD.encode(
            json!([legacy, "task-2"]).to_string(),
        ));
        assert_eq!(child_parts(&descendant).unwrap().routing_parent_session_id, parent.id);
        let current = child_session_id(&parent, "task-1", &legacy);
        let fields: Vec<String> = serde_json::from_slice(
            &URL_SAFE_NO_PAD.decode(current.strip_prefix(CHILD_PREFIX).unwrap()).unwrap(),
        ).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[2], parent.id);
    }

    #[test]
    fn malformed_child_ids_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let parent = parent(root.path());
        for payload in [
            json!([parent.id, "task", parent.id, "extra"]),
            json!([parent.id, "../escape", parent.id]),
            json!([parent.id, ".hidden", parent.id]),
            json!(["session/not-external", "task", parent.id]),
            json!([parent.id, "task", "session/not-managed"]),
            json!([parent.id, "task", 123]),
        ] {
            let id = format!("{CHILD_PREFIX}{}", URL_SAFE_NO_PAD.encode(payload.to_string()));
            assert!(child_parts(&id).is_none(), "{payload}");
        }
    }

    #[test]
    fn deeply_nested_legacy_links_decode_each_parent_once() {
        let root = "session/external-0123456789abcdef01234567";
        let mut descendant = root.to_owned();
        for _ in 0..24 {
            descendant = format!("{CHILD_PREFIX}{}", URL_SAFE_NO_PAD.encode(
                json!([descendant, "task"]).to_string(),
            ));
        }
        assert_eq!(child_parts(&descendant).unwrap().routing_parent_session_id, root);
    }

    #[test]
    fn deeply_nested_current_ids_round_trip_through_the_writer_and_reader() {
        let root = tempfile::tempdir().unwrap();
        let mut native = parent(root.path());
        let routing_root = native.id.clone();
        for _ in 0..33 {
            native.id = child_session_id(&native, "task", &routing_root);
        }
        assert_eq!(child_parts(&native.id).unwrap().routing_parent_session_id, routing_root);
    }
}
