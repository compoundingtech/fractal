//! Trusted-local shell transport. Admission remains owned by the host reconciler.
use super::*;
use crate::store::shell_resources::{Command, Execution};
use crate::store::resource_membership::{Coverage, RequestClass};

#[derive(Default, Deserialize)]
pub(super) struct ListQuery {
    host: Option<String>,
    class: Option<RequestClass>,
    head: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct WaitQuery { after: u64 }

pub(super) async fn submit(State(state): State<AppState>, Json(mut command): Json<Command>) -> Result<Json<Execution>, ApiError> {
    (|| -> anyhow::Result<()> {
        anyhow::ensure!(command.lane.host.is_empty() || command.lane.host == state.node, "shell execution must use the connected host");
        let id = format!("shell:{}", command.id);
        anyhow::ensure!(command.lane.id == id && command.lane.owner.run == id && command.lane.owner.step == id
            && command.lane.owner.attempt == 1 && command.lane.owner.invocation.as_deref() == Some(id.as_str())
            && command.lane.max_members == 1, "invalid shell execution owner");
        anyhow::ensure!(uuid::Uuid::parse_str(&command.id).is_ok(), "shell execution identity must be a UUID");
        Ok(())
    })().map_err(|error| ApiError::bad(FractalError::new("resource-exec-refused", error.to_string())))?;
    command.lane.host.clone_from(&state.node);
    if let Some(parent) = &command.parent {
        let record = state.store.resource_membership_for_grant(parent).map_err(ApiError::internal)?;
        validate_parent(&command, &record.request, &record.state).map_err(ApiError::internal)?;
        // The same durable membership owns every nested root. Never submit a second charge.
        command.lane = record.request;
    }
    let execution = state.store.submit_shell_resource(command, u64::try_from(client_now_ms()).unwrap_or(u64::MAX)).map_err(ApiError::internal)?;
    state.notify.notify_one();
    state.event_notify.send_modify(|generation| *generation = generation.saturating_add(1));
    Ok(Json(present(execution)))
}
fn validate_parent(command: &Command, parent: &crate::store::resource_membership::LaneRequest, state: &str) -> anyhow::Result<()> {
    anyhow::ensure!(matches!(state, "admitted" | "launched"), "nested parent membership is not active");
    anyhow::ensure!(parent.host == command.lane.host && parent.name == command.lane.name && parent.class == command.lane.class,
        "nested execution must inherit the parent host, lane and class");
    anyhow::ensure!(command.lane.coverage != Coverage::Enforced || parent.coverage == Coverage::Enforced,
        "nested execution cannot upgrade logical parent coverage");
    if let Some(capacity) = command.lane.capacity {
        let inherited = parent.capacity.context("nested execution cannot add parent capacity")?;
        anyhow::ensure!(capacity.cores <= inherited.cores && capacity.memory <= inherited.memory,
            "nested execution exceeds the parent reservation");
    }
    if let Some(disk) = &command.lane.disk {
        let inherited = parent.disk.as_ref().context("nested execution cannot add a disk reservation")?;
        anyhow::ensure!(disk.path == inherited.path && disk.bytes <= inherited.bytes, "nested execution exceeds the parent disk reservation");
    }
    anyhow::ensure!(command.lane.io.is_none() || command.lane.io == parent.io, "nested execution cannot add heavy I/O custody");
    Ok(())
}

fn present(mut execution: Execution) -> Execution {
    // The durable launch envelope can contain credentials. Inspection never exports it.
    execution.command.environment.clear();
    execution
}

pub(super) async fn list(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let host = query.host.as_deref().unwrap_or(&state.node);
    let rows: Vec<_> = state.store.shell_resources(host, false).map_err(ApiError::internal)?.into_iter()
        .filter(|row| query.class.is_none_or(|class| row.command.lane.class == class))
        .filter(|row| query.head.as_ref().is_none_or(|head| row.head_start.as_ref() == Some(head)))
        .map(present).collect();
    let memberships: Vec<_> = state.store.resource_memberships_for_host(host).map_err(ApiError::internal)?.into_iter()
        .filter(|row| query.class.is_none_or(|class| row.request.class == class)).collect();
    let holds = state.store.host_holds(Some(host), false).map_err(ApiError::internal)?;
    let accountant = state.seat_admission.snapshot(admission_now());
    Ok(Json(json!({"executions":rows,"memberships":memberships,"holds":holds,"accountant":accountant})))
}

pub(super) async fn show(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let execution = state.store.shell_resource(&id).map_err(ApiError::internal)?;
    let membership = state.store.resource_memberships_for_host(&execution.command.lane.host).map_err(ApiError::internal)?.into_iter()
        .find(|member| member.request.id == execution.command.lane.id);
    let holds = state.store.host_holds(Some(&execution.command.lane.host), false).map_err(ApiError::internal)?;
    Ok(Json(json!({"execution":present(execution),"membership":membership,"holds":holds,"accountant":state.seat_admission.snapshot(admission_now())})))
}

pub(super) async fn wait(State(state): State<AppState>, AxumPath(id): AxumPath<String>, Query(query): Query<WaitQuery>) -> Result<Json<Execution>, ApiError> {
    let mut changed = state.event_notify.subscribe();
    loop {
        let execution = state.store.shell_resource(&id).map_err(ApiError::internal)?;
        if execution.revision > query.after || matches!(execution.state.as_str(), "completed" | "cancelled" | "refused") { return Ok(Json(present(execution))); }
        changed.changed().await.map_err(ApiError::internal)?;
    }
}

pub(super) async fn cancel(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    state.store.cancel_shell_resource(&id).map_err(ApiError::internal)?;
    state.notify.notify_one();
    Ok(Json(json!({"cancelled":id})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource_membership::{LaneRequest, Owner};
    fn request() -> LaneRequest {
        LaneRequest { id: "shell:parent".into(), name: "checks".into(), host: "host-a".into(), max_members: 1,
            class: RequestClass::Check, capacity: Some(fractal_runtime::Capacity { cores: 4, memory: 24 << 30 }),
            disk: None, io: None, coverage: Coverage::Logical, admission_timeout_ms: None,
            owner: Owner { run:"shell:parent".into(), step:"shell:parent".into(), attempt:1, invocation:None } }
    }
    #[test]
    fn nested_execution_inherits_only_live_matching_custody() {
        let parent = request();
        let mut command = Command { id:"child".into(), lane:parent.clone(), argv:vec!["true".into()], cwd:"/work".into(), environment:BTreeMap::new(), timeout_ms:None, parent:None };
        assert!(validate_parent(&command,&parent,"launched").is_ok());
        assert!(validate_parent(&command,&parent,"retired").is_err());
        command.lane.class = RequestClass::Landing;
        assert!(validate_parent(&command,&parent,"launched").is_err());
        command.lane.class = RequestClass::Check;
        command.lane.capacity.as_mut().unwrap().memory += 1;
        assert!(validate_parent(&command,&parent,"launched").is_err());
    }

    #[tokio::test]
    async fn nested_submission_keeps_one_membership_and_rejects_a_stale_marker() {
        use crate::store::resource_membership::{Admission, Fit};
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        state.store.start_resource_authority(&state.node, "epoch").unwrap();
        let mut parent = request(); parent.host.clone_from(&state.node);
        let Admission::Admitted(grant) = state.store.request_resource_membership(&parent,
            Fit { room: Some(fractal_runtime::Capacity { cores:32, memory:128 << 30 }), ..Fit::default() }, 10).unwrap() else { panic!("parent was not admitted") };
        let id = uuid::Uuid::now_v7().to_string();
        let owner = format!("shell:{id}");
        let mut lane = parent.clone(); lane.id.clone_from(&owner);
        lane.owner = Owner { run:owner.clone(), step:owner.clone(), attempt:1, invocation:Some(owner) };
        let command = Command { id, lane, argv:vec!["true".into()], cwd:"/work".into(), environment:BTreeMap::new(),
            timeout_ms:None, parent:Some(grant.clone()) };
        let Json(child) = submit(State(state.clone()), Json(command.clone())).await.unwrap();
        assert_eq!(child.command.lane, parent);
        assert_eq!(state.store.resource_memberships_for_host(&state.node).unwrap().len(), 1);
        assert_eq!(state.store.held_resource_membership_bytes(&state.node, "").unwrap(), 24 << 30);
        state.store.cancel_resource_membership(&grant, &parent.owner, 11).unwrap();
        assert!(submit(State(state), Json(command)).await.is_err());
    }

    #[tokio::test]
    async fn native_hold_release_wakes_a_queued_shell_without_polling() {
        use crate::store::resource_membership::{Admission, BlockedBy, Fit};
        use crate::store::host_holds::{PlaceHostHold, ReleaseHostHold};
        let directory = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(directory.path());
        state.owning_person = Some("person/operator".into());
        state.store.start_resource_authority(&state.node, "epoch").unwrap();
        let Json(hold) = host_holds::place(State(state.clone()), None, Json(PlaceHostHold {
            actor:"person/operator".into(), host:state.node.clone(), reason:"inspect".into(), operations:vec!["checks".into()],
            originating_run:None, receipt:None,
        })).await.unwrap();
        let mut lane = request(); lane.host.clone_from(&state.node);
        let fit = || Fit { room:Some(fractal_runtime::Capacity { cores:32, memory:128 << 30 }), ..Fit::default() };
        assert!(matches!(state.store.request_resource_membership(&lane,fit(),10).unwrap(), Admission::Queued { .. }));
        assert_eq!(state.store.resource_membership_record(&lane.id,&lane.owner).unwrap().blocked_by,Some(BlockedBy::Hold));
        state.notify.notified().await;
        let wake = state.notify.notified(); tokio::pin!(wake); wake.as_mut().enable();
        host_holds::release(State(state.clone()), None, Json(ReleaseHostHold {
            actor:hold.owner, host:hold.host, id:hold.id, expected_revision:hold.revision, resolution:"ready".into(),
        })).await.unwrap();
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(Future::poll(wake.as_mut(),context).is_ready())).await);
        assert!(matches!(state.store.request_resource_membership(&lane,fit(),11).unwrap(), Admission::Admitted(_)));
    }

    #[tokio::test]
    async fn shell_wait_subscribes_before_reading_and_exports_no_launch_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        let execution = state.store.submit_shell_resource(Command {
            id:"waiter".into(), lane:request(), argv:vec!["true".into()], cwd:"/work".into(),
            environment:BTreeMap::from([("EXAMPLE_TOKEN".into(),"private-test-value".into())]), timeout_ms:None, parent:None,
        }, 10).unwrap();
        let waiting = wait(State(state.clone()),AxumPath("waiter".into()),Query(WaitQuery { after:0 }));
        tokio::pin!(waiting);
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(Future::poll(waiting.as_mut(),context).is_pending())).await);
        let mut completed = execution;
        completed.state = "completed".into(); completed.exit_code = Some(37);
        state.store.update_shell_resource(&completed).unwrap();
        state.event_notify.send_modify(|generation| *generation += 1);
        let Json(result) = waiting.await.unwrap();
        assert_eq!(result.exit_code,Some(37));
        assert!(result.command.environment.is_empty());
        assert_eq!(state.store.shell_resource("waiter").unwrap().command.environment.len(),1);
    }
}
