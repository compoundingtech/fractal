use super::*;

pub(super) fn selection() -> Selection {
    Selection {
        home: "example-home".into(), home_key: smallclaims::fleet::MemberKey::generate().unwrap().0.public().into(),
        host: "example-seat".into(), host_key: smallclaims::fleet::MemberKey::generate().unwrap().0.public().into(), epoch: 1,
        grant: Scope { subjects: vec!["agent/example/worker".into()], operations: vec![OperationKind::Observe, OperationKind::Stop] },
    }
}

pub(super) fn draft(selection: &Selection, decision: &str) -> CommandDraft {
    CommandDraft { decision_id: decision.into(), binding: ExecutionBinding {
        version: VERSION, home: selection.home.clone(), host: selection.host.clone(), epoch: selection.epoch,
        subject: selection.grant.subjects[0].clone(), runtime: "runtime/example".into(), incarnation: "native-birth/example".into(),
        authorization_revision: format!("authorization/{}", selection.epoch), run: None,
    }, operation: Operation::Observe }
}

fn pair(root: &Path, selection: &Selection) -> (Journal, Journal) {
    let home = Journal::open(&root.join("home.sqlite3"), Limits::default()).unwrap();
    let seat = Journal::open(&root.join("seat.sqlite3"), Limits::default()).unwrap();
    home.install_epoch(selection).unwrap(); seat.install_epoch(selection).unwrap();
    let binding=draft(selection,"fixture").binding;
    seat.offer_execution(&binding).unwrap();
    (home,seat)
}

fn drain_authority(home:&Journal,seat:&Journal) {
    for change in seat.pending_authority().unwrap() {
        let through=home.accept_authority(&change,|change,_| {
            assert!(change.binding.run.is_none(),"fixture has no graph run authority to grant");
            Ok(())
        }).unwrap();
        seat.acknowledge_authority(through).unwrap();
    }
}

fn acknowledge_epoch_ready(home:&Journal,seat:&Journal,epoch:u64) {
    home.begin_closing().unwrap();
    assert!(home.drained().unwrap());
    assert!(seat.drained().unwrap());
    home.mark_peer_drained(epoch).unwrap();
    seat.mark_peer_drained(epoch).unwrap();
}

#[test]
fn admission_receipts_and_decision_identity_survive_restart() {
    let root = tempfile::tempdir().unwrap(); let selection = selection();
    let (home,seat) = pair(root.path(), &selection);
    let command = home.enqueue(draft(&selection,"decision/one")).unwrap();
    home.mark_sent(&command).unwrap();
    let Admission::Accepted(accepted) = seat.admit(&command).unwrap() else { panic!("first admission") };
    drop(home); drop(seat);
    let (home,seat) = pair(root.path(), &selection);
    assert_eq!(home.enqueue(draft(&selection,"decision/one")).unwrap().execution_id,command.execution_id);
    assert_eq!(seat.admit(&command).unwrap(),Admission::Duplicate(accepted.clone()));
    assert_eq!(home.accept_receipt(&accepted).unwrap(),1);
    assert_eq!(home.accept_receipt(&accepted).unwrap(),1);
    seat.acknowledge_receipts(1).unwrap();
    assert!(seat.pending_receipts().unwrap().is_empty());
    let mut conflict = command.clone(); conflict.operation = Operation::Stop;
    assert!(seat.admit(&conflict).is_err());
    assert_eq!(seat.undispatched().unwrap().len(),1);
}

