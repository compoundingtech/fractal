//! Shell transport: bound callers access only their native owner; admission stays with the host reconciler.
use super::*;
use anyhow::Context;
use crate::store::shell_resources::{Command, Execution};
use crate::store::resource_membership::{Coverage, RequestClass};

pub(super) fn allows_bound_request(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;
    match path.strip_prefix("/v1/internal/resources") {
        Some("") => *method == Method::GET,
        Some("/exec") => *method == Method::POST,
        Some(rest) => {
            let Some(rest) = rest.strip_prefix('/') else { return false; };
            let mut segments = rest.split('/');
            match (segments.next(), segments.next(), segments.next()) {
                (Some(id), None, None) if !id.is_empty() => *method == Method::GET,
                (Some(id), Some("wait" | "cancel"), None) if !id.is_empty() => *method == Method::POST,
                _ => false,
            }
        },
        None => false,
    }
}

fn require_exec_platform(macos: bool) -> Result<(), ApiError> {
    if macos {
        return Err(ApiError::bad(FractalError::new("unsupported-platform",
            "resources exec is unavailable on macOS: no whole-tree retirement proof")));
    }
    Ok(())
}

/// A run credential owns its exact attempt; an agent additionally owns its native invocation.
/// Missing/stale native evidence never falls back to local-operator authority.
struct CallerOwner {
    owner: crate::store::resource_membership::Owner,
    agent: bool,
}

impl CallerOwner {
    fn resolve(state: &AppState, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>) -> Result<Option<Self>, ApiError> {
        let owner_error = |error: anyhow::Error| ApiError::bad(FractalError::new("resource-exec-owner", error.to_string()));
        if let Some(Extension(BoundRun(binding))) = run {
            return Ok(Some(Self { owner: crate::store::resource_membership::Owner {
                run: binding.run, step: binding.step_run, attempt: binding.attempt, invocation: None,
            }, agent: false }));
        }
        match bound {
            Some(BoundActor::Agent(agent)) => Ok(Some(Self {
                owner: state.store.shell_resource_agent_owner(&agent).map_err(owner_error)?, agent: true,
            })),
            Some(BoundActor::Run(_)) => Err(ApiError::bad(FractalError::new(
                "resource-exec-owner", "run caller has no verified attempt binding"))),
            None => Ok(None),
        }
    }

    fn owns(&self, owner: &crate::store::resource_membership::Owner) -> bool {
        self.owner.run == owner.run && self.owner.step == owner.step && self.owner.attempt == owner.attempt
            && (!self.agent || self.owner.invocation == owner.invocation)
    }
}

fn authorize_execution(caller: Option<&CallerOwner>, execution: &Execution) -> Result<(), ApiError> {
    if caller.is_some_and(|caller| !caller.owns(&execution.command.lane.owner)) {
        return Err(ApiError::bad(FractalError::new("resource-exec-owner",
            "shell execution belongs to another execution owner")));
    }
    Ok(())
}

