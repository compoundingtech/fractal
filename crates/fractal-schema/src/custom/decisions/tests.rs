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
fn optional_response_selection_does_not_defect_or_block_independent_approval() {
    for kind in ["assumed", "recorded"] {
        let claims = vec![request("r1", 1),
            claim("response", kind, 4, json!({"request":"r1","recorder":"agent/owner",
                "responder":"original spelling","quote":"free text","rationale":"proceed"})),
            claim("r2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
            request("independent", 3),
            claim("approved", "answered", 5, json!({"request":"independent","selection":["yes"]}))];
        let folded = result(&claims);
        assert!(folded.defects.is_empty(), "{kind}: {:?}", folded.defects);
        assert_eq!(folded.requests[1].status, Status::Moot);
        assert!(folded.requests[2].approval_gate);
    }
}

#[test]
fn response_defects_only_disqualify_the_affected_request() {
    for defect in ["fork", "provenance", "duplicate"] {
        let mut claims = vec![request("bad", 1),
            claim("bad-answer", "answered", 4, json!({"request":"bad","selection":["yes"]})),
            request("independent", 2),
            claim("approved", "answered", 5, json!({"request":"independent","selection":["yes"]}))];
        match defect {
            "fork" => {
                for id in ["first", "second"] {
                    claims.push(claim(id, "answered", 6,
                        json!({"request":"bad","selection":["yes"],"supersedes":"bad-answer"})));
                }
            }
            "provenance" => claims.push(claim("invalid", "provenance", 6,
                json!({"target":"bad","written_ms":{"unknown":"not damaged"}}))),
            _ => claims.push(request("duplicate", 1)),
        }
        let folded = result(&claims);
        assert!(!folded.defects.is_empty(), "{defect}");
        assert!(!folded.requests[0].approval_gate, "{defect}");
        assert!(folded.requests[1].approval_gate, "{defect}");
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

#[test]
fn nonstring_guard_scope_and_decision_are_defects_not_fallbacks() {
    for field in ["source_scope", "decision"] {
        for value in [Value::Null, json!(42), json!(false), json!([]), json!({})] {
            let mut guard = json!({"q":1,"selection":["yes"]});
            guard[field] = value;
            let claims = vec![request("r1", 1),
                claim("a1", "answered", 3, json!({"request":"r1","selection":["yes"]})),
                claim("r2", "requested", 2, json!({"q":2,"applies_when":[guard]})),
                claim("a2", "answered", 4, json!({"request":"r2","selection":["yes"]}))];
            let folded = result(&claims);
            assert!(folded.defects.iter().any(|d| d == "invalid guards: r2"), "{field}");
            assert_eq!(folded.requests[1].status, Status::Moot, "{field}");
            assert!(folded.requests[0].approval_gate, "{field}");
            assert!(!folded.requests[1].approval_gate, "{field}");
        }
    }
}

#[test]
fn malformed_guards_are_validated_before_short_circuit_or_dependency_resolution() {
    let malformed = [
        Value::Null,
        json!({"q":1,"selection":"yes"}),
        json!({"q":1,"selection":[42]}),
        json!({"q":1}),
        json!({"q":"1","selection":["yes"]}),
        json!({"selection":["yes"]}),
        json!({"decision":"r1","option":false}),
        json!({"decision":"r1","option":"yes","selection":false}),
    ];
    for guard in malformed {
        for first in [
            json!({"q":1,"selection":["no"]}),
            json!({"q":999,"selection":["yes"]}),
        ] {
            let claims = vec![request("r1", 1),
                claim("a1", "answered", 3, json!({"request":"r1","selection":["yes"]})),
                claim("r2", "requested", 2,
                    json!({"q":2,"applies_when":[first,guard.clone()]}))];
            let folded = result(&claims);
            assert!(folded.defects.iter().any(|d| d == "invalid guards: r2"), "{guard}");
            assert_eq!(folded.requests[1].status, Status::Moot);
            assert!(folded.requests[0].approval_gate);
            assert!(!folded.requests[1].approval_gate);
        }
    }
    // Even the first guard's malformed selection is detected with no answer to read.
    let claims = vec![request("r1", 1), claim("r2", "requested", 2,
        json!({"q":2,"applies_when":[{"q":1,"selection":false}]}))];
    assert!(result(&claims).defects.iter().any(|d| d == "invalid guards: r2"));
}

#[test]
fn deep_guard_dependency_chain_uses_no_recursive_call_stack() {
    const DEPTH: usize = 4096;
    let mut claims = Vec::new();
    // Forward dependencies force one deep traversal rather than cached shallow reads.
    for index in 0..DEPTH {
        let id = format!("r{index}");
        let fields = if index + 1 == DEPTH {
            json!({"q":index})
        } else {
            json!({"q":index,"applies_when":[{"decision":format!("r{}", index + 1),"selection":["yes"]}]})
        };
        claims.push(claim(&id, "requested", index as u64, fields));
        claims.push(claim(&format!("a{index}"), "answered", (DEPTH + index) as u64,
            json!({"request":id,"selection":["yes"]})));
    }
    let folded = result(&claims);
    assert!(folded.defects.is_empty(), "{:?}", folded.defects);
    assert_eq!(folded.requests.len(), DEPTH);
    assert!(folded.requests.iter().all(|r| r.status == Status::Answered && r.approval_gate));
}

#[test]
fn guard_cycles_remain_moot_and_never_approve() {
    let claims = vec![
        claim("r1", "requested", 1, json!({"q":1,"applies_when":[{"q":2,"selection":["yes"]}]})),
        claim("r2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
        claim("a1", "answered", 3, json!({"request":"r1","selection":["yes"]})),
        claim("a2", "answered", 4, json!({"request":"r2","selection":["yes"]})),
    ];
    let folded = result(&claims);
    assert!(folded.defects.iter().any(|d| d.contains("guard cycle")));
    assert!(folded.requests.iter().all(|r| r.status == Status::Moot && !r.approval_gate));
}

#[test]
fn long_supersession_chains_select_the_head_for_answers_and_assumptions() {
    const LENGTH: usize = 8192;
    for kind in ["answered", "assumed"] {
        let mut claims = vec![request("r", 1)];
        // The oldest candidate is visited first; without memoization every later
        // candidate repeats the entire growing predecessor walk.
        for index in 0..LENGTH {
            let mut fields = json!({"request":"r","selection":["yes"]});
            if index > 0 { fields["supersedes"] = json!(format!("a{}", index - 1)); }
            claims.push(claim(&format!("a{index}"), kind, index as u64 + 2, fields));
        }
        let folded = result(&claims);
        assert!(folded.defects.is_empty(), "{:?}", folded.defects);
        if kind == "answered" {
            let head = format!("a{}", LENGTH - 1);
            assert_eq!(folded.requests[0].answer, Some(head.as_str()));
            assert!(folded.requests[0].approval_gate);
        } else {
            assert!(folded.requests[0].assumed);
            assert!(folded.requests[0].open);
        }
    }
}

#[test]
fn indexed_supersession_retains_cycle_fork_and_missing_predecessor_defects() {
    let claims = vec![request("r", 1),
        claim("a1", "answered", 2, json!({"request":"r","supersedes":"a2"})),
        claim("a2", "answered", 3, json!({"request":"r","supersedes":"a1"})),
        claim("fork", "answered", 4, json!({"request":"r","supersedes":"a1"})),
        claim("missing", "answered", 5, json!({"request":"r","supersedes":"absent"}))];
    let folded = result(&claims);
    for defect in ["cycle", "fork", "missing answer predecessor"] {
        assert!(folded.defects.iter().any(|d| d.contains(defect)), "{defect}");
    }
    assert!(!folded.requests[0].approval_gate);
}

#[test]
fn defective_answers_cannot_activate_direct_or_transitive_guard_approvals() {
    let mut claims = vec![request("q1", 1),
        claim("a1", "answered", 5, json!({"request":"q1","selection":["no"]})),
        claim("fork-no", "answered", 6,
            json!({"request":"q1","selection":["no"],"supersedes":"a1"})),
        claim("fork-yes", "answered", 7,
            json!({"request":"q1","selection":["yes"],"supersedes":"a1"})),
        claim("q2", "requested", 2, json!({"q":2,"applies_when":[{"q":1,"selection":["yes"]}]})),
        claim("a2", "answered", 8, json!({"request":"q2","selection":["yes"]})),
        request("q3", 3),
        claim("a3", "answered", 9, json!({"request":"q3","selection":["yes"]})),
        claim("q4", "requested", 4, json!({"q":4,"applies_when":[{"q":2,"selection":["yes"]}]})),
        claim("a4", "answered", 10, json!({"request":"q4","selection":["yes"]}))];
    for reverse in [false, true] {
        if reverse {
            claims.reverse();
            // A prior non-match must not hide the later defective dependency's cause.
            claims.iter_mut().find(|claim| claim.id == "q4").unwrap().fields["applies_when"]
                .as_array_mut().unwrap().insert(0, json!({"q":3,"selection":["no"]}));
        }
        let folded = result(&claims);
        for id in ["q1", "q2", "q4"] {
            let request = folded.requests.iter().find(|request| request.request == id).unwrap();
            assert!(!request.approval_gate, "{id}");
            if id != "q1" { assert_eq!(request.status, Status::Moot, "{id}"); }
        }
        assert!(folded.requests.iter().find(|request| request.request == "q3").unwrap().approval_gate);
        for cause in ["defective guard dependency: q2 <- q1", "defective guard dependency: q4 <- q2"] {
            assert!(folded.defects.iter().any(|defect| defect == cause), "{cause}");
        }
    }
}
