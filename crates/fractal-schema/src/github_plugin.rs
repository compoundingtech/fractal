//! The two graph kinds the GitHub plugin may write (docs/st3/plugins.md). `vcs.item.state` is one
//! pull request's or issue's current state, a fixed compact field set written on change on the
//! item's own resource subject. `github.event` is one keyed thing that happened to an item, once
//! per key. Everything else the poller sees stays in its plugin database.
use super::*;

pub const ITEM_STATE: &str = "vcs.item.state";
pub const EVENT: &str = "github.event";
/// The subject family of `github.event` claims: `github-event/RESOURCE/ITEM/NUMBER/KEY...`, the
/// item's resource subject without its `resource/` prefix, then the event key.
pub const EVENT_FAMILY: &str = "github-event";
pub const VERSION: i64 = 1;

/// The most failed check names one `vcs.item.state` carries; the rest are in the mirror.
pub const MAX_FAILED_CHECKS: usize = 20;
/// The largest body the plugin may write for each kind, in bytes of canonical JSON fields.
pub const MAX_ITEM_STATE_BYTES: usize = 2 * 1024;
pub const MAX_EVENT_BYTES: usize = 1024;

/// The item kinds `vcs.item.state` describes.
pub const ITEM_KINDS: &[&str] = &["vcs.pull-request", "vcs.issue"];

/// What a `github.event` says happened.
pub const EVENTS: &[&str] = &[
    "opened",
    "closed",
    "merged",
    "reopened",
    "head-changed",
    "became-draft",
    "became-ready",
    "merge-queue-entered",
    "merge-queue-left",
    "checks-passed",
    "checks-failed",
    "comment",
    "review",
];

/// The compact facts `vcs.item.state` carries, besides its version, kind and observer. A reader
/// lays them over the item's older facts; a null removes the fact.
pub const COMPACT_FACTS: &[&str] = &[
    "state",
    "merged",
    "draft",
    "head_sha",
    "checks_state",
    "review_decision",
    "merge_queue",
    "required_checks",
];

/// The facts outside the compact set that a wake or a delivery reads. A change to any of them
/// is still recorded as a full snapshot in compact mode, so a poller without the mirror, as after
/// a move, compares with exact knowledge and delivers nothing twice. `required_checks` counts
/// through its `execution` only; its state is compact.
pub const DELIVERY_FACTS: &[&str] = &["recent_comments", "comments", "last_comment", "mentions", "reactions", "reviews"];

/// Whether `facts` differ from `prior` in a fact a wake or a delivery reads: one of
/// [`DELIVERY_FACTS`], or `required_checks.execution`. A new item, with no prior, does.
pub fn delivery_facts_changed(
    prior: Option<&serde_json::Map<String, Value>>,
    facts: &serde_json::Map<String, Value>,
) -> bool {
    let Some(prior) = prior else {
        return true;
    };
    let execution = |facts: &serde_json::Map<String, Value>| {
        facts.get("required_checks").and_then(|checks| checks.get("execution")).cloned()
    };
    DELIVERY_FACTS.iter().any(|name| prior.get(*name) != facts.get(*name)) || execution(prior) != execution(facts)
}

pub(crate) fn item_state_fields() -> &'static [(&'static str, FieldSpec)] {
    static FIELDS: OnceLock<Vec<(&'static str, FieldSpec)>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        vec![
            ("version", required_integer()),
            ("item_kind", required_enum(ITEM_KINDS)),
            ("observer", required_reference_to(&["observer"])),
            ("state", required_enum(&["open", "closed"])),
            ("merged", boolean()),
            ("draft", boolean()),
            ("head_sha", string()),
            ("checks_state", string()),
            ("review_decision", string()),
            ("merge_queue", object()),
            ("required_checks", object()),
        ]
    })
}

pub(crate) fn event_fields() -> &'static [(&'static str, FieldSpec)] {
    static FIELDS: OnceLock<Vec<(&'static str, FieldSpec)>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        vec![
            ("version", required_integer()),
            ("item", required_reference_to(&["resource"])),
            ("event", required_enum(EVENTS)),
            ("observer", required_reference_to(&["observer"])),
            ("at", required_string()),
            ("head_sha", string()),
            ("detail", object()),
        ]
    })
}

