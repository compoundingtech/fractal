//! Shared session checks for thread admission and discovery.

use super::{ApiError, AppState, ClientSession, forbidden, require_scope};
use crate::store::thread_metadata::PersonAuthority;
use serde_json::{Value, json};

pub fn read_person(session: &ClientSession) -> Result<&str, ApiError> {
    require_scope(session, "read.projections")?;
    crate::api::records::client_message_person(session)
        .ok_or_else(|| forbidden("thread reads require a concrete person"))
}

pub fn edit_authority(
    current: &ClientSession,
) -> Result<PersonAuthority<'_>, ApiError> {
    crate::api::records::client_message_person(current)
        .ok_or_else(|| forbidden("thread edits require a concrete person"))?;
    Ok(
        match (
            current.transport,
            current.pairing_grant.as_deref(),
            current.pairing_completion.as_deref(),
        ) {
            ("unix", None, None) => PersonAuthority::TrustedLocal,
            ("local-web", None, None) => PersonAuthority::LocalWeb {
                expires_at: current
                    .grant_expires_at
                    .ok_or_else(|| forbidden("the web credential is unavailable"))?,
            },
            (_, Some(grant), Some(completion)) => PersonAuthority::Paired {
                grant,
                completion,
                actor: &current.actor,
            },
            _ => return Err(forbidden("the thread credential is unavailable")),
        },
    )
}

pub fn browser_message_authority<'a>(
    state: &AppState,
    session: &'a ClientSession,
) -> Option<PersonAuthority<'a>> {
    let PersonAuthority::Paired { grant, completion, actor } = edit_authority(session).ok()? else {
        return None;
    };
    if !session.allows("control.messages") {
        return None;
    }
    crate::store::thread_metadata::browser_send_authority(
        &state.store, &session.authority_actor, grant, completion, actor,
    ).ok()
}

pub fn bound_agent(session: &ClientSession) -> Option<&str> {
    session.bound_agent.as_deref().filter(|bound| {
        session.transport == "unix"
            && session.assistant_write.is_none()
            && session.authority_actor == *bound
            && bound.strip_prefix("agent/").is_some_and(|name| !name.is_empty())
    })
}

pub(super) fn capabilities(state: &AppState, session: &ClientSession) -> impl Iterator<Item = Value> {
    let store = &state.store;
    let available = store.graph.read_snapshot_unindexed(|| {
        Ok(store.thread_write_rollout(&store.readers.get()).is_ok())
    }).unwrap_or(false);
    let read = read_person(session).is_ok();
    let edit = session.allows("control.messages") && edit_authority(session).is_ok();
    let browser_send = read && edit && browser_message_authority(state, session).is_some();
    let bound = bound_agent(session).is_some();
    let reply = edit || bound && session.allows("control.messages");
    let link = bound && session.allows("control.work");
    [
        ("threads-v1", 2, read),
        ("thread-summary-feed", 1, read),
        ("thread-browser-send", 1, browser_send),
        ("thread.read", 0, read && edit),
        ("thread.start", 0, edit),
        ("thread.reply", 0, reply),
        ("thread.rename", 0, edit),
        ("thread.close", 0, edit),
        ("thread.reopen", 0, edit),
        ("thread.link-ask", 0, link),
    ].into_iter().map(move |(id, version, allowed)| {
        let state = if !available {
            "unavailable"
        } else if allowed {
            "granted"
        } else {
            "ungranted"
        };
        json!({ "id": id, "version": version, "state": state })
    })
}
