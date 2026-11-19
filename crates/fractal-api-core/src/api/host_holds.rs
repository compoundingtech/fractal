//! The execution host alone mutates its durable holds; transport identity owns each hold.
use super::*;
use crate::store::host_holds::{HostHold, PlaceHostHold, RecoverHostHold, ReleaseHostHold};
use crate::RuntimeStartsStore as _;

fn owner(state: &AppState, bound: Option<&BoundActor>) -> Result<String, ApiError> {
    bound.map(|actor| actor.actor().to_owned()).or_else(|| state.owning_person.clone())
        .ok_or_else(|| ApiError::bad(FractalError::new("host-hold-owner", "host holds require an authenticated agent/run or configured local owning person")))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<HostHold>>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || Ok(store.host_holds(None, false)?)).await.map(Json)
}

pub async fn place(State(state): State<AppState>, bound: Option<BoundActor>, Json(request): Json<PlaceHostHold>) -> Result<Json<HostHold>, ApiError> {
    let actor = owner(&state, bound.as_ref())?;
    let store = state.store.clone();
    let hold = blocking_store(move || Ok(store.place_host_hold(&actor, &request)?)).await?;
    signal_changed(&state);
    Ok(Json(hold))
}

pub async fn release(State(state): State<AppState>, bound: Option<BoundActor>, Json(request): Json<ReleaseHostHold>) -> Result<Json<HostHold>, ApiError> {
    let actor = owner(&state, bound.as_ref())?;
    let store = state.store.clone();
    let hold = blocking_store(move || Ok(store.release_host_hold(&actor, &request)?)).await?;
    signal_changed(&state);
    Ok(Json(hold))
}

