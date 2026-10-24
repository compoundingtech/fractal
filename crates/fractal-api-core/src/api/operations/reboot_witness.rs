use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRebootWitnessRequest {
    actor: String,
    subject: String,
}

/// No import endpoint exists: only the owning daemon may author the manifest.
/// An unbound caller cannot supply an agent identity in JSON to manufacture one.
pub async fn capture_reboot_witness(
    State(state): State<AppState>,
    bound: Option<axum::Extension<BoundAgent>>,
    Json(request): Json<CaptureRebootWitnessRequest>,
) -> Result<Json<Value>, ApiError> {
    let Some(axum::Extension(bound)) = bound else {
        return Err(ApiError::bad(St3Error::new("unbound-repair-actor", "reboot witness capture requires a transport-bound agent")));
    };
    let identity = public_identity(&state.store, crate::store::identities::IdentityKind::Agent, &bound.0)?.identity;
    if !is_bound_actor(&state.store, &request.actor, &identity)? {
        return Err(ApiError::bad(St3Error::new("foreign-agent-actor", "reboot witness actor must match the bound harness")));
    }
    let result = blocking_action(move || {
        let capture = || -> anyhow::Result<Value> {
            let operation = state.store.pending_runtime_starts()?.into_iter()
                .find(|operation| operation.subject == request.subject)
                .ok_or_else(|| anyhow::anyhow!("no pending revoked operation"))?;
            anyhow::ensure!(operation.revoked_by.is_some()
                && operation.body["member"]["terminal"] == true
                && operation.body["member"]["host"].as_str() == Some(state.node.as_str()),
                "reboot witness requires a revoked owner-local terminal operation");
            let runtime_id = operation.body["member"]["runtime_id"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing runtime binding"))?;
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("owner home unavailable"))?;
            let runtime = daemon_pty(&state)?;
            let proof = runtime.capture_reboot_witness(runtime_id, &operation.id, &state.node, &home, &state.state_dir)?;
            // The runtime fence remains held through this writer transaction;
            // the writer independently checks the exact row is still revoked.
            let retained = state.store.retain_reboot_witness(&operation.id, &proof.evidence, &identity)?;
            Ok(serde_json::to_value(retained)?)
        };
        capture().map_err(legacy_start_proof_error)
    }).await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reboot_witness_capture_refuses_unbound_and_foreign_actors_before_scanning() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests::state(root.path());
        let request = || CaptureRebootWitnessRequest { actor: "agent/foreign".into(), subject: "agent/seat".into() };
        let error = capture_reboot_witness(State(state.clone()), None, Json(request())).await.unwrap_err();
        assert_eq!(error.code, "unbound-repair-actor");
        let error = capture_reboot_witness(State(state),
            Some(axum::Extension(BoundAgent("agent/owner".into()))), Json(request())).await.unwrap_err();
        assert_eq!(error.code, "foreign-agent-actor");
    }
}