#[test]
fn sequence_gaps_and_cross_epoch_identity_reuse_are_refused() {
    let root = tempfile::tempdir().unwrap(); let selection = selection(); let (home,seat) = pair(root.path(), &selection);
    let first = home.enqueue(draft(&selection,"decision/one")).unwrap();
    let second = home.enqueue(draft(&selection,"decision/two")).unwrap();
    assert_eq!(seat.admit(&second).unwrap(),Admission::Replay { next_sequence: 1 });
    seat.admit(&first).unwrap(); seat.admit(&second).unwrap();
    let mut next = selection.clone(); next.epoch = 2;
    assert_eq!(seat.install_epoch(&next).unwrap(),EpochInstall::Draining,"terminal receipts must drain before a different home/epoch can take over");
    assert_eq!(seat.pending_receipts().unwrap().len(),4,"accepted and cancelled receipts remain durable");
    drain_authority(&home,&seat);
    for receipt in seat.pending_receipts().unwrap() {
        let through=home.accept_receipt(&receipt).unwrap(); seat.acknowledge_receipts(through).unwrap();
    }
    acknowledge_epoch_ready(&home,&seat,selection.epoch);
    assert_eq!(seat.install_epoch(&next).unwrap(),EpochInstall::Installed);
    assert_eq!(home.install_epoch(&next).unwrap(),EpochInstall::Installed);
    let mut reused = first.clone();
    reused.binding.epoch = 2;
    assert!(seat.admit(&reused).is_err());
    assert!(seat.admit(&first).is_err());
    assert!(seat.install_epoch(&selection).is_err());
}

#[test]
fn uncertain_backend_entry_requires_reconciliation_and_blocks_epoch_replacement() {
    let root = tempfile::tempdir().unwrap(); let selection = selection(); let (home,seat) = pair(root.path(), &selection);
    let command = home.enqueue(draft(&selection,"decision/one")).unwrap(); home.mark_sent(&command).unwrap(); seat.admit(&command).unwrap();
    assert!(seat.enter(&command.execution_id,|_,_| bail!("simulated crash after durable backend fence")).is_err());
    drop(seat);
    let seat = Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    assert!(seat.undispatched().unwrap().is_empty());
    let recovery = seat.recoverable().unwrap(); assert_eq!(recovery.len(),1); assert!(recovery[0].custody_proof.is_none());
    assert!(seat.enter(&command.execution_id,|_,_| panic!("must never redispatch")).is_err());
    let mut next = selection.clone(); next.epoch = 2;
    assert_eq!(seat.install_epoch(&next).unwrap(),EpochInstall::Draining);
    assert_eq!(home.install_epoch(&next).unwrap(),EpochInstall::Draining,"remote sent custody is unresolved at home");
    seat.cancel_or_revoke(&command.execution_id,"epoch replacement".into(),|command,_,reason| {
        assert_eq!(command.execution_id,recovery[0].command.execution_id);
        assert_eq!(reason,"epoch replacement");
        // The injected callback above failed before performing any external operation.
        Ok(())
    }).unwrap();
    seat.complete(&command.execution_id,Outcome::Cancelled {reason:"epoch replacement".into()}).unwrap();
    drain_authority(&home,&seat);
    for receipt in seat.pending_receipts().unwrap() { let through=home.accept_receipt(&receipt).unwrap(); seat.acknowledge_receipts(through).unwrap(); }
    acknowledge_epoch_ready(&home,&seat,selection.epoch);
    assert_eq!(seat.install_epoch(&next).unwrap(),EpochInstall::Installed);
    assert_eq!(home.install_epoch(&next).unwrap(),EpochInstall::Installed);
}