pub async fn recover(State(state): State<AppState>, bound: Option<BoundActor>, Json(request): Json<RecoverHostHold>) -> Result<Json<HostHold>, ApiError> {
    if bound.is_some() {
        return Err(ApiError::bad(FractalError::new("host-hold-recovery-authority", "bound agents and runs cannot act as the configured host operator")));
    }
    let operator = state.owning_person.clone().ok_or_else(|| ApiError::bad(
        FractalError::new("host-hold-recovery-authority", "hold recovery requires a configured host operator")))?;
    let store = state.store.clone();
    let hold = blocking_store(move || Ok(store.recover_host_hold(&operator, &operator, &request)?)).await?;
    signal_changed(&state);
    Ok(Json(hold))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeRequest {
    pub actor: String,
    pub id: String,
    pub reason: String,
    pub duration_ms: u64,
    #[serde(default = "default_freeze_wait")]
    pub wait_ms: u64,
}

fn default_freeze_wait() -> u64 {
    crate::store::launch_freeze::DEFAULT_WAIT_MS
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearFreezeRequest {
    pub actor: String,
    pub id: String,
}

pub async fn launch_freeze(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || {
        let freeze = store.launch_freeze()?;
        let deferred = store.pending_runtime_starts()?.into_iter()
            .filter(|operation| !operation.dispatched && !operation.finished && operation.revoked_by.is_none())
            .map(|operation| json!({"id": operation.id, "subject": operation.subject}))
            .collect::<Vec<_>>();
        Ok(json!({"freeze": freeze, "deferred": deferred}))
    }).await.map(Json)
}

pub async fn set_launch_freeze(State(state): State<AppState>, bound: Option<BoundActor>, Json(request): Json<FreezeRequest>) -> Result<Json<Value>, ApiError> {
    let actor = owner(&state, bound.as_ref())?;
    if actor != request.actor {
        return Err(ApiError::bad(FractalError::new("launch-freeze-owner", "payload actor does not match authenticated owner")));
    }
    let store = state.store.clone();
    let result = blocking_action(move || store.set_launch_freeze(&actor, &request.id, &request.reason, request.duration_ms, request.wait_ms)).await;
    signal_changed(&state);
    Ok(Json(json!(result?)))
}

pub async fn clear_launch_freeze(State(state): State<AppState>, bound: Option<BoundActor>, Json(request): Json<ClearFreezeRequest>) -> Result<Json<Value>, ApiError> {
    let actor = owner(&state, bound.as_ref())?;
    if actor != request.actor {
        return Err(ApiError::bad(FractalError::new("launch-freeze-owner", "payload actor does not match authenticated owner")));
    }
    let store = state.store.clone();
    blocking_store(move || Ok(store.clear_launch_freeze(&actor, &request.id)?)).await?;
    signal_changed(&state);
    Ok(Json(json!({"cleared": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_reconcile::PersonWorkStore as _;
    #[tokio::test]
    async fn answered_asks_do_not_release_host_holds() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"asker\" { workspace \"/workspace\"; command \"true\" }", &state.node).unwrap();
        state.store.apply_internal(&intent, "hold-asker").unwrap();
        let ask = state.store.ask_person(&PersonAskRequest {
            deadline_ms: None, default_answer: None, legacy_request: None, priority: None,
            person: "person/operator".into(), title: "Repair approved?".into(), reason: "Operator decides".into(),
            actor: format!("agent/{}.asker", state.node), step: None, new_run: Some("hold-ask".into()),
            incarnation: None, idempotency_key: "hold-ask".into(), request: None,
        }).unwrap();
        let Json(hold) = place(State(state.clone()), None, Json(PlaceHostHold {
            actor: "person/operator".into(), host: state.node.clone(), reason: "repair pending".into(),
            operations: vec!["activation".into()], originating_run: Some(ask.run.clone()), receipt: None,
        })).await.unwrap();
        state.store.finish_person_step(&PersonStepResponse {
            delegation: None, subject: ask.subject, actor: "person/operator".into(),
            summary: "Approved".into(), evidence: vec![], episode: None,
            idempotency_key: "hold-ask-answer".into(), answer: None,
        }, false).unwrap();
        assert_eq!(state.store.host_holds(None, false).unwrap(), vec![hold]);
    }
    #[tokio::test]
    async fn operator_recovery_rejects_payload_impersonation_and_stale_revision() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let hold = state.store.place_host_hold("mission-run/finished", &PlaceHostHold {
            actor: "mission-run/finished".into(), host: state.node.clone(), reason: "investigate".into(),
            operations: vec!["*".into()], originating_run: None, receipt: None,
        }).unwrap();
        let mut request = RecoverHostHold { actor: "person/other".into(), host: hold.host.clone(),
            id: hold.id.clone(), expected_revision: hold.revision, reason: "repair verified".into() };
        assert!(recover(State(state.clone()), None, Json(request.clone())).await.is_err());
        request.actor = "person/operator".into();
        for bound in [BoundActor::Agent("agent/worker".into()), BoundActor::Run("mission-run/finished".into())] {
            assert!(recover(State(state.clone()), Some(bound), Json(request.clone())).await.is_err());
        }
        let mut unconfigured = state.clone(); unconfigured.owning_person = None;
        assert!(recover(State(unconfigured), None, Json(request.clone())).await.is_err());
        request.expected_revision += 1;
        assert!(recover(State(state.clone()), None, Json(request.clone())).await.is_err());
        request.expected_revision = hold.revision;
        assert!(recover(State(state.clone()), None, Json(request.clone())).await.is_err(),
            "a missing owner run does not prove terminal custody");
        assert_eq!(state.store.host_holds(None, false).unwrap(), vec![hold.clone()]);
        state.store.connection.write().execute(
            "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms) VALUES ('finished','example','rev','gen','rev','finished','/workspace','person/operator','{}','normal','succeeded','terminal','10','10')", []).unwrap();
        let Json(recovered) = recover(State(state.clone()), None, Json(request.clone())).await.unwrap();
        assert_eq!(recovered.owner, hold.owner);
        assert_eq!(recovered.revision, hold.revision + 1);
        assert!(recover(State(state.clone()), None, Json(request)).await.is_err());
        assert!(state.store.host_holds(None, false).unwrap().is_empty());
    }
    #[tokio::test]
    async fn transport_identity_not_payload_authenticates_owner() {
        let root = tempfile::tempdir().unwrap(); let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let request = PlaceHostHold { actor: "person/other".into(), host: state.node.clone(), reason: "investigate".into(), operations: vec!["*".into()], originating_run: None, receipt: None };
        assert!(place(State(state), None, Json(request)).await.is_err());
    }
    #[tokio::test]
    async fn committed_resolution_wakes_admission_and_removes_attention() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let Json(hold) = place(State(state.clone()), None, Json(PlaceHostHold {
            actor: "person/operator".into(), host: state.node.clone(), reason: "investigate".into(),
            operations: vec!["activation".into()], originating_run: Some("mission-run/terminal".into()),
            receipt: Some("receipt/activation".into()),
        })).await.unwrap();
        let rows = client_attention_resources(&state.store, Some("person/operator"), false).unwrap();
        let row = rows.iter().find(|row| row["attention_kind"] == "host-hold").unwrap();
        assert_eq!(row["source_id"], hold.id);
        assert!(row["mission_run_id"].is_null(), "terminal originating runs must not hide holds");
        assert_eq!(row["targets"][1], "mission-run/terminal");
        // Consume placement's coalesced notification before registering for resolution.
        state.notify.notified().await;
        let wake = state.notify.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let Json(released) = release(State(state.clone()), None, Json(ReleaseHostHold {
            actor: hold.owner, host: hold.host, id: hold.id, expected_revision: hold.revision,
            resolution: "repaired".into(),
        })).await.unwrap();
        assert_eq!(released.revision, hold.revision + 1);
        assert_eq!(released.resolution.as_deref(), Some("repaired"));
        assert!(released.released_at_unix_ms.is_some());
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(wake.as_mut(), context).is_ready())).await);
        assert!(client_attention_resources(&state.store, Some("person/operator"), false).unwrap()
            .iter().all(|row| row["attention_kind"] != "host-hold"));
    }

    #[tokio::test]
    async fn launch_freeze_api_authenticates_owner_reports_state_and_clears() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let request = || FreezeRequest {
            actor: "person/operator".into(), id: "activation".into(),
            reason: "replace daemon".into(), duration_ms: 60_000, wait_ms: 0,
        };
        let refused = set_launch_freeze(State(state.clone()),
            Some(BoundActor::Agent("agent/worker".into())), Json(request())).await.unwrap_err();
        assert_eq!(refused.code, "launch-freeze-owner");
        let Json(placed) = set_launch_freeze(State(state.clone()), None, Json(request())).await.unwrap();
        assert_eq!(placed["id"], "activation");
        let Json(shown) = launch_freeze(State(state.clone())).await.unwrap();
        assert_eq!(shown["freeze"]["owner"], "person/operator");
        assert_eq!(shown["freeze"]["reason"], "replace daemon");
        assert!(shown["freeze"]["expires_at_unix_ms"].as_u64().is_some());
        assert_eq!(shown["deferred"], json!([]));
        let Json(cleared) = clear_launch_freeze(State(state.clone()), None, Json(ClearFreezeRequest {
            actor: "person/operator".into(), id: "activation".into(),
        })).await.unwrap();
        assert_eq!(cleared["cleared"], true);
        assert!(state.store.admit_launch().unwrap().is_some());
    }

    #[tokio::test]
    async fn launch_freeze_api_timeout_is_typed_unarmed_and_wakes_the_queue() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(root.path());
        state.owning_person = Some("person/operator".into());
        let _permit = state.store.admit_launch().unwrap().unwrap();
        let refused = set_launch_freeze(State(state.clone()), None, Json(FreezeRequest {
            actor: "person/operator".into(), id: "activation".into(), reason: "replace daemon".into(),
            duration_ms: 60_000, wait_ms: 0,
        })).await.unwrap_err();
        assert_eq!(refused.code, "launch-freeze-timeout");
        assert!(state.store.launch_freeze().unwrap().is_none());
        let wake = state.notify.notified();
        tokio::pin!(wake);
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(wake.as_mut(), context).is_ready())).await);
    }
}
