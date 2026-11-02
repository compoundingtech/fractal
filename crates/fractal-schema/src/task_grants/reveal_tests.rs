use super::{bounded_text, valid_pattern, validate};
use crate::task_grants::browser::TaskBrowserStepV1;
use crate::task_grants::locator::{TaskLocatorV1, TaskNonFrameLocatorV1};
use crate::task_grants::TaskProviderV1;
use serde_json::{Value, json};

fn provider() -> TaskProviderV1 {
    TaskProviderV1 {
        name: "Example".into(), browser_profile: "default".into(),
        origins: vec!["https://example.com:443".into()],
    }
}

fn leaf() -> Value { json!({"kind":"text","value":"Token"}) }

fn step() -> Value {
    json!({"op":"revealOnce","index":0,"locator":leaf(),
        "trigger":{"locator":leaf()},"issues":"new-token"})
}

fn accepts(value: Value) -> bool {
    let step: TaskBrowserStepV1 = serde_json::from_value(value).unwrap();
    validate(&step, &provider()).is_ok()
}

#[test]
fn restricted_pattern_valid_atoms_classes_segments_and_quantifiers() {
    for value in [
        "a", "ABC_019-", "[a-zA-Z0-9_-]", "[-a-z]", "[a-z-]", "[-]",
        "[a-a]", "[0-_]", "[a-b-c]", "a+", "a*", "a?", "a{0}",
        "a{999}", "a{001}", "a{2,}", "a{0,999}", "a{007,010}",
        "abc_[A-Z]+tail", r"key_[A-Z]+\.[0-9]{2,8}\.[a-z]*", "a{2,2}",
    ] {
        assert!(valid_pattern(value), "{value:?}");
    }
}

#[test]
fn restricted_pattern_rejects_general_regex_and_invalid_bounds() {
    for value in [
        "", ".", "a.b", r"\.a", r"a\.", r"a\.\.b", r"a\x", r"a\\b",
        "a|b", "(a)", "^a$", "é", "😀", "a b", "[]", "[^a]", "[a.]",
        "[z-a]", "[9-0]", "[a--]", "[--a]", "[a--z]", "[--]", "[a", "a]", "a++", "a*?", "a+b+",
        "[a]+[b]?", "a{}", "a{,}", "a{,2}", "a{1,0}", "a{1000}",
        "a{0,1000}", "a{1,2,3}", "a{1", "a{1}*", "a{+1}", "a{-1}",
        "a{ 1}", "a\n", "[é]", "[a\\d]", "a{１}",
    ] {
        assert!(!valid_pattern(value), "{value:?}");
    }
}

#[test]
fn bounded_text_uses_ecmascript_trim_and_utf16_without_rewriting() {
    for value in ["", "\t\n\u{b}\u{c}\r ", "\u{a0}\u{1680}\u{2000}\u{200a}",
        "\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}", "a\0b"] {
        assert!(bounded_text(value, 256).is_err(), "{value:?}");
    }
    for value in ["\u{85}", "\u{180e}", "\u{200b}", "\u{1}", "\u{feff}a\n"] {
        assert!(bounded_text(value, 256).is_ok(), "{value:?}");
    }
    assert!(bounded_text(&"😀".repeat(128), 256).is_ok());
    assert!(bounded_text(&format!("{}a", "😀".repeat(128)), 256).is_err());
}

#[test]
fn source_kind_requires_text_node_if_and_only_if_selected() {
    for kind in [None, Some("input"), Some("maskedText"), Some("textNode")] {
        for has_node in [false, true] {
            let mut value = step();
            if let Some(kind) = kind { value["sourceKind"] = json!(kind); }
            if has_node { value["textNode"] = json!({"pattern":"token_[a-z]+"}); }
            assert_eq!(accepts(value), (kind == Some("textNode")) == has_node);
        }
    }
}

#[test]
fn reveal_pattern_and_intent_have_independent_256_unit_bounds() {
    let mut value = step();
    value["sourceKind"] = json!("textNode");
    value["textNode"] = json!({"pattern":"a".repeat(256)});
    value["intent"] = json!("😀".repeat(128));
    assert!(accepts(value.clone()));
    value["textNode"]["pattern"] = json!("a".repeat(257));
    assert!(!accepts(value.clone()));
    value["textNode"]["pattern"] = json!("a");
    for intent in ["a".repeat(257), "😀".repeat(129), "\u{feff}".into(), "a\0b".into()] {
        value["intent"] = json!(intent);
        assert!(!accepts(value.clone()));
    }
    value["intent"] = json!("\u{85}\u{1}");
    assert!(accepts(value));
}

