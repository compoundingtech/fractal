use super::*;
use serde_json::json;

fn claim(id: &str, kind: &str, order: u64, fields: Value) -> Claim {
    Claim { id: id.into(), kind: format!("custom.decision.tree.v2.{kind}"),
        actor: Some(if kind == "answered" { "person/lichen" } else { "agent/owner" }.into()), order, fields }
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
        if imported { claims.push(claim("p","provenance",3,json!({"target":"a","written_ms":{"known":1},"legacy_id":"a","source_scope":"s"}))); }
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
            json!({"target":target,"written_ms":{"known":written},"legacy_id":legacy,"source_scope":scope})));
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

#[test]
fn v1_legacy_guard_shape() {
    for (option, open) in [("yes", true), ("no", false)] {
        let mut claims = vec![request("r1", 1),
            claim("a", "answered", 2, json!({"request":"r1","selection":["yes"]})),
            claim("r2", "requested", 3, json!({"q":2,"applies_when":[{"decision":"r1","option":option}]}))];
        for claim in &mut claims { claim.kind = claim.kind.replace(".v2.", ".v1."); }
        let folded = result(&claims);
        assert!(folded.defects.is_empty());
        assert_eq!(folded.requests[1].open, open);
    }
}

#[test]
fn provenance_ties_use_legacy_id_then_scope() {
    for (legacy, scope, expected) in [("b", "one", "a2"), ("a", "two", "a2"), ("a", "aaa", "a1")] {
        let claims = vec![request("r", 1),
            claim("a1", "answered", 3, json!({"request":"r","selection":[]})),
            claim("a2", "answered", 2, json!({"request":"r","selection":[]})),
            claim("p1", "provenance", 4, json!({"target":"a1","written_ms":{"known":100},"legacy_id":"a","source_scope":"one"})),
            claim("p2", "provenance", 5, json!({"target":"a2","written_ms":{"known":100},"legacy_id":legacy,"source_scope":scope}))];
        assert_eq!(result(&claims).requests[0].answer, Some(expected));
    }
}

#[test]
fn unknown_time_is_only_valid_for_damaged_provenance() {
    for (kind, valid) in [("damaged", true), ("requested", false), ("recorded", false)] {
        let claims = vec![claim("target", kind, 1, json!({})),
            claim("p", "provenance", 2, json!({"target":"target","written_ms":{"unknown":"unreadable source timestamp"}}))];
        assert_eq!(result(&claims).defects.is_empty(), valid);
    }
    for time in [json!(123), json!({"known":"123"}), json!({"unknown":""}),
        json!({"known":1,"unknown":"ambiguous"})] {
        let claims = vec![claim("target", "damaged", 1, json!({})),
            claim("p", "provenance", 2, json!({"target":"target","written_ms":time}))];
        assert!(!result(&claims).defects.is_empty());
    }
}

#[test]
fn damaged_unknown_time_never_changes_answer_order_or_native_tie_break() {
    let mut claims = vec![request("r", 1),
        claim("a1", "recorded", 2, json!({"request":"r","recorder":"agent/owner"})),
        claim("a2", "recorded", 3, json!({"request":"r","recorder":"agent/owner"}))];
    for target in ["a1", "a2"] {
        claims.push(claim(&format!("p{target}"), "provenance", 4,
            json!({"target":target,"written_ms":{"known":100},"legacy_id":"same","source_scope":"one"})));
    }
    assert_eq!(result(&claims).requests[0].answer, Some("a2"));
    claims.push(claim("damaged", "damaged", 100, json!({})));
    claims.push(claim("pd", "provenance", 101,
        json!({"target":"damaged","written_ms":{"unknown":"not present in source"}})));
    let folded = result(&claims);
    assert!(folded.defects.is_empty());
    assert_eq!(folded.requests[0].answer, Some("a2"));
}

#[test]
fn supersession_head_wins_over_reverse_lexical_provenance_tie() {
    let claims = vec![request("r1", 1),
        claim("r2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
        claim("a1", "recorded", 3, json!({"request":"r1","recorder":"agent/owner","selection":["no"]})),
        claim("a2", "recorded", 4, json!({"request":"r1","recorder":"agent/owner","selection":["yes"],"supersedes":"a1"})),
        claim("p1", "provenance", 5, json!({"target":"a1","written_ms":{"known":100},"legacy_id":"z","source_scope":"same"})),
        claim("p2", "provenance", 6, json!({"target":"a2","written_ms":{"known":100},"legacy_id":"a","source_scope":"same"}))];
    let folded = result(&claims);
    assert!(folded.defects.is_empty());
    assert_eq!(folded.requests[0].answer, Some("a2"));
    assert!(folded.requests[1].open);
}

#[test]
fn imported_assumptions_use_known_source_chronology() {
    let mut claims = vec![request("r1", 1),
        claim("r2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
        claim("a1", "assumed", 3, json!({"request":"r1","selection":["yes"]})),
        claim("a2", "assumed", 4, json!({"request":"r1","selection":["no"]})),
        claim("p1", "provenance", 5, json!({"target":"a1","written_ms":{"known":200},"legacy_id":"a","source_scope":"same"})),
        claim("p2", "provenance", 6, json!({"target":"a2","written_ms":{"known":100},"legacy_id":"b","source_scope":"same"}))];
    for reverse in [false, true] {
        if reverse { claims.reverse(); }
        let folded = result(&claims);
        assert!(folded.defects.is_empty());
        let assumed = folded.requests.iter().find(|r| r.request == "r1").unwrap();
        let dependent = folded.requests.iter().find(|r| r.request == "r2").unwrap();
        assert!(assumed.assumed && assumed.open && dependent.open);
        assert!(!assumed.approval_gate && !dependent.approval_gate);
    }
}

#[test]
fn legacy_unproven_trees_report_open_requests_without_proposing_asks() {
    let claims = vec![request("r", 1)];
    for (subject, needs_ask) in [("custom/decision/tree/v2/legacy/original", false),
        ("custom/decision/tree/v2/example", true)] {
        let folded = fold(subject, &claims, json!({"revision":"fenced"}));
        assert!(folded.requests[0].open);
        assert_eq!(folded.requests[0].needs_ask, needs_ask);
    }
}

#[test]
fn duplicate_target_cannot_hide_invalid_provenance_time() {
    let mut claims = vec![request("r", 1),
        claim("a", "answered", 2, json!({"request":"r","selection":["yes"]})),
        claim("invalid", "provenance", 3, json!({
            "target":"a","written_ms":{"unknown":"not a damaged target"},
            "legacy_id":"a","source_scope":"same"})),
        claim("valid", "provenance", 4, json!({
            "target":"a","written_ms":{"known":100},"legacy_id":"a","source_scope":"same"}))];
    for reverse in [false, true] {
        if reverse { claims.reverse(); }
        let folded = result(&claims);
        assert!(folded.defects.iter().any(|d| d.contains("invalid provenance time: invalid")));
        assert!(!folded.requests[0].approval_gate);
    }
}
