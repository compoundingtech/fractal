//! The part of `api::client_v0::raw_terminal` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;
pub use lease::{Binding as LeaseBinding, Lease, ORIGIN_HEADER};
pub use st3_api_core::api::client_v0::raw_terminal::*;
pub use st3_api_core::api::client_v0::raw_terminal::{splice};
pub mod lease;
pub mod peek_capability;

const SUBPROTOCOL: &str = "st3.client.pty.v0";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRequest {
    runtime_incarnation: String,
    mode: st3_client::RawTerminalMode,
}

fn mode_name(mode: st3_client::RawTerminalMode) -> &'static str {
    match mode {
        st3_client::RawTerminalMode::Attach => "attach",
        st3_client::RawTerminalMode::Peek => "peek",
    }
}

fn authorize(session: &ClientSession, mode: st3_client::RawTerminalMode) -> Result<(), ApiError> {
    require_scope(session, "terminal.read")?;
    if !session.authority_actor.starts_with("person/") {
        return Err(forbidden(
            "raw terminal transport requires a concrete person",
        ));
    }
    if mode == st3_client::RawTerminalMode::Attach {
        require_scope(session, "terminal.control")?;
    }
    Ok(())
}

pub fn authorization_epoch(
    state: &AppState,
    session: &ClientSession,
) -> Result<String, ApiError> {
    lease::authorization_epoch(state, session)
}

pub async fn attachment(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AttachmentRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize(&session, request.mode)?;
    let terminal_id = client_detail_id("terminal", &id);
    let live =
        remote_terminal_live_session(&state, &terminal_subject(&id), &request.runtime_incarnation)?;
    if !live.terminal {
        return Err(validation("raw attachment requires a terminal runtime"));
    }
    if live.owner_host_id != client_host_id(&state.node)
        && state
            .client_relay
            .as_ref()
            .is_none_or(|relay| !relay.reaches(&live.owner_host_id))
    {
        return Err(remote_unavailable_for_owner(&state, &live.owner_host_id));
    }
    if request.mode == st3_client::RawTerminalMode::Peek {
        let capability = peek_capability::issue(&state, &session, &terminal_id, &live)?;
        return Ok(Json(json!({
            "terminal_id": terminal_id,
            "runtime_incarnation": live.incarnation_id,
            "owner_host_id": live.owner_host_id,
            "mode": "peek",
            "stream_capability": capability,
        })));
    }
    let attachment_id = format!("terminal-attachment/{}", new_request_id());
    let capability =
        derive_terminal_capability(&state, &session.actor, &attachment_id, &live.owner_host_id)?;
    state
        .store
        .append_claim(&ClaimInput {
            subject: terminal_attachment_subject(&attachment_id)?,
            kind: "custom.client.terminal-attached".into(),
            actor: Some(session_claim_actor(&session)),
            fields: BTreeMap::from([
                ("attachment_id".into(), json!(attachment_id)),
                ("terminal_id".into(), json!(terminal_id)),
                ("runtime_incarnation".into(), json!(live.incarnation_id)),
                ("runtime_id".into(), json!(live.runtime_id)),
                ("owner_host_id".into(), json!(live.owner_host_id)),
                ("session_actor".into(), json!(session.actor)),
                ("person_id".into(), json!(session.authority_actor)),
                ("raw_mode".into(), json!(mode_name(request.mode))),
                (
                    "raw_authorization_epoch".into(),
                    Value::Null,
                ),
                (
                    "capability_hash".into(),
                    json!(credential_digest(&capability)),
                ),
                ("expires_at_unix_ms".into(), json!(client_now_ms() + 60_000)),
            ]),
            evidence: Vec::new(),
            expected_subject: Some(None),
            idempotency_key: None,
        })
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(json!({
        "terminal_id": terminal_id,
        "runtime_incarnation": live.incarnation_id,
        "owner_host_id": live.owner_host_id,
        "mode": mode_name(request.mode),
        "stream_capability": capability,
    })))
}

