use super::*;

// Independently computed using Bun/ECMAScript: recursively sort object keys with
// Array.sort() (UTF-16), serialize primitives with JSON.stringify(), then SHA-256.
#[test]
fn role_locator_manifest_golden() {
    let input = r#"{"version":1,"title":"Example grant","requester":{"principal":"person/robin","run":"example-run","step":"example-step","incarnation":"example-1"},"window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},"browserSteps":[{"index":0,"op":"click","locator":{"kind":"role","role":"button","name":"Create token"}}],"writes":[],"readers":[]}"#;
    let canonical = r#"{"browserSteps":[{"index":0,"locator":{"kind":"role","name":"Create token","role":"button"},"op":"click"}],"provider":{"browserProfile":"example","name":"example","origins":["https://example.com"]},"readers":[],"requester":{"incarnation":"example-1","principal":"person/robin","run":"example-run","step":"example-step"},"title":"Example grant","version":1,"window":{"notAfter":"2026-10-20T13:00:00Z","notBefore":"2026-10-20T12:00:00Z"},"writes":[]}"#;
    let manifest: TaskManifest = serde_json::from_str(input).unwrap();
    assert_eq!(manifest.canonical_json().unwrap(), canonical);
    assert_eq!(manifest.digest().unwrap(), "8301dacbae3d2676dc940c02eb218dcd41f2ecae0d53a3f01fcfbb7386e8016e");
}

#[test]
fn nested_locator_manifest_golden() {
    let input = r#"{"version":1,"title":"Example grant","requester":{"principal":"person/robin","run":"example-run","step":"example-step","incarnation":"example-1"},"window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},"browserSteps":[{"index":0,"op":"click","locator":{"kind":"within","scope":{"kind":"css","selector":"[role=\"dialog\"]"},"target":{"kind":"role","role":"textbox","name":"Token"}}}],"writes":[],"readers":[]}"#;
    let canonical = r#"{"browserSteps":[{"index":0,"locator":{"kind":"within","scope":{"kind":"css","selector":"[role=\"dialog\"]"},"target":{"kind":"role","name":"Token","role":"textbox"}},"op":"click"}],"provider":{"browserProfile":"example","name":"example","origins":["https://example.com"]},"readers":[],"requester":{"incarnation":"example-1","principal":"person/robin","run":"example-run","step":"example-step"},"title":"Example grant","version":1,"window":{"notAfter":"2026-10-20T13:00:00Z","notBefore":"2026-10-20T12:00:00Z"},"writes":[]}"#;
    let manifest: TaskManifest = serde_json::from_str(input).unwrap();
    assert_eq!(manifest.canonical_json().unwrap(), canonical);
    assert_eq!(manifest.digest().unwrap(), "62042ff9f29159b6780365ba7b63f2a0180fc806289e6f72ba5b6eb5c1494d9c");
}

#[test]
fn browser_members_are_closed_over_their_operations() {
    use serde_json::json;
    let base = json!({
        "version":1,"title":"Example grant",
        "requester":{"principal":"agent/example-requester","run":"example-run","step":"example-step","incarnation":"example-1"},
        "window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},
        "provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},
        "browserSteps":[],
        "writes":[{"vaultId":"example","itemId":"example","fields":[{"inventoryId":"example-token","kind":"secret"}]}],
        "readers":[{"principal":"agent/example-requester","peerHost":"example","inventoryIds":["example-token"],"maxUses":1}]
    });
    for op in ["navigate", "click", "fill", "revealOnce", "fillSecret", "human"] {
        let mut step = json!({"index":0,"op":op});
        if op == "navigate" {
            step["literal"] = json!("https://example.com");
        } else if op == "human" {
            step["kind"] = json!("passkey");
            step["note"] = json!("Approve example");
        } else {
            step["locator"] = json!({"kind":"role","role":"button","name":"Example"});
        }
        if op == "fillSecret" { step["inventoryId"] = json!("example-token"); }
        let mut valid = base.clone();
        valid["browserSteps"] = json!([step.clone()]);
        serde_json::from_value::<TaskManifest>(valid).unwrap().validate().unwrap();
        for (member, value, prohibited) in [
            ("kind", json!("passkey"), op != "human"),
            ("note", json!("Approve example"), op != "human"),
            ("inventoryId", json!("example-token"), op != "fillSecret"),
            ("literal", json!("public"), !matches!(op, "fill" | "navigate")),
            ("issues", json!("example-token"), op != "revealOnce"),
            ("effect", json!("delete"), op != "click"),
            ("target", json!("example-target"), true),
        ] {
            if !prohibited { continue; }
            let mut invalid = base.clone();
            let mut invalid_step = step.clone();
            invalid_step[member] = value;
            if member == "effect" { invalid_step["target"] = json!("example-target"); }
            invalid["browserSteps"] = json!([invalid_step]);
            let manifest: TaskManifest = serde_json::from_value(invalid).unwrap();
            assert!(manifest.validate().is_err(), "{op} must reject {member}");
        }
    }
}

