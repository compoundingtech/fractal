use super::*;

pub async fn fleet_publish_endpoints(
    State(state): State<AppState>,
    Json(request): Json<FleetEndpointsRequest>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let written = blocking_store(move || {
        store.publish_fleet_endpoints(&request.mode, &request.endpoints, env!("CARGO_PKG_VERSION"))
    })
    .await?;
    if written {
        signal_changed(&state);
    }
    Ok(Json(json!({ "published": written })))
}

/// Publish the configured trust root, not an exporter's proposed replacement key.
pub async fn fleet_anchor(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || {
        let anchor = store.fleet_anchor()?
            .ok_or_else(|| anyhow::anyhow!("this store has no fleet trust anchor"))?;
        Ok(json!({ "fleet_anchor": anchor }))
    }).await.map(Json)
}

pub async fn fleet_status(State(state): State<AppState>) -> Result<Json<FleetStatus>, ApiError> {
    let peers = replication_peer_names(&state);
    let store = state.store.clone();
    let node = state.node.clone();
    let fleet_id = state.fleet_id.clone();
    let state_dir = state.state_dir.clone();
    blocking_store(move || {
        let file = crate::config::FleetFile::load(&state_dir)?;
        let replication =
            store.replication_status_sealed(fleet_id.is_some(), fleet_id.as_deref(), &peers)?;
        Ok(FleetStatus {
            node,
            fleet_id,
            member_key: store.member_public_key(),
            transports: file
                .as_ref()
                .map(|file| file.transports.clone())
                .unwrap_or_default(),
            fabric_protocol: file
                .as_ref()
                .filter(|file| file.transports.iter().any(|name| name == "fabric"))
                .map(|file| {
                    file.fabric_protocol.clone().unwrap_or_else(|| {
                        crate::fleet::transport::default_fabric_protocol(&file.fleet_id)
                    })
                }),
            view: store.fleet_view_sealed()?,
            peers: replication.peers,
            invites: store.fleet_invites(false)?,
            removed: crate::fleet::file::RemovalNotice::load(&state_dir),
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
pub struct FleetInviteListQuery {
    #[serde(default)]
    all: bool,
}

pub async fn fleet_invite_list(
    State(state): State<AppState>,
    Query(query): Query<FleetInviteListQuery>,
) -> Result<Json<Vec<crate::store::FleetInviteView>>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || store.fleet_invites(query.all))
        .await
        .map(Json)
}

pub async fn fleet_invite_create(
    State(state): State<AppState>,
    Json(request): Json<FleetInviteRequest>,
) -> Result<Json<FleetInviteCreated>, ApiError> {
    concrete_person(&request.person)?;
    let fleet_id = state.fleet_id.clone().ok_or_else(|| {
        ApiError::bad(St3Error::new(
            "not-in-a-fleet",
            "this node is not in a fleet; run st fleet create first",
        ))
    })?;
    let store = state.store.clone();
    let node = state.node.clone();
    let created = blocking_action(move || {
        let via = request.via.clone().unwrap_or_else(|| "auto".into());
        let invitation = crate::fleet::join::invite(
            &store,
            &fleet_id,
            &node,
            &crate::fleet::join::InviteOptions {
                name: request.name.clone(),
                lifetime: std::time::Duration::from_secs(request.expires_seconds),
                via,
                person: request.person.clone(),
                migrate: request.migrate,
            },
        )?;
        Ok(FleetInviteCreated {
            invite: invitation.invite,
            code: invitation.code,
            expires_at_unix_ms: invitation.expires_at_unix_ms,
        })
    })
    .await?;
    signal_changed(&state);
    Ok(Json(created))
}

pub async fn fleet_invite_revoke(
    State(state): State<AppState>,
    Json(request): Json<FleetInviteRevokeRequest>,
) -> Result<Json<Value>, ApiError> {
    concrete_person(&request.person)?;
    let store = state.store.clone();
    let invite = request
        .invite
        .strip_prefix("fleet-invite/")
        .unwrap_or(&request.invite)
        .to_owned();
    blocking_store(move || {
        store.revoke_fleet_invite(&invite, &request.reason, Some(&request.person))
    })
    .await?;
    signal_changed(&state);
    Ok(Json(json!({ "revoked": true })))
}

pub async fn fleet_redeem(
    State(state): State<AppState>,
    Json(request): Json<crate::fleet::handshake::JoinRequest>,
) -> Result<Json<Value>, ApiError> {
    use crate::store::FleetRedemption;
    let store = state.store.clone();
    let state_dir = state.state_dir.clone();
    let fleet_id = state.fleet_id.clone();
    let (answer, changed) = blocking_store(move || {
        Ok(match store.redeem_fleet_invite(&request)? {
            FleetRedemption::Closed => (json!({ "status": "closed" }), false),
            FleetRedemption::Refused(reason) => {
                (json!({ "status": "refused", "reason": reason }), false)
            }
            FleetRedemption::Admitted {
                token,
                writer_floor,
                admitted_claim,
                first,
            } => {
                if let Some(node_id) = &request.node_id {
                    store.verify_fleet_pairing_admission(
                        &request.name,
                        &request.member_key,
                        node_id,
                    )?;
                }
                let membership = store.fleet_membership()?;
                let sponsor_endpoints = match membership.state(store.origin()) {
                    crate::fleet::MemberState::Current(member) => member.endpoints.clone(),
                    _ => Vec::new(),
                };
                let sponsor_node_id = sponsor_endpoints.iter().find_map(|endpoint| {
                    (endpoint["transport"] == "fabric")
                        .then(|| endpoint["node"].as_str())
                        .flatten()
                        .map(str::to_owned)
                });
                let fleet_id =
                    fleet_id.ok_or_else(|| anyhow::anyhow!("this node is not in a fleet"))?;
                let fabric_protocol = crate::config::FleetFile::load(&state_dir)?
                    .and_then(|file| file.fabric_protocol)
                    .unwrap_or_else(|| crate::fleet::transport::default_fabric_protocol(&fleet_id));
                (
                    json!({
                        "status": "admitted",
                        "token": hex::encode(token),
                        "writer_floor": writer_floor,
                        "admitted_claim": admitted_claim,
                        "anchor_key": store.fleet_anchor()?,
                        "fleet_id": fleet_id,
                        "fabric_protocol": fabric_protocol,
                        "sponsor_endpoints": sponsor_endpoints,
                        "sponsor_node_id": sponsor_node_id,
                    }),
                    first,
                )
            }
        })
    })
    .await?;
    if changed {
        signal_changed(&state);
    }
    Ok(Json(answer))
}

pub async fn fleet_remove(
    State(state): State<AppState>,
    Json(request): Json<FleetRemoveRequest>,
) -> Result<Json<crate::store::FleetRemoval>, ApiError> {
    concrete_person(&request.person)?;
    let store = state.store.clone();
    let removal = blocking_action(move || {
        store.remove_fleet_member(&request.name, &request.reason, &request.person)
    })
    .await?;
    signal_changed(&state);
    Ok(Json(removal))
}

pub async fn fleet_leave_begin(
    State(state): State<AppState>,
    Json(request): Json<FleetPersonRequest>,
) -> Result<Json<Value>, ApiError> {
    concrete_person(&request.person)?;
    let store = state.store.clone();
    blocking_store(move || store.set_fleet_leaving(true)).await?;
    Ok(Json(json!({ "leaving": true })))
}

/// The body is read and ignored, so the answer never races a client still sending it.
pub async fn fleet_leave_cancel(
    State(state): State<AppState>,
    _body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || store.set_fleet_leaving(false)).await?;
    Ok(Json(json!({ "leaving": false })))
}

pub async fn fleet_leave_claim(
    State(state): State<AppState>,
    Json(request): Json<FleetPersonRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    concrete_person(&request.person)?;
    let store = state.store.clone();
    let claim = blocking_action(move || store.leave_fleet(&request.person)).await?;
    signal_changed(&state);
    Ok(Json(claim))
}

pub async fn fleet_membership_view(
    State(state): State<AppState>,
) -> Result<Json<crate::fleet::FleetView>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || store.fleet_view_sealed())
        .await
        .map(Json)
}

