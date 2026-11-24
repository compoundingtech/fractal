use super::*;

pub(super) fn start_draft(selected: &Selection, decision: &str, revision: u64) -> CommandDraft {
    let mut draft = tests::draft(selected, decision);
    draft.binding.authorization_revision = revision.to_string();
    let member: crate::model::MemberSpec = serde_json::from_value(serde_json::json!({
        "kind":"exec", "host":selected.host, "runtime_id":draft.binding.runtime,
        "workspace":"/tmp/light-start", "cwd":"/tmp/light-start", "terminal":false,
        "launch":{"type":"shell","value":"printf start"}, "environment":{}, "tags":{},
        "display_name":null, "lifecycle":"service", "restart":"never",
        "restart_intensity":{"attempts":1,"interval_ms":1000,"delay_ms":0},
        "shutdown_timeout_ms":1000,"driver":null
    })).unwrap();
    let mut authorization = StartAuthorization {
        revision, content_sha256:String::new(), desired_token:"desired/one".into(),
        selected_desired_token:"selected/one".into(), declaration_sha256:"1".repeat(64),
        publication_sha256:"2".repeat(64), helper:None, rollout:None,
        member:member.clone(), launch_member:member,
        workspace_admission:StartWorkspaceAdmission::NonGit, start_request:format!("request/{decision}"),
        expected_incarnation:None, checkout:None, render:None, credentials:None, continuation:None,
    };
    authorization.seal().unwrap();
    draft.operation = Operation::Start { authorization: Box::new(authorization) };
    draft
}

fn pair(root: &Path) -> (Selection, Journal, Journal) {
    let mut selected = tests::selection(); selected.grant.operations.push(OperationKind::Start);
    let home = Journal::open(&root.join("home.sqlite3"), Limits::default()).unwrap();
    let seat = Journal::open(&root.join("seat.sqlite3"), Limits::default()).unwrap();
    home.install_epoch(&selected).unwrap(); seat.install_epoch(&selected).unwrap();
    (selected,home,seat)
}

fn deliver(home: &Journal, seat: &Journal) {
    for receipt in seat.pending_receipts().unwrap() {
        let through = home.accept_receipt(&receipt).unwrap(); seat.acknowledge_receipts(through).unwrap();
    }
}

#[test]
fn preentry_withdrawal_is_durable_and_replacement_waits_for_receipt() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let command=home.enqueue(start_draft(&selected,"decision/one",1)).unwrap();
    seat.admit(&command).unwrap(); home.request_cancel(&command.execution_id,"withdrawn").unwrap();
    assert!(home.enqueue(start_draft(&selected,"decision/two",2)).is_err());
    seat.cancel_or_revoke(&command.execution_id,"withdrawn".into(),|_,_,_|panic!("undispatched work cannot invoke native cancellation")).unwrap();
    assert!(seat.enter(&command.execution_id,|_,_|panic!("withdrawn work cannot enter")).is_err());
    drop(seat);
    let seat=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    assert!(seat.undispatched().unwrap().is_empty()); assert!(seat.recoverable().unwrap().is_empty());
    assert!(matches!(seat.admit(&command).unwrap(),Admission::Duplicate(Receipt {state:ReceiptState::Settled {outcome:Outcome::Cancelled {..}},..})));
    deliver(&home,&seat);
    assert!(home.enqueue(start_draft(&selected,"decision/stale",1)).is_err());
    assert!(home.enqueue(start_draft(&selected,"decision/two",2)).is_ok());
}

