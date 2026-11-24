//! Home graph admission and withdrawal stay outside the graph-free executor.
use super::*;
use fractal_daemon::light_node::journal::{OperationKind, StartWorkspaceAdmission};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartEnqueue {
    host: String,
    decision_id: String,
    binding: ExecutionBinding,
    revision: u64,
    workspace_admission: StartWorkspaceAdmission,
}

pub(super) async fn enqueue_start(State(controls): State<Controls>, Json(request): Json<StartEnqueue>)
    -> Result<Json<Value>, ControlError> {
    let admission = (|| -> Result<Operation> {
        request.binding.validate()?;
        anyhow::ensure!(!request.decision_id.is_empty() && request.decision_id.len() <= 1024,
            "remote Start decision identity must be bounded and stable");
        let journal = controls.journals.get(&request.host).context("host has no explicit session grant")?;
        let selected = journal.selection()?;
        anyhow::ensure!(request.host == request.binding.host && selected.host == request.binding.host
            && selected.home == request.binding.home && selected.epoch == request.binding.epoch
            && selected.grant.operations.contains(&OperationKind::Start)
            && selected.grant.subjects.contains(&request.binding.subject)
            && request.binding.run.is_none()
            && request.binding.authorization_revision == request.revision.to_string(),
            "remote Start is outside its selected exact grant");
        let store = &controls.reconciler.store;
        let desired = store.desired_subjects_named(std::slice::from_ref(&request.binding.subject))?
            .into_iter().next().context("remote Start has no selected declaration")?;
        let member = desired.member.as_ref().context("remote Start declaration has no physical member")?;
        anyhow::ensure!(member.runtime_id == request.binding.runtime
            && member.host.trim_start_matches("host/") == request.host,
            "remote Start names another declaration's native member");
        let token = store.selected_desired_token(&desired.subject)?.context("remote Start declaration disappeared")?;
        let intent = store.append_claim(&ClaimInput {
            subject: desired.subject.clone(), kind: "runtime.action.requested".into(), actor: None,
            fields: BTreeMap::from([
                ("action".into(), json!("start")), ("operation".into(), json!(request.binding.incarnation)),
                ("runtime_id".into(), json!(member.runtime_id)), ("terminal".into(), json!(member.terminal)),
            ]),
            evidence: vec![token], expected_subject: None,
            idempotency_key: Some(format!("remote-start-request:{}", request.decision_id)),
        })?;
        let authorization = controls.reconciler.prepare_home_start(&request.binding, request.revision,
            &intent.id, request.workspace_admission.clone())?;
        Ok(Operation::Start { authorization: Box::new(authorization) })
    })().map_err(control_error)?;
    // The shared queue rechecks the complete graph decision while holding its writer,
    // then admits to the durable home journal before releasing that decision cut.
    super::enqueue(State(controls), Json(Enqueue { host: request.host, decision_id: request.decision_id,
        binding: request.binding, operation: admission })).await
}

fn reconcile_home_starts(controls: &Controls) -> Result<()> {
    for journal in controls.journals.values() {
        let withdrawing = journal.pending_cancellations()?.into_iter()
            .map(|(execution, _)| execution).collect::<std::collections::BTreeSet<_>>();
        for command in journal.home_active_starts()? {
            if withdrawing.contains(&command.execution_id) { continue; }
            let Operation::Start { authorization } = &command.operation else { continue; };
            let _decision = controls.reconciler.store.connection.write();
            match controls.reconciler.validate_retained_home_start(&command.binding, authorization) {
                Ok(()) => {}
                Err(error) if error.downcast_ref::<FractalError>().is_some() => {
                    // This writes ordered withdrawal, not a fabricated physical completion.
                    // Entered/native custody must return its exact terminal receipt first.
                    journal.request_cancel_if_unset(&command.execution_id, "home Start authorization changed")?;
                }
                Err(error) => return Err(error.context("read home Start authorization for withdrawal")),
            }
        }
    }
    Ok(())
}

