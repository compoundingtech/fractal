//! Real POSIX processes, PTY servers, and durable journals; no fake native backend.
use super::*;
use fractal_light_node_core::{custody::MemberLifecycle, journal::{Admission, CommandDraft, Limits, OperationKind, ReceiptState, Scope}};
use std::{collections::BTreeMap, path::Path};

struct Fixture {
    root: tempfile::TempDir,
    selection: Selection,
    home: Journal,
    seat: Journal,
    pty: PtyRuntime,
    exec: ExecRuntime,
    driver: NativeExecutor,
    local: LocalSession,
    binding: ExecutionBinding,
}

impl Fixture {
    fn new(borrowed: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let selection = Selection {
            home: "example-home".into(), home_key: fractal_auth::MemberKey::generate().unwrap().0.public().into(),
            host: "example-seat".into(), host_key: fractal_auth::MemberKey::generate().unwrap().0.public().into(), epoch: 1,
            grant: Scope { subjects: vec!["agent/example/worker".into()], operations: vec![OperationKind::Observe, OperationKind::Stop] },
        };
        let home = Journal::open(&root.path().join("home.sqlite3"), Limits::default()).unwrap();
        let seat = Journal::open(&root.path().join("seat.sqlite3"), Limits::default()).unwrap();
        home.install_epoch(&selection).unwrap();
        seat.install_epoch(&selection).unwrap();
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let binary = fractal_runtime::resolve_executable("pty", &environment).expect("native tests require the real pinned pty binary");
        let pty = PtyRuntime::new(root.path().join("pty")).with_binary(binary);
        spawn_pty(&pty, root.path());
        let item = pty.snapshot().unwrap().into_iter().find(|item| item.name == "native-worker").unwrap();
        let generation = format!("{}:{}", item.pid.unwrap(), item.created_at.unwrap());
        let incarnation = if borrowed { format!("{generation}:bound:explicit-invocation") } else { generation.clone() };
        let local = LocalSession {
            subject: selection.grant.subjects[0].clone(), runtime_id: "native-worker".into(),
            incarnation: incarnation.clone(), native_incarnation: generation, terminal: true, driver: None,
            lifecycle: if borrowed { MemberLifecycle::TerminalBound } else { MemberLifecycle::Service },
            custody: if borrowed { SessionCustody::Borrowed } else { SessionCustody::Owned },
            start_operation: None, invocation: borrowed.then_some(incarnation),
        };
        let exec = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let driver = NativeExecutor::new(selection.host.clone(), seat.clone(), pty.clone(), exec.clone()).unwrap();
        driver.bootstrap(&selection, std::slice::from_ref(&local)).unwrap();
        let binding = execution_binding(&selection, &local).unwrap();
        Self { root, selection, home, seat, pty, exec, driver, local, binding }
    }

    fn acknowledge(&self) {
        for change in self.seat.pending_authority().unwrap() {
            let through = self.home.accept_authority(&change, |change, _| {
                assert!(change.binding.run.is_none());
                Ok(())
            }).unwrap();
            self.seat.acknowledge_authority(through).unwrap();
        }
        self.driver.refresh_bindings().unwrap();
    }

    fn command(&self, decision: &str, operation: Operation) -> Command {
        self.home.enqueue(CommandDraft { decision_id: decision.into(), binding: self.binding.clone(), operation }).unwrap()
    }

    fn enter(&self, command: &Command) -> Recovery {
        assert!(matches!(self.seat.admit(command).unwrap(), Admission::Accepted(_)));
        self.seat.enter(&command.execution_id, |command, authority| self.driver.enter(command, authority)).unwrap()
    }

    fn reopened(&self) -> (Journal, NativeExecutor) {
        let seat = Journal::open(&self.root.path().join("seat.sqlite3"), Limits::default()).unwrap();
        let driver = NativeExecutor::new(self.selection.host.clone(), seat.clone(), self.pty.clone(), self.exec.clone()).unwrap();
        (seat, driver)
    }

