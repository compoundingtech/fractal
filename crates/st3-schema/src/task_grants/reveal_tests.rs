use super::*;
use serde_json::{Value, json};

// Independently computed in Bun: UTF-16 Object.keys().sort(), JSON.stringify
// primitives, arrays in declared order, then node:crypto SHA-256 over UTF-8.
const INPUT: &str = r#"{"version":1,"title":"Example reveal","requester":{"principal":"agent/example-requester","run":"example-run","step":"example-step","incarnation":"example-1"},"window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},"browserSteps":[{"index":0,"op":"revealOnce","locator":{"kind":"label","value":"Token"},"trigger":{"locator":{"kind":"role","role":"button","name":"Create token"}},"issues":"example-token","sourceKind":"textNode","textNode":{"pattern":"[A-Za-z0-9_-]{16,64}"},"intent":"Create example token","safeUrl":"https://example.com/settings"}],"writes":[{"vaultId":"example-vault","itemId":"example-item","fields":[{"inventoryId":"example-token","kind":"secret"}]}],"readers":[]}"#;
const CANONICAL: &str = r#"{"browserSteps":[{"index":0,"intent":"Create example token","issues":"example-token","locator":{"kind":"label","value":"Token"},"op":"revealOnce","safeUrl":"https://example.com/settings","sourceKind":"textNode","textNode":{"pattern":"[A-Za-z0-9_-]{16,64}"},"trigger":{"locator":{"kind":"role","name":"Create token","role":"button"}}}],"provider":{"browserProfile":"example","name":"example","origins":["https://example.com"]},"readers":[],"requester":{"incarnation":"example-1","principal":"agent/example-requester","run":"example-run","step":"example-step"},"title":"Example reveal","version":1,"window":{"notAfter":"2026-10-20T13:00:00Z","notBefore":"2026-10-20T12:00:00Z"},"writes":[{"fields":[{"inventoryId":"example-token","kind":"secret"}],"itemId":"example-item","vaultId":"example-vault"}]}"#;

fn fixture() -> Value { serde_json::from_str(INPUT).unwrap() }
fn refused(value: Value) {
    if let Ok(manifest) = serde_json::from_value::<TaskManifest>(value) {
        assert!(manifest.validate().is_err(), "unexpectedly accepted {manifest:?}");
    }
}

#[test]
fn reveal_once_text_node_golden() {
    let manifest: TaskManifest = serde_json::from_str(INPUT).unwrap();
    assert_eq!(manifest.canonical_json().unwrap(), CANONICAL);
    assert_eq!(manifest.digest().unwrap(), "80fb6ef6e2f9ae31f8fea172bde4cb3247f9db641758bcbf41790f68f1df82a1");
    let mut absent = fixture();
    absent["browserSteps"][0].as_object_mut().unwrap().remove("sourceKind");
    absent["browserSteps"][0].as_object_mut().unwrap().remove("textNode");
    let manifest: TaskManifest = serde_json::from_value(absent).unwrap();
    manifest.validate().unwrap();
    assert!(!serde_json::to_value(manifest).unwrap()["browserSteps"][0].as_object().unwrap().contains_key("sourceKind"));
}

#[test]
fn reveal_once_requires_trigger_unique_secret_issue_and_matching_source_kind() {
    for member in ["trigger", "issues", "sourceKind", "textNode"] {
        let mut value = fixture();
        value["browserSteps"][0].as_object_mut().unwrap().remove(member);
        refused(value);
    }
    let mut metadata = fixture();
    metadata["writes"][0]["fields"][0]["kind"] = json!("metadata");
    refused(metadata);
    let mut duplicate = fixture();
    let mut step = duplicate["browserSteps"][0].clone();
    step["index"] = json!(1);
    duplicate["browserSteps"].as_array_mut().unwrap().push(step);
    refused(duplicate);
    for kind in ["input", "maskedText"] {
        let mut value = fixture();
        value["browserSteps"][0]["sourceKind"] = json!(kind);
        refused(value.clone()); // textNode forbidden for other source kinds.
        value["browserSteps"][0].as_object_mut().unwrap().remove("textNode");
        serde_json::from_value::<TaskManifest>(value).unwrap().validate().unwrap();
    }
    for pointer in ["/browserSteps/0/trigger", "/browserSteps/0/textNode"] {
        let mut value = fixture();
        value.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("unknown".into(), json!(true));
        refused(value);
    }
}

#[test]
fn reveal_once_checks_safe_url_intent_and_pattern_capture_bounds() {
    for safe_url in ["https://example.com/#secret", "https://example.com/#", "https://example.com:444/", "http://example.com/", "https://person@example.com/"] {
        let mut value = fixture();
        value["browserSteps"][0]["safeUrl"] = json!(safe_url);
        refused(value);
    }
    for safe_url in [format!("https://example.com/{}", "x".repeat(4096)), format!("https://example.com/{}", "é".repeat(2040))] {
        let mut value = fixture();
        value["browserSteps"][0]["safeUrl"] = json!(safe_url);
        refused(value);
    }
    for pattern in ["", "a+z+", "(a+)+", ".*", "[z-a]", "a{3,2}", "a{1000}", "a\\."] {
        let mut value = fixture();
        value["browserSteps"][0]["textNode"]["pattern"] = json!(pattern);
        refused(value);
    }
    for (member, value) in [("intent", "x".repeat(257)), ("intent", "😀".repeat(129)), ("intent", "\u{feff}".into())] {
        let mut manifest = fixture();
        manifest["browserSteps"][0][member] = json!(value);
        refused(manifest);
    }
    let mut too_long = fixture();
    too_long["browserSteps"][0]["textNode"]["pattern"] = json!("x".repeat(257));
    refused(too_long);
    for pattern in ["[A-Za-z0-9_-]{16,64}", "a+\\.b+", "example_token"] {
        let mut value = fixture();
        value["browserSteps"][0]["textNode"]["pattern"] = json!(pattern);
        serde_json::from_value::<TaskManifest>(value).unwrap().validate().unwrap();
    }
}