#[test]
fn safe_url_checks_parsed_origin_credentials_fragment_and_both_bounds() {
    let mut value = step();
    for url in ["https://example.com/path?q=1", "HTTPS://EXAMPLE.COM:443/", " https://example.com/ "] {
        value["safeUrl"] = json!(url);
        assert!(accepts(value.clone()), "{url:?}");
    }
    for url in ["http://example.com", "https://other.com", "https://user@example.com",
        "https://user:pass@example.com", "https://example.com/#", "https://example.com/#x",
        "\u{feff}", "https://example.com/\0"] {
        value["safeUrl"] = json!(url);
        assert!(!accepts(value.clone()), "{url:?}");
    }
    let prefix = "https://example.com/";
    value["safeUrl"] = json!(format!("{prefix}{}", "a".repeat(4096 - prefix.len())));
    assert!(accepts(value.clone()));
    value["safeUrl"] = json!(format!("{prefix}{}", "a".repeat(4097 - prefix.len())));
    assert!(!accepts(value.clone()));
    let count = (4096 - prefix.len()) / 4;
    let padding = (4096 - prefix.len()) % 4;
    value["safeUrl"] = json!(format!("{prefix}{}{}", "😀".repeat(count), "a".repeat(padding)));
    assert!(accepts(value.clone()));
    value["safeUrl"] = json!(format!("{prefix}{}{}a", "😀".repeat(count), "a".repeat(padding)));
    assert!(!accepts(value));
}

fn within(scope: Value, target: Value) -> Value {
    json!({"kind":"within","scope":scope,"target":target})
}

fn balanced(height: usize) -> Value {
    if height == 1 { leaf() } else { within(balanced(height - 1), balanced(height - 1)) }
}

fn nth(child: Value, index: u64) -> Value {
    json!({"kind":"nth","within":child,"role":"button","index":index})
}

fn valid_locator(value: Value) -> bool {
    serde_json::from_value::<TaskLocatorV1>(value).unwrap().validate().is_ok()
}

#[test]
fn locator_node_depth_and_nth_synthetic_leaf_budgets() {
    // Every branching node has two children, including nth's synthetic leaf:
    // representable node counts are odd, so 31 and 33 straddle the 32-node cap.
    assert!(valid_locator(balanced(5)));
    assert!(!valid_locator(within(balanced(5), leaf())));
    let mut chain = leaf();
    for _ in 0..7 { chain = nth(chain, 512); }
    assert!(valid_locator(chain.clone()));
    assert!(!valid_locator(nth(chain, 1)));
    assert!(valid_locator(nth(leaf(), 1)));
    assert!(!valid_locator(nth(leaf(), 0)));
    assert!(!valid_locator(nth(leaf(), 513)));
    // Seven branches plus eight nth nodes, each with two leaves, total 31.
    let mut count = nth(leaf(), 1);
    for _ in 0..3 { count = within(count.clone(), count); }
    assert!(valid_locator(count.clone()));
    // Adding one nth contributes itself AND its synthetic role leaf.
    assert!(!valid_locator(nth(count, 1)));
    let inner: TaskNonFrameLocatorV1 = serde_json::from_value(balanced(5)).unwrap();
    assert!(inner.validate().is_ok());
}

#[test]
fn frame_adds_outer_node_and_source_trigger_budgets_are_independent() {
    let mut value = step();
    value["locator"] = balanced(5);
    value["trigger"]["locator"] = balanced(5);
    assert!(accepts(value.clone()));
    value["trigger"]["locator"] = within(balanced(5), leaf());
    assert!(!accepts(value.clone()));
    value["trigger"]["locator"] = leaf();
    assert!(accepts(value.clone()));
    value["locator"] = within(balanced(5), leaf());
    assert!(!accepts(value));
    assert!(valid_locator(json!({"kind":"withinFrame","frame":balanced(4),"target":balanced(4)})));
    assert!(!valid_locator(json!({"kind":"withinFrame","frame":balanced(5),"target":leaf()})));
    let mut target = leaf();
    for _ in 0..6 { target = nth(target, 1); }
    assert!(valid_locator(json!({"kind":"withinFrame","frame":leaf(),"target":target})));
    target = nth(target, 1);
    assert!(!valid_locator(json!({"kind":"withinFrame","frame":leaf(),"target":target})));
}

#[test]
fn every_locator_payload_uses_its_utf16_blank_and_nul_bounds() {
    for (kind, field, limit) in [("role", "name", 512), ("label", "value", 512),
        ("placeholder", "value", 512), ("testId", "value", 512), ("text", "value", 512),
        ("css", "selector", 2048)] {
        let mut value = json!({"kind":kind});
        if kind == "role" { value["role"] = json!("button"); }
        value[field] = json!("😀".repeat(limit / 2));
        assert!(valid_locator(value.clone()));
        value[field] = json!(format!("{}a", "😀".repeat(limit / 2)));
        assert!(!valid_locator(value.clone()));
        for bad in ["", "\u{feff}\u{a0}\n", "a\0b"] {
            value[field] = json!(bad);
            assert!(!valid_locator(value.clone()));
        }
        value[field] = json!("\u{85}\u{1}");
        assert!(valid_locator(value));
    }
}