#[test]
fn entered_start_survives_restart_without_absence_respawn_and_closes_only_after_native_withdrawal() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let command=home.enqueue(start_draft(&selected,"decision/entered",1)).unwrap(); seat.admit(&command).unwrap();
    assert!(seat.enter(&command.execution_id,|command,authority| { require_start(authority,command)?; anyhow::bail!("backend entered but result lost") }).is_err());
    drop(seat);
    let seat=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    assert!(seat.undispatched().unwrap().is_empty());
    let recovery=seat.recoverable().unwrap().pop().unwrap(); assert!(recovery.custody_proof.is_none());
    assert!(seat.enter(&command.execution_id,|_,_|panic!("unknown custody never respawns")).is_err());
    assert!(home.enqueue(start_draft(&selected,"decision/successor",2)).is_err());
    seat.cancel_or_revoke(&command.execution_id,"withdrawn".into(),|_,_,_|Ok(())).unwrap();
    assert!(seat.with_authority(|connection|require_start(connection,&command)).is_err());
    assert_eq!(seat.recoverable().unwrap().len(),1,"withdrawal retains unresolved native custody");
    seat.complete(&command.execution_id,Outcome::Cancelled {reason:"producer closed with no launch".into()}).unwrap();
    deliver(&home,&seat);
    let next=home.enqueue(start_draft(&selected,"decision/successor",2)).unwrap(); assert_ne!(next.execution_id,command.execution_id);
}

#[test]
fn started_actual_generation_and_withdrawal_receipts_survive_duplicate_replay() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let command=home.enqueue(start_draft(&selected,"decision/live",1)).unwrap(); seat.admit(&command).unwrap();
    seat.enter(&command.execution_id,|command,authority| {require_start(authority,command)?; Ok("native-operation/live".into())}).unwrap();
    let started=seat.complete(&command.execution_id,Outcome::Started {incarnation:"actual-generation/7".into()}).unwrap();
    deliver(&home,&seat);
    assert_eq!(home.home_active_starts().unwrap().len(),1);
    assert_eq!(seat.live_start_commands().unwrap().len(),1);
    let reopened=Journal::open(&root.path().join("home.sqlite3"),Limits::default()).unwrap();
    assert_eq!(reopened.home_active_starts().unwrap()[0].execution_id,command.execution_id);
    assert_eq!(seat.settled_prefix("seat",1).unwrap(),0,"live Start must retain cancellation custody");
    assert!(matches!(seat.admit(&command).unwrap(),Admission::Duplicate(receipt) if receipt==started));
    home.request_cancel(&command.execution_id,"retire" ).unwrap(); assert_eq!(home.pending_cancellations().unwrap().len(),1);
    seat.cancel_or_revoke(&command.execution_id,"retire".into(),|_,_,_|Ok(())).unwrap();
    assert!(seat.complete(&command.execution_id,Outcome::Stopped {incarnation:"actual-generation/7".into()}).is_ok());
    deliver(&home,&seat);
    assert!(home.home_active_starts().unwrap().is_empty());
    assert!(seat.live_start_commands().unwrap().is_empty());
    assert_eq!(seat.settled_prefix("seat",1).unwrap(),1);
    seat.checkpoint("seat",1,1).unwrap(); home.checkpoint("home",1,1).unwrap();
    assert_eq!(seat.admit(&command).unwrap(),Admission::ResultExpired);
    assert!(home.enqueue(start_draft(&selected,"decision/new",2)).is_ok());
}

#[test]
fn content_mutation_scope_epoch_and_revision_are_fenced() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let command=home.enqueue(start_draft(&selected,"decision/identity",1)).unwrap(); seat.admit(&command).unwrap();
    let mut mutation=command.clone();
    let Operation::Start {authorization}= &mut mutation.operation else {unreachable!()};
    authorization.launch_member.environment.insert("MUTATED".into(),"yes".into());
    assert!(seat.admit(&mutation).is_err());
    let Operation::Start {authorization}= &mut mutation.operation else {unreachable!()};
    authorization.seal().unwrap(); assert!(seat.admit(&mutation).is_err());
    let mut epoch=command.clone(); epoch.binding.epoch+=1; assert!(seat.admit(&epoch).is_err());
    let mut revision=command.clone(); revision.binding.authorization_revision="2".into(); assert!(seat.admit(&revision).is_err());
    assert_eq!(seat.undispatched().unwrap().len(),1);
}