#[derive(Deserialize)]
pub struct StreamQuery {
    incarnation: String,
    mode: st3_client::RawTerminalMode,
}

pub async fn stream(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    upgrade: Option<Extension<crate::otel::UpgradeContext>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let upgrade = upgrade.and_then(|Extension(context)| context.0);
    authorize(&session, query.mode)?;
    // This protocol subscribes in the upgrade request, without a JSON subscribe command.
    let first_frame = subscription_first_frame("terminal", "raw", upgrade.as_ref())
        .map(|first| first.span);
    let protocols = headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect::<Vec<_>>();
    // The upgrade is already authenticated. Native PEEK clients need no capability;
    // browsers may continue using their capability and ATTACH still requires one.
    let (selected, capability) = super::super::wire_names::terminal_protocol(
        &protocols, SUBPROTOCOL, query.mode == st3_client::RawTerminalMode::Peek,
    ).ok_or_else(|| validation("raw terminal requires one supported pty protocol plus one capability protocol (optional for peek)"))?;
    let direct_peek = capability.is_none();
    let live = remote_terminal_live_session(&state, &terminal_subject(&id), &query.incarnation)?;
    if !live.terminal {
        return Err(validation("raw attachment requires a terminal runtime"));
    }
    let acquisition_epoch = if direct_peek {
        Some(authorization_epoch(&state, &session)?)
    } else if query.mode == st3_client::RawTerminalMode::Peek {
        Some(peek_capability::consume(
            &state, &session, &client_detail_id("terminal", &id), &live, capability.unwrap(),
        )?)
    } else {
        consume_terminal_attachment_mode(
            &state,
            &session,
            &client_detail_id("terminal", &id),
            &query.incarnation,
            capability,
            Some(mode_name(query.mode)),
        )?;
        None
    };
    let origin = headers
        .get(ORIGIN_HEADER)
        .map(|header| {
            if session.transport != "unix" || query.mode != st3_client::RawTerminalMode::Peek {
                return Err(forbidden(
                    "raw origin bindings require a trusted Unix PEEK route",
                ));
            }
            serde_json::from_slice::<LeaseBinding>(header.as_bytes()).map_err(ApiError::internal)
        })
        .transpose()?;
    let lease = (query.mode == st3_client::RawTerminalMode::Peek)
        .then(|| {
            Lease::register(
                &state,
                &session,
                &client_detail_id("terminal", &id),
                &live.owner_host_id,
                &query.incarnation,
                origin,
                acquisition_epoch
                    .as_deref()
                    .ok_or_else(|| forbidden("raw PEEK capability has no authorization epoch"))?,
            )
        })
        .transpose()?;
    // Open and fence before HTTP upgrade: a stale owner incarnation is a refusal, not a blank pane.
    let (transport, control) = if live.owner_host_id == client_host_id(&state.node) {
        let terminal = crate::model::LocalTerminal {
            subject: terminal_subject(&id),
            runtime_id: live.runtime_id,
            incarnation_id: live.incarnation_id,
            pty_root: state.pty_root.clone(),
        };
        let stream = crate::client::open_local_terminal(&terminal)
            .await
            .map_err(|error| stale(error.to_string()))?;
        stream.set_nonblocking(true).map_err(ApiError::internal)?;
        (
            UnixStream::from_std(stream).map_err(ApiError::internal)?,
            None,
        )
    } else {
        let (transport, control) = state
            .client_relay
            .as_ref()
            .ok_or_else(|| remote_unavailable_for_owner(&state, &live.owner_host_id))?
            .raw_terminal(
                &live.owner_host_id,
                &session.authority_actor,
                &client_detail_id("terminal", &id),
                &query.incarnation,
                query.mode,
                lease.clone().map(|lease| lease as Arc<dyn crate::peer::RawTerminalLease>),
            )
            .await
            .map_err(|error| remote_read_error(&live.owner_host_id, error))?;
        (transport, Some(control))
    };
    if let Some(lease) = lease.clone() {
        lease_query(lease, |lease| lease.revalidate())
            .await
            .map_err(|error| stale(error.to_string()))?;
    }
    if query.mode == st3_client::RawTerminalMode::Peek {
        tracing::info!(
            person = %session.authority_actor,
            session = %session.actor,
            terminal = %id,
            owner = %live.owner_host_id,
            incarnation = %query.incarnation,
            "raw PEEK open authorized"
        );
    }
    Ok(websocket
        .protocols([selected.to_owned()])
        .max_message_size(CHUNK * 4)
        .max_frame_size(CHUNK * 4)
        .on_upgrade(move |socket| splice(socket, transport, Some(query.mode), lease, control, upgrade, first_frame)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pty_core::protocol::{encode_attach, encode_data, encode_peek, encode_resize};
    use st3_client::RawTerminalMode;

    #[tokio::test]
    async fn peek_authority_rejects_writer_frames_before_forwarding_headers() {
        for prohibited in [
            encode_attach(1, 1),
            encode_data(b"must-not-reach-child"),
            encode_resize(1, 1),
            pty_core::protocol::encode_packet(
                pty_core::protocol::MessageType::ResetInputModes,
                &[],
            ),
        ] {
            let (mut bridge, mut owner) = tokio::io::duplex(128);
            let mut gate = FrameGate::new(Some(RawTerminalMode::Peek));
            let peek = encode_peek(false, true);
            gate.write(&mut bridge, &peek).await.unwrap();
            let mut forwarded = vec![0; peek.len()];
            owner.read_exact(&mut forwarded).await.unwrap();
            assert_eq!(forwarded, peek);
            // Even a header split over several WebSocket messages cannot bypass the mode gate.
            gate.write(&mut bridge, &prohibited[..2]).await.unwrap();
            let error = gate.write(&mut bridge, &prohibited[2..]).await.unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            drop(bridge);
            let mut forbidden = Vec::new();
            owner.read_to_end(&mut forbidden).await.unwrap();
            assert_eq!(
                forbidden, b"",
                "no unauthorized header or payload may reach the PTY"
            );
        }
    }

    #[tokio::test]
    async fn writer_rejects_lifecycle_management_commands() {
        let (mut bridge, mut owner) = tokio::io::duplex(128);
        let mut gate = FrameGate::new(Some(RawTerminalMode::Attach));
        let attach = encode_attach(24, 80);
        gate.write(&mut bridge, &attach).await.unwrap();
        let mut forwarded = vec![0; attach.len()];
        owner.read_exact(&mut forwarded).await.unwrap();
        let cas =
            pty_core::protocol::encode_packet(pty_core::protocol::MessageType::LifecycleCas, b"{}");
        assert_eq!(
            gate.write(&mut bridge, &cas).await.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        drop(bridge);
        let mut forbidden = Vec::new();
        owner.read_to_end(&mut forbidden).await.unwrap();
        assert_eq!(forbidden, b"");
    }

    #[tokio::test]
    async fn attached_writer_can_recover_modes_but_cannot_add_a_payload() {
        let (mut bridge, mut owner) = tokio::io::duplex(128);
        let mut gate = FrameGate::new(Some(RawTerminalMode::Attach));
        let attach = encode_attach(24, 80);
        let reset = pty_core::protocol::encode_packet(
            pty_core::protocol::MessageType::ResetInputModes,
            &[],
        );
        gate.write(&mut bridge, &attach).await.unwrap();
        gate.write(&mut bridge, &reset[..2]).await.unwrap();
        gate.write(&mut bridge, &reset[2..]).await.unwrap();
        let expected = [attach, reset].concat();
        let mut bytes = vec![0; expected.len()];
        owner.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, expected);
        let malformed = pty_core::protocol::encode_packet(
            pty_core::protocol::MessageType::ResetInputModes,
            b"child input",
        );
        assert_eq!(
            gate.write(&mut bridge, &malformed)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        drop(bridge);
        let mut forbidden = Vec::new();
        owner.read_to_end(&mut forbidden).await.unwrap();
        assert!(forbidden.is_empty());
    }
}