#[derive(Default, Deserialize)]
pub(super) struct ListQuery {
    host: Option<String>,
    class: Option<RequestClass>,
    head: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct WaitQuery { after: u64 }

pub(super) async fn submit(State(state): State<AppState>, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>, Json(mut command): Json<Command>) -> Result<Json<Execution>, ApiError> {
    require_exec_platform(cfg!(target_os = "macos"))?;
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
    let local_operator = run.is_none() && !matches!(&bound, Some(BoundActor::Agent(_) | BoundActor::Run(_)));
    let agent_caller = run.is_none() && matches!(&bound, Some(BoundActor::Agent(_)));
    let owner = if let Some(Extension(BoundRun(binding))) = run {
        crate::store::resource_membership::Owner { run: binding.run, step: binding.step_run, attempt: binding.attempt,
            invocation: Some(format!("shell:{}", command.id)) }
    } else if let Some(BoundActor::Agent(agent)) = bound {
        state.store.shell_resource_agent_owner(&agent).map_err(|error|
            ApiError::bad(FractalError::new("resource-exec-owner", error.to_string())))?
    } else if matches!(bound, Some(BoundActor::Run(_))) {
        return Err(ApiError::bad(FractalError::new("resource-exec-owner", "run caller has no verified attempt binding")));
    } else {
        crate::store::resource_membership::Owner {
            run: state.owning_person.clone().unwrap_or_else(|| format!("local:{}", state.node)),
            step: format!("shell:{}", command.id), attempt: 1, invocation: Some(format!("shell:{}", command.id)),
        }
    };
    state.store.shell_resource_owner_live(&owner).map_err(|error|
        ApiError::bad(FractalError::new("resource-exec-owner", error.to_string())))?;
    if let Some(parent) = &command.parent {
        let record = state.store.resource_membership_for_grant(parent).map_err(ApiError::internal)?;
        if record.request.owner.run != owner.run || (!local_operator
            && (record.request.owner.step != owner.step || record.request.owner.attempt != owner.attempt))
            || (agent_caller && record.request.owner.invocation != owner.invocation) {
            return Err(ApiError::bad(FractalError::new("resource-exec-owner", "nested membership belongs to another execution owner")));
        }
        validate_parent(&command, &record.request, &record.state).map_err(ApiError::internal)?;
        // The same durable membership owns every nested root. Never submit a second charge.
        command.lane = record.request;
    } else {
        command.lane.owner = owner;
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
    execution.command.parent = None;
    execution
}

pub(super) async fn list(State(state): State<AppState>, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>, Query(query): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let caller = CallerOwner::resolve(&state, bound, run)?;
    let host = query.host.as_deref().unwrap_or(&state.node);
    let rows: Vec<_> = state.store.shell_resources(host, false).map_err(ApiError::internal)?.into_iter()
        .filter(|row| caller.as_ref().is_none_or(|caller| caller.owns(&row.command.lane.owner)))
        .filter(|row| query.class.is_none_or(|class| row.command.lane.class == class))
        .filter(|row| query.head.as_ref().is_none_or(|head| row.head_start.as_ref() == Some(head)))
        .map(present).collect();
    let memberships: Vec<_> = state.store.resource_memberships_for_host(host).map_err(ApiError::internal)?.into_iter()
        .filter(|row| caller.as_ref().is_none_or(|caller| caller.owns(&row.request.owner)))
        .filter(|row| query.class.is_none_or(|class| row.request.class == class))
        .map(|mut row| { row.grant = None; row }).collect();
    let holds = if caller.is_none() { state.store.host_holds(Some(host), false).map_err(ApiError::internal)? } else { Vec::new() };
    let accountant = caller.is_none().then(|| state.seat_admission.snapshot(admission_now()));
    Ok(Json(json!({"executions":rows,"memberships":memberships,"holds":holds,"accountant":accountant})))
}

pub(super) async fn show(State(state): State<AppState>, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let caller = CallerOwner::resolve(&state, bound, run)?;
    let execution = state.store.shell_resource(&id).map_err(ApiError::internal)?;
    authorize_execution(caller.as_ref(), &execution)?;
    let membership = state.store.resource_memberships_for_host(&execution.command.lane.host).map_err(ApiError::internal)?.into_iter()
        .find(|member| member.request.id == execution.command.lane.id
            && caller.as_ref().is_none_or(|caller| caller.owns(&member.request.owner)))
        .map(|mut member| { member.grant = None; member });
    let holds = if caller.is_none() { state.store.host_holds(Some(&execution.command.lane.host), false).map_err(ApiError::internal)? } else { Vec::new() };
    let accountant = caller.is_none().then(|| state.seat_admission.snapshot(admission_now()));
    Ok(Json(json!({"execution":present(execution),"membership":membership,"holds":holds,"accountant":accountant})))
}

pub(super) async fn wait(State(state): State<AppState>, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>, AxumPath(id): AxumPath<String>, Query(query): Query<WaitQuery>) -> Result<Json<Execution>, ApiError> {
    let caller = CallerOwner::resolve(&state, bound, run)?;
    let mut changed = state.event_notify.subscribe();
    loop {
        let execution = state.store.shell_resource(&id).map_err(ApiError::internal)?;
        authorize_execution(caller.as_ref(), &execution)?;
        if execution.revision > query.after || matches!(execution.state.as_str(), "completed" | "cancelled" | "refused") { return Ok(Json(present(execution))); }
        changed.changed().await.map_err(ApiError::internal)?;
    }
}

pub(super) async fn cancel(State(state): State<AppState>, bound: Option<BoundActor>, run: Option<Extension<BoundRun>>, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let caller = CallerOwner::resolve(&state, bound, run)?;
    let execution = state.store.shell_resource(&id).map_err(ApiError::internal)?;
    authorize_execution(caller.as_ref(), &execution)?;
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

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn nested_submission_keeps_one_membership_and_rejects_a_stale_marker() {
        use crate::store::resource_membership::{Admission, Fit};
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        state.store.start_resource_authority(&state.node, "epoch").unwrap();
        let mut parent = request(); parent.host.clone_from(&state.node);
        parent.owner.run = state.owning_person.clone().unwrap_or_else(|| format!("local:{}", state.node));
        let Admission::Admitted(grant) = state.store.request_resource_membership(&parent,
            Fit { room: Some(fractal_runtime::Capacity { cores:32, memory:128 << 30 }), ..Fit::default() }, 10).unwrap() else { panic!("parent was not admitted") };
        let id = uuid::Uuid::now_v7().to_string();
        let owner = format!("shell:{id}");
        let mut lane = parent.clone(); lane.id.clone_from(&owner);
        lane.owner = Owner { run:owner.clone(), step:owner.clone(), attempt:1, invocation:Some(owner) };
        let command = Command { id, lane, argv:vec!["true".into()], cwd:"/work".into(), environment:BTreeMap::new(),
            timeout_ms:None, parent:Some(grant.clone()) };
        let mut other_owner = state.clone();
        other_owner.owning_person = Some("person/other".into());
        assert!(submit(State(other_owner), None, None, Json(command.clone())).await.is_err());
        let Json(child) = submit(State(state.clone()), None, None, Json(command.clone())).await.unwrap();
        assert_eq!(child.command.lane, parent);
        assert!(child.command.parent.is_none(), "inspection must not export replayable parent grants");
        let Json(details) = show(State(state.clone()), None, None, AxumPath(child.command.id.clone())).await.unwrap();
        assert!(details["membership"]["grant"].is_null(), "operator inspection must not export grants");
        assert!(state.store.shell_resource(&child.command.id).unwrap().command.parent.is_some());
        assert_eq!(state.store.resource_memberships_for_host(&state.node).unwrap().len(), 1);
        assert_eq!(state.store.held_resource_membership_bytes(&state.node, "").unwrap(), 24 << 30);
        state.store.cancel_resource_membership(&grant, &parent.owner, 11).unwrap();
        assert!(submit(State(state), None, None, Json(command)).await.is_err());
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
        let Json(released) = host_holds::release(State(state.clone()), None, Json(ReleaseHostHold {
            actor:hold.owner, host:hold.host, id:hold.id, expected_revision:hold.revision, resolution:"ready".into(),
        })).await.unwrap();
        assert_eq!(released.revision, hold.revision + 1);
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(Future::poll(wake.as_mut(),context).is_ready())).await);
        assert!(matches!(state.store.request_resource_membership(&lane,fit(),11).unwrap(), Admission::Admitted(_)));
    }