/// Read the live replication authority; a journal cannot report failed persistence.
pub async fn fleet_leave_state(
    State(state): State<AppState>,
) -> Result<Json<smallclaims::sync::FleetLeaveSnapshot>, ApiError> {
    if state.fleet_id.is_none() {
        return Ok(Json(smallclaims::sync::FleetLeaveSnapshot {
            fleet_id: None,
            own_member_key: None,
            membership: crate::fleet::FleetView::default(),
            members: Vec::new(),
            persistence_error: None,
        }));
    }
    let http = reqwest::Client::builder()
        .unix_socket(smallclaims::sync::leave_state_socket(&state.state_dir))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(ApiError::internal)?;
    let snapshot = http
        .get("http://localhost/v1/internal/fleet/leave-state")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(ApiError::internal)?
        .json::<smallclaims::sync::FleetLeaveSnapshot>()
        .await
        .map_err(ApiError::internal)?;
    if snapshot.fleet_id != state.fleet_id
        || snapshot.own_member_key != state.store.member_public_key()
    {
        return Err(ApiError::internal(anyhow::anyhow!(
            "leave authority belongs to a different fleet incarnation"
        )));
    }
    Ok(Json(snapshot))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetPairingAdmission {
    request: smallclaims::fleet::handshake::JoinRequest,
    node_id: String,
}

/// Carry signed admission data for the external adapter; never program Fabric policy here.
pub async fn fleet_pairing_admission(
    State(state): State<AppState>,
    Json(request): Json<FleetPairingAdmission>,
) -> Result<Json<smallclaims::fleet::handshake::JoinResponse>, ApiError> {
    let fleet_id = state.fleet_id.as_deref().ok_or_else(|| {
        ApiError::bad(St3Error::new("not-in-a-fleet", "this node is not in a fleet"))
    })?;
    let sponsor_key = state.store.member_public_key().ok_or_else(|| {
        ApiError::internal(anyhow::anyhow!("this fleet has no member signing key"))
    })?;
    let http = reqwest::Client::builder()
        .unix_socket(smallclaims::sync::leave_state_socket(&state.state_dir))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(ApiError::internal)?;
    let response = http
        .post("http://localhost/v1/internal/fleet/pairing-admission")
        .json(&json!({
            "request": request.request,
            "node_id": request.node_id,
            "expected_fleet_id": fleet_id,
            "expected_sponsor_key": sponsor_key,
        }))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(ApiError::internal)?
        .json::<smallclaims::fleet::handshake::JoinResponse>()
        .await
        .map_err(ApiError::internal)?;
    if response.sponsor_key != sponsor_key {
        return Err(ApiError::internal(anyhow::anyhow!(
            "pairing admission belongs to a different fleet incarnation"
        )));
    }
    signal_changed(&state);
    Ok(Json(response))
}

/// What doctor's fleet-admission check reports as admitted before this node knew better, for
/// `st fleet wait` to refuse on.
pub async fn fleet_admission_residue(
    State(state): State<AppState>,
) -> Result<Json<Vec<smallclaims::store::FleetAdmissionResidue>>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || store.fleet_admission_residue())
        .await
        .map(Json)
}