pub(super) async fn watch_home_starts(controls: Controls, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    let wake = controls.changed.clone();
    // Callback does no SQL and owns no Store, avoiding writer reentrancy and retention cycles.
    // Register before the initial restart reconciliation; unrelated commits cannot lose a wake.
    let _commits = controls.reconciler.store.connection.observe_tables(
        &["claims", "local_observations", "desired"], move |_| wake.notify_one());
    loop {
        if *shutdown.borrow() { return Ok(()); }
        let changed = controls.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        reconcile_home_starts(&controls)?;
        tokio::select! {
            _ = changed => {}
            stopped = shutdown.changed() => {
                if stopped.is_err() || *shutdown.borrow() { return Ok(()); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_daemon::{MissionsStore as _, parse_intent};
    use fractal_daemon::light_node::journal::{Command, Scope};
    use fractal_daemon::model::IntentInput;
    use fractal_daemon::reconcile::NativeRuntime;
    use fractal_daemon::store::Store;

    fn publish(store: &Store, workspace: &Path, command: &str, key: &str) {
        let source = format!("version 2\nagent \"worker\" {{ host \"example-seat\"; workspace {:?}; command {command:?} }}\n",
            workspace.display().to_string());
        let intent = parse_intent(&source, "example-home").unwrap();
        let plan = store.mission(&intent, IntentInput { kdl: source, source_name: None }).unwrap();
        store.apply(&intent, &plan.subject_tokens, key).unwrap();
    }

    fn fixture(root: &Path) -> (Controls, ExecutionBinding) {
        let store = Arc::new(Store::open(&root.join("home-graph.sqlite3"), "example-home").unwrap());
        publish(&store, root, "sleep 100", "first-declaration");
        let desired = store.desired_subjects().unwrap().into_iter().find(|desired| desired.kind == "agent").unwrap();
        let member = desired.member.as_ref().unwrap();
        let home_key = MemberKey::generate().unwrap().0;
        let seat_key = MemberKey::generate().unwrap().0;
        let selected = Selection { home: "example-home".into(), home_key: home_key.public().into(),
            host: "example-seat".into(), host_key: seat_key.public().into(), epoch: 1,
            grant: Scope { subjects: vec![desired.subject.clone()],
                operations: vec![OperationKind::Start, OperationKind::Observe, OperationKind::Stop] } };
        let journal = Journal::open(&root.join("home-session.sqlite3"), Limits::default()).unwrap();
        journal.install_epoch(&selected).unwrap();
        let runtime = Arc::new(NativeRuntime::new(root, Some(&root.join("pty")), Path::new("unused-pty")));
        let reconciler = Arc::new(Reconciler::new(store, runtime, selected.home.clone(), Arc::default())
            .with_person(Some("person/avery".into())));
        let binding = ExecutionBinding { version: 1, home: selected.home, host: selected.host.clone(),
            epoch: selected.epoch, subject: desired.subject, runtime: member.runtime_id.clone(),
            incarnation: "fresh-launch-proposal".into(), authorization_revision: "1".into(), run: None };
        (Controls { journals: BTreeMap::from([(selected.host, journal)]), reconciler,
            changed: Arc::default() }, binding)
    }

    async fn queue(controls: &Controls, binding: &ExecutionBinding) -> Command {
        let Json(value) = enqueue_start(State(controls.clone()), Json(StartEnqueue {
            host: binding.host.clone(), decision_id: "home-start/one".into(), binding: binding.clone(),
            revision: 1, workspace_admission: StartWorkspaceAdmission::NonGit,
        })).await.unwrap();
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn home_constructs_and_rechecks_graph_bound_payload_before_admission() {
        let root = tempfile::tempdir().unwrap();
        let (controls, binding) = fixture(root.path());
        let command = queue(&controls, &binding).await;
        assert_eq!(queue(&controls, &binding).await.execution_id, command.execution_id);
        let Operation::Start { mut authorization } = command.operation else { panic!("expected Start"); };
        controls.reconciler.validate_home_start(&binding, &authorization).unwrap();
        authorization.launch_member.launch = fractal_daemon::model::LaunchSpec::Shell("false".into());
        authorization.seal().unwrap();
        let error = super::super::enqueue(State(controls.clone()), Json(Enqueue {
            host: binding.host.clone(), decision_id: "home-start/tampered".into(), binding,
            operation: Operation::Start { authorization },
        })).await.unwrap_err();
        assert_eq!(error.1.0["error"]["code"], "light-node-start-authorization-changed");
        assert_eq!(controls.journals["example-seat"].pending_commands().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn actual_graph_commit_orders_withdrawal_without_claiming_native_completion() {
        let root = tempfile::tempdir().unwrap();
        let (controls, binding) = fixture(root.path());
        let command = queue(&controls, &binding).await;
        let journal = controls.journals["example-seat"].clone();
        let (shutdown, stopped) = watch::channel(false);
        let watcher = tokio::spawn(watch_home_starts(controls.clone(), stopped));
        let changed = journal.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        publish(&controls.reconciler.store, root.path(), "sleep 200", "replacement-declaration");
        tokio::time::timeout(std::time::Duration::from_secs(5), changed).await.unwrap();
        assert_eq!(journal.pending_cancellations().unwrap(),
            vec![(command.execution_id.clone(), "home Start authorization changed".to_owned())]);
        assert!(journal.command_receipt(&command.execution_id).unwrap().is_none(),
            "home withdrawal cannot fabricate native producer closure");
        shutdown.send(true).unwrap();
        watcher.await.unwrap().unwrap();
        // Restart reconciliation retains the same first intent and performs no duplicate writes.
        reconcile_home_starts(&controls).unwrap();
        assert_eq!(journal.pending_cancellations().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn person_geometry_is_sealed_at_admission_but_later_resize_does_not_stop_live_work() {
        let root = tempfile::tempdir().unwrap();
        let (controls, binding) = fixture(root.path());
        let geometry = |rows, columns| controls.reconciler.store.append_client_claim(
            &fractal_daemon::model::ClaimInput {
                subject: "person/avery".into(), kind: "terminal.launch-geometry".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([("rows".into(), Value::from(rows)),
                    ("columns".into(), Value::from(columns))]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        geometry(24, 80);
        let command = queue(&controls, &binding).await;
        let Operation::Start { authorization } = &command.operation else { panic!("expected Start"); };
        let size = authorization.launch_member.terminal_size.as_ref().unwrap();
        assert_eq!((size.rows, size.columns), (24, 80));
        controls.reconciler.validate_home_start(&binding, authorization).unwrap();
        geometry(48, 160);
        assert!(controls.reconciler.validate_home_start(&binding, authorization).is_err(),
            "a newly admitted command cannot substitute an outdated launch preference");
        reconcile_home_starts(&controls).unwrap();
        assert!(controls.journals["example-seat"].pending_cancellations().unwrap().is_empty(),
            "changing a launch-only preference must not retire an admitted producer");
    }

    #[tokio::test]
    async fn a_running_harness_session_publication_does_not_revoke_its_original_launch() {
        let root = tempfile::tempdir().unwrap();
        let (controls, mut binding) = fixture(root.path());
        let store = &controls.reconciler.store;
        let source = format!("version 2\nagent \"worker\" {{ host \"example-seat\"; workspace {:?}; harness \"codex\" {{}} }}\n",
            root.path().display().to_string());
        let intent = parse_intent(&source, "example-home").unwrap();
        let plan = store.mission(&intent, IntentInput { kdl: source, source_name: None }).unwrap();
        store.apply(&intent, &plan.subject_tokens, "harness-declaration").unwrap();
        let desired = store.desired_subjects_named(std::slice::from_ref(&binding.subject)).unwrap().remove(0);
        binding.runtime = desired.member.unwrap().runtime_id;
        let command = queue(&controls, &binding).await;
        let Operation::Start { authorization } = &command.operation else { panic!("expected Start"); };
        assert!(authorization.continuation.is_none());
        store.append_client_claim(&fractal_daemon::model::ClaimInput {
            subject: binding.subject.clone(), kind: "harness.session-file".into(),
            actor: Some(binding.subject.clone()),
            fields: BTreeMap::from([("harness".into(), Value::from("codex")),
                ("session_id".into(), Value::from("actual-session")),
                ("path".into(), Value::from("sessions/actual-session.jsonl"))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(controls.reconciler.validate_home_start(&binding, authorization).is_err(),
            "new admission must still recheck its selected continuation");
        controls.reconciler.validate_retained_home_start(&binding, authorization).unwrap();
        reconcile_home_starts(&controls).unwrap();
        assert!(controls.journals["example-seat"].pending_cancellations().unwrap().is_empty(),
            "a producer's own session publication must not retire its admitted launch");
    }
}