    fn running(&self) -> bool {
        self.pty.snapshot().unwrap().into_iter().any(|item| item.name == "native-worker" && item.status == "running")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.pty.stop("native-worker");
        if let Ok(Some(ExecObservation::Running(generation) | ExecObservation::Exited(generation))) =
            self.exec.observe("native-exec") {
            let _ = self.exec.kill_if("native-exec", Some(&generation.generation_id));
            let _ = self.exec.wait_for_exit("native-exec", &generation.generation_id);
        }
    }
}

fn spawn_pty(pty: &PtyRuntime, cwd: &Path) {
    pty.spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()), cwd,
        &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
}

#[test]
fn native_entry_requires_home_ack_and_current_exact_authority() {
    let fixture = Fixture::new(false);
    let command = Command { version: 1, sequence: 1, execution_id: "direct-observation".into(),
        decision_id: "direct-observation".into(), binding: fixture.binding.clone(), operation: Operation::Observe };
    assert!(fixture.seat.with_authority(|authority| fixture.driver.enter(&command, authority)).is_err());
    assert!(fixture.running());
    fixture.acknowledge();
    assert!(fixture.seat.with_authority(|authority| fixture.driver.enter(&command, authority)).is_ok());
    let mut wrong_epoch = command.clone();
    wrong_epoch.binding.epoch += 1;
    assert!(fixture.seat.with_authority(|authority| fixture.driver.enter(&wrong_epoch, authority)).is_err());
    let mut start = command;
    start.operation = Operation::Start;
    let error = fixture.seat.with_authority(|authority| fixture.driver.enter(&start, authority)).unwrap_err();
    assert_eq!(error.downcast_ref::<fractal_wire::Error>().unwrap().code, "light-node-start-not-supported");
}

#[tokio::test]
async fn owned_stop_retains_positive_proof_before_restart_and_receipt() {
    let fixture = Fixture::new(false);
    fixture.acknowledge();
    let observe = fixture.command("observe", Operation::Observe);
    let recovery = fixture.enter(&observe);
    assert!(matches!(fixture.driver.recover(recovery).await.unwrap(), Outcome::Observed { .. }));
    assert!(fixture.running());
    let stop = fixture.command("stop", Operation::Stop);
    let recovery = fixture.enter(&stop);
    let NativeProof::Stop { retired: true, .. } = serde_json::from_str(recovery.custody_proof.as_ref().unwrap()).unwrap() else {
        panic!("physical stop must retain positive proof, not signal acceptance");
    };
    assert!(fixture.seat.with_authority(|authority| custody::retirement_verified(authority, &fixture.local)).unwrap());
    let (seat, driver) = fixture.reopened();
    // The original private bootstrap file remains valid, but cannot revive settled custody.
    driver.bootstrap(&fixture.selection, std::slice::from_ref(&fixture.local)).unwrap();
    assert_eq!(driver.recover(recovery).await.unwrap(), Outcome::Stopped { incarnation: fixture.local.incarnation.clone() });
    assert!(seat.with_authority(|authority| Ok(custody::local_session(authority, &fixture.local.subject)?.is_none())).unwrap());
    assert!(seat.pending_terminal_results().unwrap().is_empty());
    let receipt = seat.pending_receipts().unwrap().into_iter().find(|receipt|
        receipt.execution_id == stop.execution_id && matches!(receipt.state, ReceiptState::Settled { .. })).unwrap();
    assert!(matches!(receipt.state, ReceiptState::Settled { outcome: Outcome::Stopped { .. } }));
    assert!(!fixture.running());
    driver.bootstrap(&fixture.selection, std::slice::from_ref(&fixture.local)).unwrap();
    assert!(seat.with_authority(|authority| Ok(custody::local_session(authority, &fixture.local.subject)?.is_none())).unwrap());
}

