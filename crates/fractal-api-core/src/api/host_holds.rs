//! The execution host alone mutates its durable holds; transport identity owns each hold.
use super::*;
use crate::store::host_holds::{HostHold, PlaceHostHold, ReleaseHostHold};

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