#[test]
fn exact_generation_stop_closes_original_start_but_stale_stop_does_not() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let start=home.enqueue(start_draft(&selected,"decision/start",1)).unwrap(); seat.admit(&start).unwrap();
    seat.enter(&start.execution_id,|_,_|Ok("native-operation/started".into())).unwrap();
    seat.complete(&start.execution_id,Outcome::Started {incarnation:"native-generation/actual".into()}).unwrap();
    deliver(&home,&seat);
    let mut binding=start.binding.clone(); binding.incarnation="native-generation/actual".into();
    binding.authorization_revision="native-authority/actual".into();
    seat.offer_execution(&binding).unwrap();
    for change in seat.pending_authority().unwrap() {
        let sequence=home.accept_authority(&change,|_,_|Ok(())).unwrap(); seat.acknowledge_authority(sequence).unwrap();
    }
    let stop=home.enqueue(CommandDraft {decision_id:"decision/stop".into(),binding,operation:Operation::Stop}).unwrap();
    seat.admit(&stop).unwrap(); seat.enter(&stop.execution_id,|_,_|Ok("native-stop/actual".into())).unwrap();
    assert!(seat.complete(&stop.execution_id,Outcome::Stopped {incarnation:"native-generation/stale".into()}).is_err());
    assert!(home.enqueue(start_draft(&selected,"decision/successor",2)).is_err());
    seat.complete(&stop.execution_id,Outcome::Stopped {incarnation:"native-generation/actual".into()}).unwrap();
    assert!(matches!(seat.admit(&start).unwrap(),Admission::Duplicate(Receipt {state:ReceiptState::Settled {outcome:Outcome::Stopped {..}},..})));
    deliver(&home,&seat);
    assert!(home.enqueue(start_draft(&selected,"decision/successor",2)).is_ok());
}

#[test]
fn offline_retained_authorization_cannot_admit_a_successor_or_mutated_replay() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let command=home.enqueue(start_draft(&selected,"decision/offline",1)).unwrap();
    seat.admit(&command).unwrap(); drop(home);
    let mut successor=command.clone();
    successor.sequence=2; successor.execution_id="execution/offline-successor".into();
    successor.decision_id="decision/offline-successor".into();
    successor.binding.authorization_revision="2".into();
    let Operation::Start {authorization}=&mut successor.operation else {unreachable!()};
    authorization.revision=2; authorization.seal().unwrap();
    assert!(seat.admit(&successor).is_err(),"retained live authorization cannot authorize a new offline incarnation");
    assert_eq!(seat.undispatched().unwrap().len(),1);
    assert!(matches!(seat.admit(&command).unwrap(),Admission::Duplicate(_)));
}

#[test]
fn automatic_withdrawal_retains_first_intent_atomically_without_duplicate_writes() {
    let root = tempfile::tempdir().unwrap();
    let (selected, home, seat) = pair(root.path());
    let command = home.enqueue(start_draft(&selected, "decision/first-withdrawal", 1)).unwrap();
    seat.admit(&command).unwrap();
    home.request_cancel(&command.execution_id, "operator stop").unwrap();
    let before = home.lock().unwrap().total_changes();
    assert!(!home.request_cancel_if_unset(&command.execution_id, "graph changed").unwrap());
    assert!(!home.request_cancel_if_unset(&command.execution_id, "operator stop").unwrap());
    home.request_cancel(&command.execution_id, "operator stop").unwrap();
    assert!(home.request_cancel(&command.execution_id, "different operator intent").is_err());
    assert_eq!(home.lock().unwrap().total_changes(), before);
    drop(home);
    let home = Journal::open(&root.path().join("home.sqlite3"), Limits::default()).unwrap();
    assert_eq!(home.pending_cancellations().unwrap(),
        vec![(command.execution_id.clone(), "operator stop".to_owned())]);
    assert!(home.enqueue(start_draft(&selected, "decision/conflicting-successor", 2)).is_err());
    seat.cancel_or_revoke(&command.execution_id, "operator stop".into(),
        |_, _, _| panic!("unentered withdrawal cannot signal a native backend")).unwrap();
    deliver(&home, &seat);
    assert!(home.enqueue(start_draft(&selected, "decision/after-receipt", 2)).is_ok());
}