#[tokio::test]
async fn borrowed_stop_is_durable_detach_and_does_not_terminate_shell() {
    let fixture = Fixture::new(true);
    fixture.acknowledge();
    let command = fixture.command("detach", Operation::Stop);
    let recovery = fixture.enter(&command);
    assert!(fixture.running());
    let (seat, driver) = fixture.reopened();
    driver.bootstrap(&fixture.selection, std::slice::from_ref(&fixture.local)).unwrap();
    let outcome = driver.recover(recovery).await.unwrap();
    assert_eq!(outcome, Outcome::Detached { incarnation: fixture.local.incarnation.clone() });
    seat.complete(&command.execution_id, outcome).unwrap();
    assert!(seat.with_authority(|authority| custody::detached(authority, &fixture.local.subject, &fixture.local.incarnation)).unwrap());
    assert!(!seat.with_authority(|authority| custody::retirement_verified(authority, &fixture.local)).unwrap());
    assert!(fixture.running(), "borrowed invocation detach must leave the person's shell alive");
    let (_, restarted) = fixture.reopened();
    drop(restarted);
    assert!(fixture.running(), "executor drop/restart is not process ownership");
}

#[test]
fn explicit_provisioning_rejects_provenance_changes_and_borrowed_escalation() {
    let fixture = Fixture::new(true);
    let mut changed = fixture.local.clone();
    changed.start_operation = Some("invented-operation".into());
    assert!(fixture.driver.bootstrap(&fixture.selection, &[changed]).is_err());
    let mut escalation = fixture.local.clone();
    escalation.custody = SessionCustody::Owned;
    escalation.incarnation = escalation.native_incarnation.clone();
    escalation.invocation = None;
    escalation.lifecycle = MemberLifecycle::Service;
    custody::validate(&escalation).unwrap();
    assert!(fixture.driver.bootstrap(&fixture.selection, &[escalation]).is_err());
    let mut missing_fence = fixture.local.clone();
    missing_fence.invocation = None;
    assert!(fixture.driver.bootstrap(&fixture.selection, &[missing_fence]).is_err());
    let mut fresh = fixture.local.clone();
    fresh.incarnation = format!("{}:bound:fresh", fresh.native_incarnation);
    fresh.invocation = Some(fresh.incarnation.clone());
    fresh.start_operation = Some("not-the-native-launch".into());
    assert!(fixture.driver.bootstrap(&fixture.selection, &[fresh]).is_err());
    let mut ungranted = fixture.local.clone();
    ungranted.subject = "agent/example/not-granted".into();
    assert!(fixture.driver.bootstrap(&fixture.selection, &[ungranted]).is_err());
    assert!(fixture.running());
}

#[tokio::test]
async fn entered_unknown_stop_cannot_close_reused_native_generation() {
    let fixture = Fixture::new(false);
    fixture.acknowledge();
    let command = fixture.command("interrupted-before-native-entry", Operation::Stop);
    fixture.seat.admit(&command).unwrap();
    assert!(fixture.seat.enter(&command.execution_id, |_, _| anyhow::bail!("crash before retained proof")).is_err());
    let recovery = fixture.seat.recoverable().unwrap().into_iter().find(|item| item.command.execution_id == command.execution_id).unwrap();
    assert!(recovery.custody_proof.is_none());
    fixture.pty.stop_if("native-worker", Some(&fixture.local.native_incarnation)).unwrap();
    fixture.pty.remove("native-worker").unwrap();
    spawn_pty(&fixture.pty, fixture.root.path());
    let replacement = fixture.pty.snapshot().unwrap().into_iter().find(|item| item.name == "native-worker").unwrap();
    assert_ne!(format!("{}:{}", replacement.pid.unwrap(), replacement.created_at.unwrap()), fixture.local.native_incarnation);
    let (seat, driver) = fixture.reopened();
    assert!(driver.recover(recovery).await.is_err(), "absence/reused runtime ID is not whole-scope proof");
    assert!(!seat.with_authority(|authority| custody::retirement_verified(authority, &fixture.local)).unwrap());
    assert!(fixture.running(), "unknown old Stop must not signal a replacement generation");
    assert!(!seat.pending_receipts().unwrap().iter().any(|receipt|
        receipt.execution_id == command.execution_id && matches!(receipt.state, ReceiptState::Settled { .. })));
}

