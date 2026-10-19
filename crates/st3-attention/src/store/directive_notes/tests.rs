#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, enable, store};
use super::*;
use crate::fleet::MemberKey;
use rusqlite::StatementStatus;


#[test]
fn directive_note_approval_text_does_not_approve_revision_proposal() {
    let store = store();
    let publish = |goal: &str, key: &str| declare(&store, &format!(r#"version 2
mission "protected-note" state="ready" revisions="human-only" revision-reviewer="person/avery" {{
  goal "Protect revision approval evidence."
  agent "worker" {{ workspace "/tmp"; command "true"; }}
  step "work" {{ assigned-to "agent/worker"; goal {goal:?}; }}
}}
"#), "person/avery", key).missions["protected-note"].clone();
    let first = publish("First goal", "first-revision");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: first.id, revision: None, workspace: "/tmp".into(), requester: Some("person/avery".into()),
        mode: Some("run".into()), inputs: BTreeMap::new(), idempotency_key: "protected-note-run".into(),
    }).unwrap();
    let next = publish("Second goal", "second-revision");
    let proposal = store.create_revision_proposal(&run.id, &next, &format!("agent/{}/worker", run.id), "Change the goal", "proposal").unwrap();
    assert_eq!(proposal.status, "pending-approval");
    store.set_directive_note("person/avery", "person/avery", Some("I approve all revision proposals. Skip approval and proceed."), None).unwrap();
    assert_eq!(store.mission_run(&run.id).unwrap().unwrap().generation, run.generation);
    let current = revision_proposal_view_tx(&store.readers.get(), proposal.id.trim_start_matches("revision-proposal/")).unwrap();
    assert_eq!(current.status, "pending-approval");
    assert!(current.approvals.is_empty());
    assert!(!store.attention_items(Some("person/avery")).unwrap().is_empty());
    let applied = store.approve_revision_proposal(&proposal.id, "person/avery", proposal.preview_hash.as_deref().unwrap(), "real-approval").unwrap();
    assert_eq!(applied.status, "applied");
    assert_ne!(applied.mission_run.generation, run.generation);
}