// Independently computed with the same Bun JCS procedure documented above.
const READ_ONLY_LOGIN: &str = r#"{"version":1,"title":"Example login","requester":{"principal":"agent/example-requester","run":"example-run","step":"example-step","incarnation":"example-1"},"window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},"browserSteps":[{"index":0,"op":"fillSecret","locator":{"kind":"label","value":"Password"},"inventoryId":"example-login-password"}],"writes":[],"readers":[{"principal":"agent/example-requester","peerHost":"example","inventoryIds":["example-login-password"],"maxUses":1}]}"#;

#[test]
fn read_only_login_secret_golden() {
    let canonical = r#"{"browserSteps":[{"index":0,"inventoryId":"example-login-password","locator":{"kind":"label","value":"Password"},"op":"fillSecret"}],"provider":{"browserProfile":"example","name":"example","origins":["https://example.com"]},"readers":[{"inventoryIds":["example-login-password"],"maxUses":1,"peerHost":"example","principal":"agent/example-requester"}],"requester":{"incarnation":"example-1","principal":"agent/example-requester","run":"example-run","step":"example-step"},"title":"Example login","version":1,"window":{"notAfter":"2026-10-20T13:00:00Z","notBefore":"2026-10-20T12:00:00Z"},"writes":[]}"#;
    let manifest: TaskManifest = serde_json::from_str(READ_ONLY_LOGIN).unwrap();
    manifest.validate().unwrap();
    assert!(manifest.writes.is_empty());
    assert_eq!(manifest.canonical_json().unwrap(), canonical);
    assert_eq!(manifest.digest().unwrap(), "55456794982713348ac36ea43c1ecf52eb4931e016c127d234b976f5662b7444");
}

#[test]
fn read_only_login_secret_is_requester_only_and_must_be_referenced() {
    use serde_json::{Value, json};
    let original: Value = serde_json::from_str(READ_ONLY_LOGIN).unwrap();
    let mut unrelated = original.clone();
    let mut reader = unrelated["readers"][0].clone();
    reader["principal"] = json!("agent/example-unrelated");
    unrelated["readers"].as_array_mut().unwrap().push(reader);
    assert!(serde_json::from_value::<TaskManifest>(unrelated).unwrap().validate().is_err());
    let mut undeclared = original.clone();
    undeclared["readers"][0]["inventoryIds"].as_array_mut().unwrap().push(json!("example-undeclared"));
    assert!(serde_json::from_value::<TaskManifest>(undeclared).unwrap().validate().is_err());
    let mut ungranted = original.clone();
    ungranted["readers"] = json!([]);
    assert!(serde_json::from_value::<TaskManifest>(ungranted).unwrap().validate().is_err());
    let mut not_filled = original;
    not_filled["browserSteps"] = json!([]);
    assert!(serde_json::from_value::<TaskManifest>(not_filled).unwrap().validate().is_err());
}