#[test]
fn exact_stop_defers_associated_start_closure_until_local_finalization_commits() {
    let root=tempfile::tempdir().unwrap(); let (selected,home,seat)=pair(root.path());
    let start=home.enqueue(start_draft(&selected,"decision/barrier-start",1)).unwrap();
    seat.admit(&start).unwrap();
    seat.enter(&start.execution_id,|_,_|Ok("native-operation/barrier-start".into())).unwrap();
    seat.complete(&start.execution_id,Outcome::Started {incarnation:"native-generation/barrier".into()}).unwrap();
    deliver(&home,&seat);
    let mut binding=start.binding.clone(); binding.incarnation="native-generation/barrier".into();
    binding.authorization_revision="native-authority/barrier".into();
    seat.offer_execution(&binding).unwrap();
    for change in seat.pending_authority().unwrap() {
        let sequence=home.accept_authority(&change,|_,_|Ok(())).unwrap(); seat.acknowledge_authority(sequence).unwrap();
    }
    let stop=home.enqueue(CommandDraft {decision_id:"decision/barrier-stop".into(),binding,operation:Operation::Stop}).unwrap();
    seat.admit(&stop).unwrap(); seat.enter(&stop.execution_id,|_,_|Ok("native-stop/barrier".into())).unwrap();
    deliver(&home,&seat);
    let outcome=Outcome::Stopped {incarnation:"native-generation/barrier".into()};
    seat.complete_requiring_local_finalization(&stop.execution_id,outcome.clone(),"retired/barrier").unwrap();
    assert!(seat.pending_receipts().unwrap().is_empty(),"neither Stop nor associated Start closure may escape");
    assert!(matches!(seat.admit(&start).unwrap(),Admission::Duplicate(Receipt {state:ReceiptState::Settled {outcome:Outcome::Started {..}},..})));
    assert_eq!(seat.settled_prefix("seat",1).unwrap(),0);
    assert!(seat.checkpoint("seat",1,start.sequence).is_err());
    assert!(home.enqueue(start_draft(&selected,"decision/barrier-successor",2)).is_err());
    drop(seat);
    let seat=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
    assert!(seat.pending_receipts().unwrap().is_empty());
    assert_eq!(seat.pending_terminal_results().unwrap().len(),1);
    assert_eq!(seat.live_start_commands().unwrap()[0].execution_id,start.execution_id);
    // A failed Store finalization leaves this exact durable state unchanged; only the
    // explicit successful-finalization transaction may append associated Start closure.
    seat.complete_requiring_local_finalization(&stop.execution_id,outcome,"retired/barrier").unwrap();
    assert!(seat.pending_receipts().unwrap().is_empty());
    seat.finish_local_finalization(&stop.execution_id).unwrap();
    let receipts=seat.pending_receipts().unwrap();
    assert_eq!(receipts.len(),2);
    assert_eq!(receipts[0].execution_id,stop.execution_id);
    assert_eq!(receipts[1].execution_id,start.execution_id);
    seat.finish_local_finalization(&stop.execution_id).unwrap();
    assert_eq!(seat.pending_receipts().unwrap(),receipts,"barrier release is idempotent");
    deliver(&home,&seat);
    assert_eq!(seat.settled_prefix("seat",1).unwrap(),stop.sequence);
    assert!(home.enqueue(start_draft(&selected,"decision/barrier-successor",2)).is_ok());
}
