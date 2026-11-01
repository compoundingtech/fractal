use super::*;
use crate::task_grants::parse_task_manifest_v1;
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({
        "version":1,"title":"Rotate provider token",
        "requester":{"principal":"agent/requester","run":"run","step":"step","incarnation":"incarnation"},
        "window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},
        "provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},
        "browserSteps":[
            {"index":2,"op":"navigate","literal":"https://example.com/settings"},
            {"index":9,"op":"revealOnce","locator":{"kind":"label","value":"Token"},"trigger":{"locator":{"kind":"role","role":"button","name":"Create token"}},"issues":"token"}
        ],
        "writes":[{"vaultId":"vault","itemId":"item","fields":[{"inventoryId":"token","kind":"secret"}]}],
        "readers":[]
    })
}

fn check(value: Value) -> Result<TaskManifestV1, TaskManifestError> {
    parse_task_manifest_v1(&serde_json::to_vec(&value).unwrap())
}

fn refuse(value: Value) { assert!(check(value.clone()).is_err(), "unexpectedly accepted {value}"); }

#[test]
fn zero_reader_rotation_pilot_is_valid_without_rewriting() {
    let value = fixture();
    let manifest = check(value.clone()).unwrap();
    assert!(manifest.readers.is_empty());
    assert_eq!(serde_json::to_value(manifest).unwrap(), value);
}

#[test]
fn exact_principal_and_text_rules() {
    for principal in ["person/a", "agent/a/b-c_d.e", "task/a", "service/op-proxy"] {
        let mut value = fixture(); value["requester"]["principal"] = json!(principal);
        check(value).unwrap();
    }
    for principal in ["agent", "agent/", "agent//a", "agent/.", "agent/..", "agent/**", "program/a", "host/a", "agent/é"] {
        let mut value = fixture(); value["requester"]["principal"] = json!(principal); refuse(value);
    }
    for pointer in ["/title", "/requester/run", "/requester/step", "/requester/incarnation", "/provider/name", "/provider/browserProfile"] {
        for bad in [" \t", "a\nb", "a\u{7f}b"] {
            let mut value = fixture(); *value.pointer_mut(pointer).unwrap() = json!(bad); refuse(value);
        }
    }
}

#[test]
fn utc_window_bounds_preserve_approved_spelling() {
    for (before, after) in [
        ("2026-10-20T12:00:00+00:00", "2026-10-21T12:00:00Z"),
        ("2026-10-20T12:00:00.000Z", "2026-10-20T12:00:00.001Z"),
    ] {
        let mut value = fixture(); value["window"] = json!({"notBefore":before,"notAfter":after});
        assert_eq!(serde_json::to_value(check(value.clone()).unwrap()).unwrap(), value);
    }
    for (before, after) in [
        ("2026-10-20T12:00:00Z", "2026-10-20T12:00:00Z"),
        ("2026-10-20T13:00:00Z", "2026-10-20T12:00:00Z"),
        ("2026-10-20T12:00:00Z", "2026-10-21T12:00:00.001Z"),
        ("2026-10-20T12:00:00-00:00", "2026-10-20T13:00:00Z"),
        ("2026-10-20T12:00:00+01:00", "2026-10-20T13:00:00Z"),
        ("2026-10-20", "2026-10-20T13:00:00Z"),
    ] {
        let mut value = fixture(); value["window"] = json!({"notBefore":before,"notAfter":after}); refuse(value);
    }
}

#[test]
fn provider_origins_and_navigate_urls() {
    for origin in ["https://example.com", "https://EXAMPLE.com:443", "https://example.com:65535", "https://[::1]", "https://[2001:db8::1]:443"] {
        let mut value = fixture(); value["provider"]["origins"] = json!([origin, origin]);
        value["browserSteps"][0]["literal"] = json!(format!("{origin}/settings?x=1#section")); check(value).unwrap();
    }
    for origin in ["http://example.com", "HTTPS://example.com", "https://example.com/", "https://*.example.com", "https://a@b", "https://example.com?x", "https://example.com#x", "https://example.com:0", "https://example.com:65536", "https://-a.com", "https://a-.com", "https://a..com", "https://é.com", "https://%61.com", "https://[x]", "https://a\\b"] {
        let mut value = fixture(); value["provider"]["origins"] = json!([origin]); refuse(value);
    }
    for url in ["http://example.com", "HTTPS://example.com", "https://person@example.com", "https://example.com:444/", "https://example.com/a b", "https://example.com/\\x", "https://example.com/\nx"] {
        let mut value = fixture(); value["browserSteps"][0]["literal"] = json!(url); refuse(value);
    }
    let mut value = fixture(); value["provider"]["origins"] = json!([]); refuse(value);
}