#[test]
fn checkpoint_requires_joint_settlement_and_retains_replay_floors() {
    let root = tempfile::tempdir().unwrap(); let selection = selection(); let (home,seat) = pair(root.path(), &selection);
    let command = home.enqueue(draft(&selection,"decision/one")).unwrap(); home.mark_sent(&command).unwrap(); seat.admit(&command).unwrap();
    assert!(seat.checkpoint("seat",1,1).is_err());
    seat.enter(&command.execution_id,|_,_| Ok("native-proof/example".into())).unwrap();
    let settled=seat.complete(&command.execution_id,Outcome::Observed { incarnation: "native-birth/example".into(), state: "running".into() }).unwrap();
    assert!(seat.checkpoint("seat",1,1).is_err(),"unacknowledged receipts cannot be reclaimed");
    for receipt in seat.pending_receipts().unwrap() { home.accept_receipt(&receipt).unwrap(); }
    seat.acknowledge_receipts(settled.receipt_sequence).unwrap();
    seat.checkpoint("seat",1,1).unwrap(); home.checkpoint("home",1,1).unwrap();
    assert!(home.command_receipt(&command.execution_id).unwrap_err().to_string().contains("result-expired"));
    assert_eq!(seat.admit(&command).unwrap(),Admission::ResultExpired);
    assert!(home.enqueue(draft(&selection,"decision/one")).unwrap_err().to_string().contains("result-expired"));
    assert_eq!(home.accept_receipt(&settled).unwrap(),settled.receipt_sequence,"lost ACK replay survives payload compaction");
    seat.acknowledge_receipts(settled.receipt_sequence).unwrap();
    assert!(seat.checkpoint("seat",1,0).is_err());
    let next = home.enqueue(draft(&selection,"decision/two")).unwrap(); assert_eq!(next.sequence,2);
    assert!(matches!(seat.admit(&next).unwrap(),Admission::Accepted(_)));
    assert!(seat.pending_receipts().unwrap()[0].receipt_sequence > settled.receipt_sequence);
}

#[test]
fn bounded_capacity_refuses_without_losing_accepted_work() {
    let root=tempfile::tempdir().unwrap(); let selection=selection();
    let limits=Limits { max_commands:1,..Limits::default() };
    let home=Journal::open(&root.path().join("home.sqlite3"),limits).unwrap(); home.install_epoch(&selection).unwrap();
    let first=home.enqueue(draft(&selection,"decision/one")).unwrap();
    assert!(home.enqueue(draft(&selection,"decision/two")).unwrap_err().to_string().contains("backpressure"));
    assert_eq!(home.pending_commands().unwrap()[0].execution_id,first.execution_id);
    drop(home);
    let home=Journal::open(&root.path().join("home.sqlite3"),Limits::default()).unwrap();
    assert_eq!(home.pending_commands().unwrap().len(),1);
}

#[test]
fn unknown_journal_versions_and_unversioned_authoritative_bytes_fail_closed() {
    let root=tempfile::tempdir().unwrap(); let path=root.path().join("unknown.sqlite3");
    let connection=Connection::open(&path).unwrap(); connection.execute_batch("CREATE TABLE future(value TEXT); INSERT INTO future VALUES('preserve'); PRAGMA user_version=2;").unwrap(); drop(connection);
    assert!(Journal::open(&path,Limits::default()).is_err());
    let connection=Connection::open(&path).unwrap(); assert_eq!(connection.query_row("SELECT value FROM future",[],|row|row.get::<_,String>(0)).unwrap(),"preserve");
    connection.execute_batch("PRAGMA user_version=0").unwrap(); drop(connection);
    assert!(Journal::open(&path,Limits::default()).is_err());
}

#[test]
fn a_fresh_home_uses_epoch_scoped_receipt_and_authority_watermarks() {
    let root=tempfile::tempdir().unwrap(); let first=selection(); let (home,seat)=pair(root.path(),&first);
    let command=home.enqueue(draft(&first,"decision/old-home")).unwrap(); home.mark_sent(&command).unwrap(); seat.admit(&command).unwrap();
    seat.begin_closing().unwrap(); home.begin_closing().unwrap();
    drain_authority(&home,&seat);
    for receipt in seat.pending_receipts().unwrap() {
        home.accept_receipt(&receipt).unwrap(); seat.acknowledge_receipts(receipt.receipt_sequence).unwrap();
    }
    acknowledge_epoch_ready(&home,&seat,first.epoch);
    let mut next=first.clone(); next.epoch=2; next.home="next-home".into();
    next.home_key=smallclaims::fleet::MemberKey::generate().unwrap().0.public().into();
    assert_eq!(seat.install_epoch(&next).unwrap(),EpochInstall::Installed);
    let next_home=Journal::open(&root.path().join("next-home.sqlite3"),Limits::default()).unwrap();
    assert_eq!(next_home.install_epoch(&next).unwrap(),EpochInstall::Installed);
    let binding=draft(&next,"fixture").binding;
    assert_eq!(seat.offer_execution(&binding).unwrap(),Offer::Admitted);
    assert_eq!(seat.pending_authority().unwrap()[0].sequence,1);
    drain_authority(&next_home,&seat);
    let command=next_home.enqueue(draft(&next,"decision/new-home")).unwrap();
    assert_eq!(command.sequence,1);
    let Admission::Accepted(receipt)=seat.admit(&command).unwrap() else {panic!("new epoch admission")};
    assert_eq!(receipt.receipt_sequence,1);
    assert_eq!(next_home.accept_receipt(&receipt).unwrap(),1);
    assert_eq!(home.epoch_progress(first.epoch).unwrap().0,2);
}

