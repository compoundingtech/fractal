use super::*;
use serde_json::json;

fn claim(id: &str, kind: &str, order: u64, fields: Value) -> Claim {
    Claim { id: id.into(), kind: format!("custom.decision.tree.v2.{kind}"),
        actor: Some(if kind == "answered" { "person/alice" } else { "agent/owner" }.into()), order, fields }
}
fn request(id: &str, q: i64) -> Claim { claim(id, "requested", q.unsigned_abs(), json!({"q":q})) }
fn result(claims: &[Claim]) -> Fold<'_> { fold("custom/decision/tree/v2/owner", claims, json!({"revision":"fenced"})) }

#[test]
fn guard_on_off_and_assumptions_stay_open() {
    for (selection, expected) in [("yes", Status::Pending), ("no", Status::Moot)] {
        let claims = vec![request("r1", 1), claim("a1", "assumed", 3,
            json!({"request":"r1","selection":[selection]})),
            claim("r2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]}))];
        let folded = result(&claims);
        assert!(folded.defects.is_empty());
        assert!(folded.requests[0].assumed && folded.requests[0].open);
        assert_eq!(folded.requests[1].status, expected);
    }
}
#[test]
fn parent_linked_reframe_withdrawn_and_ask_links() {
    let claims = vec![request("r1",1), claim("r2","requested",2,json!({"q":2,"parent":"r1"})),
        claim("w2","withdrawn",3,json!({"request":"r2","reason":"duplicate"})),
        claim("link","ask_linked",4,json!({"request":"r1","ask":"step-run/ask","activation":"one"}))];
    let folded = result(&claims);
    assert_eq!(folded.requests[0].status, Status::Superseded);
    assert_eq!(folded.requests[1].status, Status::Withdrawn);
    assert_eq!(folded.requests[0].ask_links.len(),1);
    assert!(!folded.requests.iter().any(|r| r.open || r.needs_ask));
}
#[test]
fn supersession_revives_selection_and_reports_forks() {
    let mut claims = vec![request("r1",1),
        claim("r2","requested",2,json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
        claim("a1","answered",3,json!({"request":"r1","selection":["yes"]})),
        claim("a2","answered",4,json!({"request":"r1","selection":["no"],"supersedes":"a1"}))];
    assert_eq!(result(&claims).requests[1].status, Status::Moot);
    claims.push(claim("a3","answered",5,json!({"request":"r1","selection":["yes"],"supersedes":"a2"})));
    let folded = result(&claims);
    assert_eq!(folded.requests[0].answer,Some("a3"));
    assert!(folded.requests[1].open);
    assert!(folded.defects.is_empty());
    claims.push(claim("fork","answered",6,json!({"request":"r1","selection":[],"supersedes":"a2"})));
    assert!(result(&claims).defects.iter().any(|d| d.contains("fork")));
}
#[test]
fn recorded_and_imported_answers_never_approve() {
    for (kind, imported, expected) in [("answered",false,true),("answered",true,false),("recorded",false,false)] {
        let mut claims = vec![request("r",1), claim("a",kind,2,
            json!({"request":"r","recorder":"agent/owner","responder":"Alice Original","quote":"yes","selection":["yes"]}))];
        if imported { claims.push(claim("p","provenance",3,json!({"target":"a","written_ms":1,"legacy_id":"a","source_scope":"s"}))); }
        let folded = result(&claims);
        assert_eq!(folded.requests[0].status,Status::Answered);
        assert_eq!(folded.requests[0].approval_gate,expected);
        assert!(!folded.requests[0].open);
    }
}
#[test]
fn original_provenance_orders_answers_and_scopes_keys() {
    let mut claims = vec![request("r1",1),request("r2",1),
        claim("a1","recorded",10,json!({"request":"r1","recorder":"agent/owner"})),
        claim("a2","recorded",20,json!({"request":"r1","recorder":"agent/owner"}))];
    for (target, written, legacy, scope) in [("r1",1,"r","one"),("r2",1,"r","two"),
        ("a1",200,"a","one"),("a2",100,"b","one")] {
        claims.push(claim(&format!("p{target}"),"provenance",30,
            json!({"target":target,"written_ms":written,"legacy_id":legacy,"source_scope":scope})));
    }
    let folded=result(&claims);
    assert!(folded.defects.is_empty());
    assert_eq!(folded.requests[0].answer,Some("a1"));
    assert_eq!(folded.requests[1].source_scope,"two");
}
#[test]
fn v1_only_and_native_order() {
    let mut claims = vec![request("r",1),claim("a","answered",2,json!({"request":"r","selection":["yes"]}))];
    for c in &mut claims { c.kind=c.kind.replace(".v2.",".v1."); }
    let folded = fold("custom/decision/tree/v1/owner",&claims,json!({"revision":"v1"}));
    assert!(folded.defects.is_empty());
    assert_eq!(folded.requests[0].status,Status::Answered);
    assert!(folded.requests[0].approval_gate);
    assert_eq!(folded.basis["revision"],"v1");
    assert_eq!(input_kinds(1).len(),6);
}