#[test]
fn ordered_indexes_and_secret_issue_destination() {
    for index in [0, 2, MAX_SAFE_INTEGER + 1] {
        let mut value = fixture(); value["browserSteps"][1]["index"] = json!(index); refuse(value);
    }
    let mut value = fixture(); value["browserSteps"][0]["index"] = json!(MAX_SAFE_INTEGER - 1);
    value["browserSteps"][1]["index"] = json!(MAX_SAFE_INTEGER); check(value).unwrap();
    for bad in ["missing", ""] {
        let mut value = fixture(); value["browserSteps"][1]["issues"] = json!(bad); refuse(value);
    }
    let mut value = fixture(); value["writes"][0]["fields"][0]["kind"] = json!("metadata"); refuse(value);
    let mut value = fixture(); let mut duplicate = value["browserSteps"][1].clone(); duplicate["index"] = json!(10);
    value["browserSteps"].as_array_mut().unwrap().push(duplicate); refuse(value);
}

#[test]
fn write_uniqueness_destructive_shapes_and_exact_targets() {
    let mut value = fixture(); let duplicate = value["writes"][0].clone(); value["writes"].as_array_mut().unwrap().push(duplicate); refuse(value);
    let mut value = fixture(); let duplicate = value["writes"][0]["fields"][0].clone(); value["writes"][0]["fields"].as_array_mut().unwrap().push(duplicate); refuse(value);
    for item_op in ["archive", "destroy"] {
        let mut value = fixture(); value["browserSteps"] = json!([]);
        value["writes"][0]["itemOp"] = json!(item_op); refuse(value.clone());
        value["writes"][0]["fields"] = json!([]); check(value.clone()).unwrap();
        if item_op == "destroy" { value["writes"][0]["itemId"] = json!("*"); refuse(value); }
    }
    let mut value = fixture(); value["browserSteps"] = json!([]); value["writes"][0]["fields"] = json!([]); check(value).unwrap();
    let mut value = fixture(); value["browserSteps"] = json!([]); value["writes"][0]["fields"][0]["op"] = json!("remove");
    value["writes"][0]["fields"][0]["inventoryId"] = json!("*"); refuse(value);
    let locator = json!({"kind":"text","value":"Delete"});
    for step in [
        json!({"index":0,"op":"click","locator":locator,"effect":"delete"}),
        json!({"index":0,"op":"click","locator":locator,"target":"token"}),
        json!({"index":0,"op":"click","locator":locator,"effect":"delete","target":"*"}),
        json!({"index":0,"op":"human","kind":"passkey","note":"\n"}),
    ] { let mut value = fixture(); value["browserSteps"] = json!([step]); refuse(value); }
}

#[test]
fn readers_are_exact_scoped_positive_and_pair_unique() {
    let reader = json!({"principal":"agent/reader","peerHost":"peer","inventoryIds":["token","token"],"maxUses":1});
    let mut value = fixture(); value["readers"] = json!([reader]); check(value.clone()).unwrap();
    for (key, bad) in [("principal", json!("person/reader")), ("principal", json!("agent/**")), ("peerHost", json!("*")), ("maxUses", json!(0)), ("maxUses", json!(MAX_SAFE_INTEGER+1)), ("inventoryIds", json!(["unknown"]))] {
        let mut invalid = value.clone(); invalid["readers"][0][key] = bad; refuse(invalid);
    }
    let mut duplicate = value["readers"][0].clone(); duplicate["maxUses"] = json!(2);
    value["readers"].as_array_mut().unwrap().push(duplicate); refuse(value.clone());
    value["readers"][1]["peerHost"] = json!("other-peer"); check(value.clone()).unwrap();
    value["readers"][0]["inventoryIds"] = json!([]); check(value).unwrap();
}

#[test]
fn fill_secret_read_only_scope_is_requester_only() {
    let mut value = fixture(); value["browserSteps"] = json!([{"index":0,"op":"fillSecret","locator":{"kind":"label","value":"Password"},"inventoryId":"password"}]);
    refuse(value.clone());
    value["readers"] = json!([{"principal":"agent/requester","peerHost":"peer","inventoryIds":["password"],"maxUses":1}]); check(value.clone()).unwrap();
    value["readers"].as_array_mut().unwrap().push(json!({"principal":"agent/other","peerHost":"peer","inventoryIds":["password"],"maxUses":1})); refuse(value.clone());
    value["writes"][0]["fields"].as_array_mut().unwrap().push(json!({"inventoryId":"password","kind":"secret"})); check(value).unwrap();
}

#[test]
fn general_text_shares_ecmascript_blank_set_but_still_forbids_controls() {
    let whitespace = "\u{0009}\u{000a}\u{000b}\u{000c}\u{000d}\u{0020}\u{00a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";
    for c in whitespace.chars() {
        assert!(crate::task_grants::ecma_blank(&c.to_string()));
        assert!(text(&c.to_string()).is_err());
    }
    assert!(!crate::task_grants::ecma_blank("\u{85}"));
    assert!(text("\u{85}x").is_err()); // C1 control, not blank content.
    assert!(text("\u{feff}x").is_ok());
    for pointer in ["/title", "/requester/run", "/requester/step", "/requester/incarnation",
        "/provider/name", "/provider/browserProfile", "/writes/0/vaultId", "/writes/0/itemId"] {
        let mut value = fixture();
        *value.pointer_mut(pointer).unwrap() = json!("\u{feff}");
        refuse(value);
    }
}