#[test]
fn old_endpoints_are_immutable_and_survive_journal_restart() {
    let root=tempfile::tempdir().unwrap(); let selected=selection();
    let path=root.path().join("seat.sqlite3"); let journal=Journal::open(&path,Limits::default()).unwrap();
    journal.install_epoch(&selected).unwrap();
    let endpoint="ws://127.0.0.1:1234/v1/light-node/session";
    journal.remember_endpoint(&selected,endpoint).unwrap();
    journal.remember_endpoint(&selected,endpoint).unwrap();
    assert!(journal.remember_endpoint(&selected,"ws://127.0.0.1:4321/v1/light-node/session").is_err());
    drop(journal);
    let journal=Journal::open(&path,Limits::default()).unwrap();
    assert_eq!(journal.endpoint(selected.epoch).unwrap().as_deref(),Some(endpoint));
}

#[test]
fn credentials_wait_for_home_admission_and_then_remint_offline() {
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (home,seat)=pair(root.path(),&selected);
    let binding=draft(&selected,"fixture").binding;
    let error=seat.activate_and_issue(&binding,1000,100).unwrap_err();
    assert_eq!(error.downcast_ref::<crate::model::FractalError>().unwrap().code,"local-authority-backpressure");
    assert_eq!(seat.pending_authority().unwrap().len(),1,"wait must retain its durable admission");
    drain_authority(&home,&seat);
    let secret=seat.activate_and_issue(&binding,1000,100).unwrap();
    assert_eq!(seat.with_authority(|connection|Ok(local::local_binding(connection,&secret,101)?)).unwrap(),binding);
    assert!(seat.pending_authority().unwrap().is_empty());
    assert_ne!(seat.activate_and_issue(&binding,1000,102).unwrap(),secret);
    seat.revoke_execution(&binding).unwrap();
    assert!(seat.with_authority(|connection|Ok(local::local_binding(connection,&secret,103)?)).is_err());
}

#[test]
fn start_is_a_typed_unsupported_operation_and_never_reaches_custody() {
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (home,seat)=pair(root.path(),&selected);
    let mut unsupported=draft(&selected,"decision/start");
    unsupported.operation=Operation::Start;
    let error=home.enqueue(unsupported).unwrap_err();
    assert_eq!(error.downcast_ref::<crate::model::FractalError>().unwrap().code,"light-node-start-not-supported");
    assert!(home.pending_commands().unwrap().is_empty());
    let command=Command {version:VERSION,sequence:1,execution_id:"execution/unsupported-start".into(),decision_id:"decision/start".into(),binding:draft(&selected,"fixture").binding,operation:Operation::Start};
    let error=seat.admit(&command).unwrap_err();
    assert_eq!(error.downcast_ref::<crate::model::FractalError>().unwrap().code,"light-node-start-not-supported");
    assert!(seat.pending_receipts().unwrap().is_empty());
    assert!(seat.undispatched().unwrap().is_empty()); assert!(seat.recoverable().unwrap().is_empty());
    assert!(seat.enter(&command.execution_id,|_,_|panic!("unsupported Start cannot enter any native backend")).is_err());
}