    #[tokio::test]
    async fn shell_wait_subscribes_before_reading_and_exports_no_launch_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        let mut lane = request();
        lane.host.clone_from(&state.node);
        let execution = state.store.submit_shell_resource(Command {
            id:"waiter".into(), lane, argv:vec!["true".into()], cwd:"/work".into(),
            environment:BTreeMap::from([("EXAMPLE_TOKEN".into(),"private-test-value".into())]), timeout_ms:None, parent:None,
        }, 10).unwrap();
        let waiting = wait(State(state.clone()),None,None,AxumPath("waiter".into()),Query(WaitQuery { after:0 }));
        tokio::pin!(waiting);
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(Future::poll(waiting.as_mut(),context).is_pending())).await);
        let mut completed = execution;
        completed.state = "completed".into(); completed.exit_code = Some(37);
        state.store.update_shell_resource(&completed).unwrap();
        state.event_notify.send_modify(|generation| *generation += 1);
        let Json(result) = waiting.await.unwrap();
        assert_eq!(result.exit_code,Some(37));
        assert!(result.command.environment.is_empty());
        assert!(state.store.shell_resource("waiter").unwrap().command.environment.is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn submission_persists_authenticated_owner_and_never_promotes_environment_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::state(directory.path());
        state.owning_person = Some("person/operator".into());
        let id = uuid::Uuid::now_v7().to_string();
        let transport = format!("shell:{id}");
        let mut lane = request(); lane.id.clone_from(&transport); lane.host.clone_from(&state.node);
        lane.owner = Owner { run:transport.clone(), step:transport.clone(), attempt:1, invocation:Some(transport.clone()) };
        let command = Command { id:id.clone(), lane, argv:vec!["true".into()], cwd:"/work".into(),
            environment:BTreeMap::from([("ST_MISSION_RUN".into(),"mission-run/spoofed".into())]), timeout_ms:None, parent:None };
        assert!(submit(State(state.clone()),Some(BoundActor::Agent("agent/missing".into())),None,Json(command.clone())).await.is_err());
        let Json(execution) = submit(State(state.clone()),None,None,Json(command)).await.unwrap();
        assert_eq!(execution.command.lane.owner.run,"person/operator");
        assert_eq!(execution.command.lane.owner.step,transport);
        let mut persisted = state.store.shell_resource(&id).unwrap();
        persisted.command.environment.clear();
        state.store.update_shell_resource(&persisted).unwrap();
        assert_eq!(state.store.shell_resource(&id).unwrap().command.lane.owner.run,"person/operator");
    }

    #[test]
    fn macos_exec_requires_whole_tree_retirement_proof() {
        let error = require_exec_platform(true).unwrap_err();
        assert_eq!(error.code, "unsupported-platform");
        assert!(error.message.contains("no whole-tree retirement proof"));
        assert!(require_exec_platform(false).is_ok());
        assert!(!allows_bound_request(&axum::http::Method::HEAD, "/v1/internal/resources"));
    }

    #[test]
    fn bound_resource_exception_matches_only_registered_methods_and_paths() {
        use axum::http::Method;
        for (path, allowed) in [
            ("/v1/internal/resources", Some(Method::GET)),
            ("/v1/internal/resources/exec", Some(Method::POST)),
            ("/v1/internal/resources/owned", Some(Method::GET)),
            ("/v1/internal/resources/owned/wait", Some(Method::POST)),
            ("/v1/internal/resources/owned/cancel", Some(Method::POST)),
            ("/v1/internal/resources/", None),
            ("/v1/internal/resources/owned/", None),
            ("/v1/internal/resources/owned/cancel/extra", None),
            ("/v1/internal/resources/owned/other", None),
            ("/v1/internal/resources-extra", None),
            ("/v1/internal/unrelated", None),
        ] {
            for method in [Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE, Method::HEAD, Method::OPTIONS] {
                assert_eq!(allows_bound_request(&method, path), allowed.as_ref() == Some(&method), "{method} {path}");
            }
        }
    }
    #[tokio::test]
    async fn resource_write_exception_keeps_native_attribution_and_ancestry_checks() {
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        let step = state.store.start_program_step_for_tests();
        let credential = state.store.issue_run_credential(&step.subject, 1, 60_000).unwrap();
        let binding = BoundRun(state.store.run_binding(&credential).unwrap());
        let request = |path: &str, actor: &str, unverifiable: bool| {
            let mut request = Request::builder().method("POST").uri(path)
                .header("content-type", "application/json").extension(binding.clone());
            if unverifiable { request = request.extension(HarnessAncestry::Unavailable); }
            request.body(Body::from(json!({"actor": actor}).to_string())).unwrap()
        };
        let _accepted = super::super::write_principal::bind_request(&state,
            request("/v1/internal/resources/exec", &binding.0.run, false)).await.unwrap();
        for (path, actor, unverifiable) in [
            ("/v1/internal/resources/exec", "person/other", false),
            ("/v1/internal/resources/child/other", binding.0.run.as_str(), false),
            ("/v1/internal/resources/exec", binding.0.run.as_str(), true),
        ] {
            let error = super::super::write_principal::bind_request(&state,
                request(path, actor, unverifiable)).await.unwrap_err();
            assert_eq!(error.code, "foreign-agent-actor", "{path}: {error:?}");
        }
    }


    async fn bound_request(state: &AppState, agent: Option<&str>, credential: Option<&str>, method: &str, path: &str, body: Value) -> (axum::http::StatusCode, Value) {
        use tower::ServiceExt;
        let app = router(state.clone()).layer(from_fn_with_state((state.store.clone(), None), guard_bound_actor));
        let mut request = Request::builder().method(method).uri(path).header("content-type", "application/json");
        if let Some(agent) = agent { request = request.extension(BoundAgent(agent.into())); }
        if let Some(credential) = credential { request = request.extension(RunCredential::new(credential)); }
        let response = app.oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let envelope: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!envelope.to_string().contains("must-not-export"), "the entire wire response must redact launch credentials");
        let value = if status.is_success() { envelope["value"].clone() } else { envelope };
        (status, value)
    }

    #[tokio::test]
    async fn bound_resource_routes_enforce_native_ownership_through_transport_guard() {
        use axum::http::StatusCode;
        use crate::store::invocation_receipts::native_receipt;
        use fractal_schema::invocation::{InvocationPhase, KIND};
        let directory = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(directory.path());
        let agent = "agent/host-a/worker";
        let receipt = native_receipt("resource-owner", agent,
            &json!({"subject":{"kind":"agent"},"launch_member":{"driver":"omp","environment":{}}}),
            InvocationPhase::Started, Some(&json!({"verification":"operation-incarnation","incarnation":"exact"}))).unwrap().unwrap();
        for (kind, fields) in [
            ("runtime.observed", json!({"status":"running","runtime_id":"worker","incarnation_id":"exact"})),
            ("harness.observed", json!({"state":"ready","driver":"omp","incarnation_id":"exact"})),
            (KIND, serde_json::to_value(receipt).unwrap()),
        ] {
            state.store.append_claim(&crate::model::ClaimInput { subject:agent.into(), kind:kind.into(), actor:None,
                fields:serde_json::from_value(fields).unwrap(), evidence:vec![], idempotency_key:None, expected_subject:None }).unwrap();
        }
        let agent_owner = state.store.shell_resource_agent_owner(agent).unwrap();
        let step = state.store.start_program_step_for_tests();
        let credential = state.store.issue_run_credential(&step.subject, 1, 60_000).unwrap();
        let binding = state.store.run_binding(&credential).unwrap();
        let run_owner = Owner { run:binding.run, step:binding.step_run, attempt:binding.attempt, invocation:Some("shell:run".into()) };
        state.store.start_resource_authority(&state.node, "epoch").unwrap();
        for (index, owner) in [agent_owner.clone(), run_owner.clone()].into_iter().enumerate() {
            let id = format!("owned-{index}");
            let mut lane = request(); lane.id.clone_from(&id); lane.host.clone_from(&state.node); lane.owner = owner;
            let mut execution = state.store.submit_shell_resource(Command { id, lane:lane.clone(), argv:vec!["true".into()],
                cwd:"/work".into(), environment:BTreeMap::from([("SECRET".into(),"must-not-export".into())]),
                timeout_ms:None, parent:None },10).unwrap();
            execution.state = "completed".into();
            state.store.update_shell_resource(&execution).unwrap();
            state.store.request_resource_membership(&lane, crate::store::resource_membership::Fit {
                room:Some(fractal_runtime::Capacity { cores:32, memory:128 << 30 }), ..Default::default()
            },10).unwrap();
        }
        for (index, bound_agent, run_credential) in [(0, Some(agent), None), (1, None, Some(credential.as_str()))] {
            let own = format!("owned-{index}");
            let other = format!("owned-{}", 1-index);
            let (status, list) = bound_request(&state,bound_agent,run_credential,"GET","/v1/internal/resources",Value::Null).await;
            assert_eq!(status,StatusCode::OK,"{list}");
            assert_eq!(list["executions"].as_array().unwrap().len(),1);
            assert_eq!(list["executions"][0]["command"]["id"],own);
            assert_eq!(list["memberships"].as_array().unwrap().len(),1);
            assert!(list["memberships"][0]["grant"].is_null());
            assert!(list["holds"].as_array().unwrap().is_empty());
            assert!(list["accountant"].is_null());
            assert!(!list.to_string().contains("must-not-export"));
            for (method, suffix) in [("GET",""),("POST","/wait?after=0"),("POST","/cancel")] {
                let (status, body) = bound_request(&state,bound_agent,run_credential,method,&format!("/v1/internal/resources/{other}{suffix}"),Value::Null).await;
                assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY,"{body}");
                assert_eq!(body["code"],"resource-exec-owner", "{method} {suffix}: {body}");
                let (status, body) = bound_request(&state,bound_agent,run_credential,method,&format!("/v1/internal/resources/{own}{suffix}"),Value::Null).await;
                assert_eq!(status,StatusCode::OK,"{body}");
                assert!(!body.to_string().contains("must-not-export"));
            }
            for (method,path) in [("GET","/v1/internal/unrelated"),("POST","/v1/internal/resources"),
                ("GET","/v1/internal/resources/exec"),("DELETE","/v1/internal/resources/owned-0"),
                ("POST","/v1/internal/resources/owned-0/cancel/extra")] {
                let (status, body) = bound_request(&state,bound_agent,run_credential,method,path,Value::Null).await;
                assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY,"{method} {path}: {body}");
                assert_eq!(body["code"],"foreign-agent-actor");
            }
            let id = uuid::Uuid::now_v7().to_string();
            let transport = format!("shell:{id}");
            let mut lane = request(); lane.id.clone_from(&transport); lane.host.clone_from(&state.node);
            lane.owner = Owner { run:transport.clone(), step:transport.clone(), attempt:1, invocation:Some(transport) };
            let command = Command { id:id.clone(), lane, argv:vec!["true".into()], cwd:"/work".into(),
                environment:BTreeMap::from([("ST_MISSION_RUN".into(),"spoofed".into())]), timeout_ms:None,parent:None };
            let (status, body) = bound_request(&state,bound_agent,run_credential,"POST","/v1/internal/resources/exec",serde_json::to_value(command).unwrap()).await;
            if cfg!(target_os = "macos") {
                assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY,"{body}");
                assert_eq!(body["code"],"unsupported-platform");
                assert!(state.store.shell_resource(&id).is_err());
            } else {
                assert_eq!(status,StatusCode::OK,"{body}");
                let persisted = state.store.shell_resource(&id).unwrap();
                let expected = if index == 0 { &agent_owner } else { &run_owner };
                assert_eq!(persisted.command.lane.owner.run,expected.run);
                assert_eq!(persisted.command.lane.owner.step,expected.step);
                assert_eq!(persisted.command.lane.owner.attempt,1);
                assert!(!body.to_string().contains("spoofed"));
            }
        }
        state.store.set_step_run_state_for_tests(&step.subject, "working", 2);
        for (method,path) in [("GET","/v1/internal/resources"),("GET","/v1/internal/resources/owned-1"),
            ("POST","/v1/internal/resources/owned-1/wait?after=0"),("POST","/v1/internal/resources/owned-1/cancel")] {
            let (status, body) = bound_request(&state,None,Some(&credential),method,path,Value::Null).await;
            assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY,"{body}");
            assert_eq!(body["code"],"foreign-agent-actor", "stale credentials must not fall back to operator authority");
        }
    }

    #[test]
    fn owner_scope_separates_attempts_steps_and_agent_invocations() {
        let owner = request().owner;
        let caller = CallerOwner { owner:owner.clone(), agent:true };
        assert!(caller.owns(&owner));
        let mut other = owner.clone(); other.attempt += 1;
        assert!(!caller.owns(&other));
        let mut other = owner.clone(); other.step.push_str("-other");
        assert!(!caller.owns(&other));
        let mut other = owner.clone(); other.run.push_str("-other");
        assert!(!caller.owns(&other));
        let mut other = owner.clone(); other.invocation = Some("another-invocation".into());
        assert!(!caller.owns(&other));
        let run = CallerOwner { owner, agent:false };
        assert!(run.owns(&other), "run scope is the authenticated step attempt, not one shell ID");
    }
}
