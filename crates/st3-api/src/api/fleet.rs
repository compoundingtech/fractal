use super::*;

pub(super) fn fabric_reconcile_status(
    state_dir: &std::path::Path,
    file: Option<&crate::config::FleetFile>,
) -> smallclaims::fleet::fabric_reconcile::ReconcileStatus {
    use smallclaims::fleet::fabric_reconcile::ReconcileStatus;
    let path = state_dir.join("fleet/fabric-reconcile.json");
    let result = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<ReconcileStatus>(&bytes)
            .map_err(|error| format!("read {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ReconcileStatus {
                state: if file.is_some_and(|file| {
                    file.transports.iter().any(|transport| transport == "fabric")
                }) {
                    "pending"
                } else {
                    "disabled"
                }
                .into(),
                ..ReconcileStatus::default()
            };
        }
        Err(error) => Err(format!("read {}: {error}", path.display())),
    };
    match result {
        Ok(status) if !status.state.is_empty() => status,
        Ok(_) => ReconcileStatus {
            state: "error".into(),
            message: Some(format!("read {}: missing reconciliation state", path.display())),
            policy_revision: None,
        },
        Err(message) => ReconcileStatus {
            state: "error".into(),
            message: Some(message),
            policy_revision: None,
        },
    }
}

#[cfg(test)]
mod fabric_reconcile_status_tests {
    use super::fabric_reconcile_status;

    #[test]
    fn missing_result_is_pending_only_with_fabric_enabled() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(fabric_reconcile_status(root.path(), None).state, "disabled");
        let mut file = crate::config::FleetFile::default();
        file.transports = vec!["tailscale".into()];
        assert_eq!(
            fabric_reconcile_status(root.path(), Some(&file)).state,
            "disabled"
        );
        file.transports.push("fabric".into());
        assert_eq!(
            fabric_reconcile_status(root.path(), Some(&file)).state,
            "pending"
        );
    }

    #[test]
    fn recorded_results_are_preserved_and_bad_results_are_errors() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("fleet")).unwrap();
        let path = root.path().join("fleet/fabric-reconcile.json");
        for state in ["applied", "error", "pending", "runtime_peers_disabled"] {
            std::fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!({
                    "state": state,
                    "message": "recorded diagnostic",
                    "policy_revision": 42
                }))
                .unwrap(),
            )
            .unwrap();
            let status = fabric_reconcile_status(root.path(), None);
            assert_eq!(status.state, state);
            assert_eq!(status.message.as_deref(), Some("recorded diagnostic"));
            assert_eq!(status.policy_revision, Some(42));
        }
        for bytes in [b"not json".as_slice(), br#"{"state":""}"#.as_slice()] {
            std::fs::write(&path, bytes).unwrap();
            let status = fabric_reconcile_status(root.path(), None);
            assert_eq!(status.state, "error");
            assert!(status.message.is_some());
            assert_eq!(status.policy_revision, None);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(fabric_reconcile_status(root.path(), None).state, "error");
    }
}

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
            fabric_reconcile: fabric_reconcile_status(&state_dir, file.as_ref()),
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