#[test]
fn constructor_repairs_pending_finalization_only_from_retained_positive_proof() {
    let fixture = Fixture::new(false);
    fixture.acknowledge();
    let command = fixture.command("stop-before-finalization-crash", Operation::Stop);
    let recovery = fixture.enter(&command);
    let outcome = Outcome::Stopped { incarnation: fixture.local.incarnation.clone() };
    fixture.seat.complete_requiring_local_finalization(&command.execution_id, outcome, recovery.custody_proof.as_ref().unwrap()).unwrap();
    assert_eq!(fixture.seat.pending_terminal_results().unwrap().len(), 1);
    let (seat, _) = fixture.reopened();
    assert!(seat.pending_terminal_results().unwrap().is_empty());
    assert!(seat.with_authority(|authority| Ok(custody::local_session(authority, &fixture.local.subject)?.is_none())).unwrap());
    assert!(seat.with_authority(|authority| custody::retirement_verified(authority, &fixture.local)).unwrap());
}

#[tokio::test]
async fn real_exec_stop_requires_whole_scope_proof_not_just_process_exit() {
    let fixture = Fixture::new(false);
    fixture.pty.stop("native-worker").unwrap();
    let generation = fixture.exec.clone().with_launch_operation(Some(fractal_runtime::LaunchOperation("explicit-exec-launch".into())))
        .spawn("native-exec", &fractal_runtime::Launch::Shell("exec sleep 600".into()), fixture.root.path(), &std::env::vars().collect()).unwrap();
    let binding = LocalSession {
        subject: fixture.local.subject.clone(), runtime_id: "native-exec".into(),
        incarnation: generation.generation_id.clone(), native_incarnation: generation.generation_id.clone(),
        terminal: false, driver: None, lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
        start_operation: Some("explicit-exec-launch".into()), invocation: None,
    };
    fixture.driver.bootstrap(&fixture.selection, std::slice::from_ref(&binding)).unwrap();
    fixture.acknowledge();
    // Replacement authority must drain the revoked native generation before a fresh offer.
    fixture.driver.refresh_bindings().unwrap();
    fixture.acknowledge();
    let execution = execution_binding(&fixture.selection, &binding).unwrap();
    let command = fixture.home.enqueue(CommandDraft { decision_id: "real-exec-stop".into(), binding: execution, operation: Operation::Stop }).unwrap();
    let recovery = fixture.enter(&command);
    let outcome = fixture.driver.recover(recovery).await;
    if generation.scope_unit.is_some() {
        assert_eq!(outcome.unwrap(), Outcome::Stopped { incarnation: binding.incarnation.clone() });
        assert!(fixture.exec.retired_if("native-exec", &binding.native_incarnation).unwrap());
        assert!(fixture.seat.with_authority(|authority| custody::retirement_verified(authority, &binding)).unwrap());
    } else {
        assert!(outcome.is_err(), "logical-only containment cannot prove escaped descendant retirement");
        assert!(!fixture.seat.with_authority(|authority| custody::retirement_verified(authority, &binding)).unwrap());
        assert!(!fixture.seat.pending_receipts().unwrap().iter().any(|receipt|
            receipt.execution_id == command.execution_id && matches!(receipt.state, ReceiptState::Settled { .. })));
    }
}

#[test]
fn provisioning_waits_for_the_exact_configured_open_epoch_without_blocking_drain() {
    let fixture = Fixture::new(false);
    let mut successor = fixture.selection.clone();
    successor.epoch += 1;
    let mut future = fixture.local.clone();
    future.subject = "agent/example/future".into();
    successor.grant.subjects.push(future.subject.clone());
    assert!(!fixture.driver.bootstrap(&successor, std::slice::from_ref(&future)).unwrap());
    assert!(fixture.seat.with_authority(|authority|
        Ok(custody::local_session(authority, &future.subject)?.is_none())).unwrap());
    assert_eq!(fixture.seat.selection().unwrap(), fixture.selection);
    fixture.seat.begin_closing().unwrap();
    assert!(!fixture.driver.bootstrap(&fixture.selection, std::slice::from_ref(&fixture.local)).unwrap());
    assert_eq!(fixture.seat.with_authority(|authority|
        custody::local_session(authority, &fixture.local.subject)).unwrap(), Some(fixture.local.clone()));
    assert!(fixture.running(), "pending epoch provisioning must not signal or relabel native custody");
}