#[test]
fn revoked_outbox_command_is_durably_refused_without_blocking_its_successor() {
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (home,seat)=pair(root.path(),&selected);
    let command=home.enqueue(draft(&selected,"decision/stale")).unwrap();
    seat.revoke_execution(&command.binding).unwrap();
    assert!(matches!(seat.admit(&command).unwrap(),Admission::Accepted(_)));
    let terminal=seat.pending_receipts().unwrap().pop().unwrap();
    assert!(matches!(&terminal.state,ReceiptState::Settled {outcome:Outcome::Refused {..}}));
    assert!(seat.undispatched().unwrap().is_empty()); assert!(seat.recoverable().unwrap().is_empty());
    drop(seat);
    let seat=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    assert_eq!(seat.admit(&command).unwrap(),Admission::Duplicate(terminal));
    drain_authority(&home,&seat);
    let mut next=draft(&selected,"decision/successor");
    let binding=&mut next.binding;
    binding.authorization_revision="authorization/successor".into(); binding.incarnation="native-birth/successor".into();
    assert_eq!(seat.offer_execution(binding).unwrap(),Offer::Admitted);
    let next=home.enqueue(next).unwrap(); assert_eq!(next.sequence,2);
    assert!(matches!(seat.admit(&next).unwrap(),Admission::Accepted(_)));
    assert_eq!(seat.undispatched().unwrap().len(),1);
}

#[test]
fn revocation_after_admission_proves_no_native_entry_and_settles() {
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (home,seat)=pair(root.path(),&selected);
    let command=home.enqueue(draft(&selected,"decision/withdrawn")).unwrap();
    seat.admit(&command).unwrap(); seat.revoke_execution(&command.binding).unwrap();
    assert!(seat.enter(&command.execution_id,|_,_|panic!("revoked execution must not enter native backend")).is_err());
    assert!(seat.undispatched().unwrap().is_empty()); assert!(seat.recoverable().unwrap().is_empty());
    assert!(matches!(seat.pending_receipts().unwrap().last().unwrap().state,ReceiptState::Settled {outcome:Outcome::Refused {..}}));
}

#[test]
fn revocation_winning_between_entry_transactions_does_not_invent_unknown_custody() {
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (home,seat)=pair(root.path(),&selected);
    let command=home.enqueue(draft(&selected,"decision/fenced")).unwrap(); seat.admit(&command).unwrap();
    // Reproduce the committed conservative fence, before this live process calls the
    // native adapter. A separate SQLite connection then wins the authority withdrawal.
    seat.lock().unwrap().execute("UPDATE commands SET state='entered' WHERE side='seat' AND execution=?1",[&command.execution_id]).unwrap();
    let other=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    other.revoke_execution(&command.binding).unwrap();
    {
        let mut connection=seat.lock().unwrap();
        assert!(seat.enter_committed(&mut connection,command,|_,_|panic!("authority recheck must precede native callback")).is_err());
    }
    assert!(seat.recoverable().unwrap().is_empty());
    assert!(matches!(seat.pending_receipts().unwrap().last().unwrap().state,ReceiptState::Settled {outcome:Outcome::Refused {..}}));
}

#[tokio::test]
async fn pending_admission_credential_retry_does_not_wake_itself() {
    use futures_util::FutureExt;
    let root=tempfile::tempdir().unwrap(); let selected=selection(); let (_,seat)=pair(root.path(),&selected);
    let binding=draft(&selected,"fixture").binding;
    let wake=seat.authority_changed.notified(); tokio::pin!(wake); wake.as_mut().enable();
    for _ in 0..3 {
        let error=seat.activate_and_issue(&binding,1000,100).unwrap_err();
        assert_eq!(error.downcast_ref::<crate::model::FractalError>().unwrap().code,"local-authority-backpressure");
        assert!(wake.as_mut().now_or_never().is_none(),"offline ACK wait must not create a local retry loop");
    }
    assert_eq!(seat.pending_authority().unwrap().len(),1);
}