/// The `vcs.item.state` fields for an item's full facts: `None` for a kind it does not describe.
/// `required_checks` keeps its state, source and at most [`MAX_FAILED_CHECKS`] failed names.
pub fn compact_item_state(
    item_kind: &str,
    observer: &str,
    facts: &serde_json::Map<String, Value>,
) -> Option<serde_json::Map<String, Value>> {
    if !ITEM_KINDS.contains(&item_kind) {
        return None;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("version".into(), VERSION.into());
    fields.insert("item_kind".into(), item_kind.into());
    fields.insert("observer".into(), observer.into());
    let state = match facts.get("state").and_then(Value::as_str) {
        Some("open") => "open",
        _ if facts.get("merged").and_then(Value::as_bool) == Some(true) => "closed",
        Some(_) => "closed",
        None => "open",
    };
    fields.insert("state".into(), state.into());
    for name in ["merged", "draft", "head_sha", "checks_state", "review_decision", "merge_queue"] {
        fields.insert(name.into(), facts.get(name).cloned().unwrap_or(Value::Null));
    }
    let required = facts.get("required_checks").and_then(Value::as_object).map(|required| {
        let mut summary = serde_json::Map::new();
        for name in ["state", "source"] {
            if let Some(value) = required.get(name) {
                summary.insert(name.into(), value.clone());
            }
        }
        if let Some(failed) = required.get("failed").and_then(Value::as_array) {
            summary.insert(
                "failed".into(),
                Value::Array(failed.iter().take(MAX_FAILED_CHECKS).cloned().collect()),
            );
        }
        Value::Object(summary)
    });
    fields.insert("required_checks".into(), required.unwrap_or(Value::Null));
    Some(fields)
}

/// What a `vcs.item.state` sets on an item's facts: each compact fact's value, or `None` where it
/// removes that fact. Facts it does not carry (title, comments) stay as they were.
pub fn item_state_updates(
    state: &serde_json::Map<String, Value>,
) -> impl Iterator<Item = (&'static str, Option<&Value>)> {
    COMPACT_FACTS
        .iter()
        .map(|name| (*name, state.get(*name).filter(|value| !value.is_null())))
}

/// The value a compact fact takes over an item's `prior` value of it. `required_checks` is a
/// summary: its state, source and failed names replace those of the prior object, and the rest
/// (the checks it counts, their execution) stays. That rest is the last snapshot's, and after a
/// new head it may belong to an earlier one; the next look then sees a different execution and
/// writes a snapshot. Every other compact fact replaces its prior.
pub fn compact_fact_over(name: &str, prior: Option<&Value>, compact: &Value) -> Value {
    match (name, prior.and_then(Value::as_object), compact.as_object()) {
        ("required_checks", Some(prior), Some(summary)) => {
            let mut merged = prior.clone();
            for field in ["state", "source", "failed"] {
                match summary.get(field) {
                    Some(value) => merged.insert(field.into(), value.clone()),
                    None => merged.remove(field),
                };
            }
            Value::Object(merged)
        }
        _ => compact.clone(),
    }
}

/// Lay a `vcs.item.state`'s compact fields over an item's facts: each compact fact replaces the
/// fact of that name (see [`compact_fact_over`]), and a null removes it. Facts it does not carry
/// (title, comments) stay.
pub fn overlay_item_state(
    facts: &mut serde_json::Map<String, Value>,
    state: &serde_json::Map<String, Value>,
) {
    for (name, value) in item_state_updates(state) {
        match value {
            None => {
                facts.remove(name);
            }
            Some(value) => {
                let value = compact_fact_over(name, facts.get(name), value);
                facts.insert(name.into(), value);
            }
        }
    }
}

/// A claim body as readers of a pinned or discovered item observation read it: a
/// `vcs.item.state` on `subject` becomes `{"fields": {"kind", "facts"}}`, its facts the compact
/// fields that are set and the item's number, so a reader of `fields.kind` and `fields.facts`
/// reads it like a `resource.observed`. Any other claim's body is returned as it is.
pub fn observed_body(kind: &str, subject: &str, body: &Value) -> Value {
    if kind != ITEM_STATE {
        return body.clone();
    }
    let fields = body.get("fields").and_then(Value::as_object);
    let mut facts = serde_json::Map::new();
    if let Some(fields) = fields {
        for (name, value) in item_state_updates(fields) {
            if let Some(value) = value {
                facts.insert(name.into(), value.clone());
            }
        }
    }
    if let Some(number) = subject.rsplit('/').next().and_then(|number| number.parse::<u64>().ok()) {
        facts.insert("number".into(), number.into());
    }
    let mut observed = serde_json::Map::new();
    observed.insert(
        "fields".into(),
        serde_json::json!({
            "kind": fields.and_then(|fields| fields.get("item_kind")).cloned().unwrap_or(Value::Null),
            "facts": facts,
            "observer": fields.and_then(|fields| fields.get("observer")).cloned().unwrap_or(Value::Null),
        }),
    );
    if let Some(evidence) = body.get("evidence") {
        observed.insert("evidence".into(), evidence.clone());
    }
    Value::Object(observed)
}

/// The subject of one keyed event on one item. `item` is the item's resource subject,
/// `RESOURCE/pull-request/N` or `RESOURCE/issue/N`, usually `resource/github/OWNER/REPO/...`;
/// `key` names the event, such as `checks/SHA/fail` or `comment/123`.
pub fn event_subject(item: &str, key: &str) -> Option<String> {
    let rest = item.strip_prefix("resource/").filter(|rest| !rest.is_empty())?;
    (!key.is_empty()).then(|| format!("{EVENT_FAMILY}/{rest}/{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_compact_state_keeps_only_the_fixed_fields_and_lays_over_older_facts() {
        let facts = json!({
            "state": "open", "draft": false, "head_sha": "abc", "title": "Garden paths",
            "checks_state": "pending", "review_decision": "APPROVED",
            "recent_comments": [{"kind": "comment", "id": 1}],
            "required_checks": {"state": "fail", "source": "rules", "checks": ["build"],
                "failed": (0..30).map(|n| format!("check-{n}")).collect::<Vec<_>>()},
        });
        let compact = compact_item_state(
            "vcs.pull-request", "observer/github/acme/garden", facts.as_object().unwrap(),
        ).unwrap();
        assert_eq!(compact["state"], "open");
        assert_eq!(compact["merge_queue"], Value::Null);
        assert_eq!(compact["required_checks"]["failed"].as_array().unwrap().len(), MAX_FAILED_CHECKS);
        assert!(compact["required_checks"].get("checks").is_none());
        assert!(!compact.contains_key("title") && !compact.contains_key("recent_comments"));
        assert!(serde_json::to_vec(&compact).unwrap().len() <= MAX_ITEM_STATE_BYTES);
        registry().validate_claim("resource/github/acme/garden/pull-request/7", ITEM_STATE, &compact.clone().into_iter().collect()).unwrap();

        let mut older = json!({"state": "open", "title": "Garden paths",
            "merge_queue": {"state": "QUEUED", "position": 2}, "head_sha": "old"});
        let mut merged = compact.clone();
        merged.insert("state".into(), "closed".into());
        merged.insert("merged".into(), true.into());
        overlay_item_state(older.as_object_mut().unwrap(), &merged);
        assert_eq!(older["state"], "closed");
        assert_eq!(older["merged"], true);
        assert_eq!(older["head_sha"], "abc");
        assert_eq!(older["title"], "Garden paths");
        assert!(older.get("merge_queue").is_none());
        let mut checked = json!({"required_checks": {"state": "pending", "checks": ["build"], "execution": {"run": 4}}});
        let mut passing = compact.clone();
        passing.insert("required_checks".into(), json!({"state": "pass", "source": "rules"}));
        overlay_item_state(checked.as_object_mut().unwrap(), &passing);
        assert_eq!(checked["required_checks"], json!({"state": "pass", "source": "rules", "checks": ["build"], "execution": {"run": 4}}),
            "the summary keeps the snapshot's checks and execution");
        assert!(compact_item_state("vcs.repository", "observer/x", &serde_json::Map::new()).is_none());
    }

    #[test]
    fn a_compact_state_reads_as_an_observation_of_its_item() {
        let state = compact_item_state(
            "vcs.pull-request", "observer/github/acme/garden",
            json!({"state": "open", "head_sha": "a", "draft": true}).as_object().unwrap(),
        ).unwrap();
        let body = json!({"fields": state, "evidence": ["claim-1"]});
        let observed = observed_body(ITEM_STATE, "resource/github/acme/garden/pull-request/7", &body);
        assert_eq!(observed["fields"]["kind"], "vcs.pull-request");
        assert_eq!(observed["fields"]["facts"], json!({"state": "open", "head_sha": "a", "draft": true, "number": 7}));
        assert_eq!(observed["evidence"], json!(["claim-1"]));
        let snapshot = json!({"fields": {"kind": "vcs.issue", "facts": {"state": "open"}}});
        assert_eq!(observed_body("resource.observed", "resource/x/issue/1", &snapshot), snapshot);
    }

    #[test]
    fn an_item_state_refuses_snapshot_fields_and_an_event_needs_its_key_family() {
        let mut fields: BTreeMap<String, Value> = compact_item_state(
            "vcs.issue", "observer/github/acme/garden", json!({"state": "closed"}).as_object().unwrap(),
        ).unwrap().into_iter().collect();
        fields.insert("recent_comments".into(), json!([]));
        assert!(registry().validate_claim("resource/github/acme/garden/issue/3", ITEM_STATE, &fields).is_err());

        let subject = event_subject("resource/github/acme/garden/pull-request/7", "checks/abc/fail").unwrap();
        assert_eq!(subject, "github-event/github/acme/garden/pull-request/7/checks/abc/fail");
        let event: BTreeMap<String, Value> = BTreeMap::from([
            ("version".into(), json!(VERSION)),
            ("item".into(), json!("resource/github/acme/garden/pull-request/7")),
            ("event".into(), json!("checks-failed")),
            ("observer".into(), json!("observer/github/acme/garden")),
            ("at".into(), json!("2026-10-30T12:00:00.000Z")),
            ("head_sha".into(), json!("abc")),
        ]);
        registry().validate_claim(&subject, EVENT, &event).unwrap();
        assert!(registry().validate_claim("resource/github/acme/garden/pull-request/7", EVENT, &event).is_err());
        assert!(event_subject("observer/github/acme/garden", "x").is_none());
    }
}
