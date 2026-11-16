//! Verified GitHub webhook deliveries ask the observers of the repository they name to poll now.
//! See `docs/st3/github-observer-admission.md#webhook-delivery`.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::model::{DoctorCheck, ObserverSpec};

/// The receiver's only route; the public proxy forwards this path verbatim.
pub const ROUTE: &str = "/fractal/github-webhook";

/// The prefix of `observer.refresh-requested.attempt` on a refresh a verified delivery requested.
/// Declared refreshes carry a bare hex digest, so the prefix alone marks webhook refreshes.
pub const ATTEMPT_PREFIX: &str = "github-webhook:";

/// How long after a webhook-requested refresh its observer counts as webhook-covered, so its
/// polls are only a backstop.
pub const COVERAGE_MS: u128 = 30 * 60 * 1_000;

/// GitHub's own limit on a delivery's payload.
pub const BODY_LIMIT: usize = 5 * 1024 * 1024;

/// Events that can change what a repository or ref observer reads.
const REFRESHING: &[&str] = &[
    "pull_request",
    "pull_request_review",
    "pull_request_review_comment",
    "pull_request_review_thread",
    "issue_comment",
    "issues",
    "push",
    "check_suite",
    "check_run",
    "status",
    "workflow_run",
    "merge_group",
    "create",
    "delete",
];

/// Events whose deliveries refresh only once their run has finished.
const ON_COMPLETION: &[&str] = &["check_suite", "check_run", "workflow_run"];

/// The metric and doctor outcome of one delivery.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Outcome {
    Refreshed,
    Ignored,
    Unauthorized,
    Unavailable,
    Invalid,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refreshed => "refreshed",
            Self::Ignored => "ignored",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
            Self::Invalid => "invalid",
        }
    }
}

/// A bounded metric label for an unauthenticated event header.
pub fn event_label(event: &str) -> &'static str {
    REFRESHING
        .iter()
        .chain(&["ping"])
        .find(|known| **known == event)
        .copied()
        .unwrap_or("other")
}

/// What one delivery refreshes: the repository's observers and, for a branch it moved, that
/// branch's ref observers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scope {
    /// `owner/repo`, lowercased.
    pub repository: String,
    pub branch: Option<String>,
}

/// The scope of a delivery, `None` for an event that changes nothing an observer reads, or an
/// error for a payload without the repository it concerns.
pub fn scope(event: &str, payload: &Value) -> Result<Option<Scope>, &'static str> {
    if !REFRESHING.contains(&event)
        || (ON_COMPLETION.contains(&event)
            && payload.get("action").and_then(Value::as_str) != Some("completed"))
    {
        return Ok(None);
    }
    let repository = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .filter(|name| name.split_once('/').is_some_and(|(owner, repo)| !owner.is_empty() && !repo.is_empty() && !repo.contains('/')))
        .ok_or("the payload names no repository.full_name")?
        .to_ascii_lowercase();
    let reference = payload.get("ref").and_then(Value::as_str);
    let branch = match event {
        "push" => reference.and_then(|name| name.strip_prefix("refs/heads/")),
        "create" | "delete" => reference
            .filter(|_| payload.get("ref_type").and_then(Value::as_str) == Some("branch")),
        _ => None,
    };
    Ok(Some(Scope {
        repository,
        branch: branch.filter(|branch| !branch.is_empty()).map(str::to_owned),
    }))
}

/// Whether a delivery with `scope` refreshes an observer of `spec`: a repository observer whose
/// locator names the repository in any case, or a ref observer of the branch it moved.
pub fn refreshes(spec: &ObserverSpec, scope: &Scope) -> bool {
    match spec.provider.as_str() {
        "github.repository" => spec.locator.eq_ignore_ascii_case(&scope.repository),
        "github.ref" => scope.branch.as_deref().is_some_and(|branch| {
            spec.locator.rsplit_once('@').is_some_and(|(repository, name)| {
                repository.eq_ignore_ascii_case(&scope.repository) && name == branch
            })
        }),
        _ => false,
    }
}

/// The refresh attempt one delivery asks of one observer; a redelivery asks the same one.
pub fn attempt(delivery: &str, observer: &str) -> String {
    format!(
        "{ATTEMPT_PREFIX}{}",
        hex::encode(Sha256::digest(format!("fractal.github-webhook.v1\0{delivery}\0{observer}")))
    )
}

#[derive(Default)]
struct Receiver {
    listen: Option<String>,
    last_verified_unix_ms: Option<u128>,
    last: Option<Outcome>,
    counts: BTreeMap<Outcome, u64>,
}

fn receiver() -> &'static Mutex<Receiver> {
    static RECEIVER: Mutex<Receiver> = Mutex::new(Receiver {
        listen: None,
        last_verified_unix_ms: None,
        last: None,
        counts: BTreeMap::new(),
    });
    &RECEIVER
}

/// Mark this daemon as running the receiver, so doctor reports it.
pub fn listening(listen: &str) {
    receiver().lock().unwrap_or_else(PoisonError::into_inner).listen = Some(listen.to_owned());
}

/// Count one delivery for doctor. A delivery whose signature verified sets the last verified time.
pub fn record(outcome: Outcome, now_unix_ms: u128) {
    let mut receiver = receiver().lock().unwrap_or_else(PoisonError::into_inner);
    if matches!(outcome, Outcome::Refreshed | Outcome::Ignored) {
        receiver.last_verified_unix_ms = Some(now_unix_ms);
    }
    receiver.last = Some(outcome);
    *receiver.counts.entry(outcome).or_default() += 1;
}

/// The `github-webhook` doctor line, only on a daemon that runs the receiver.
pub fn doctor_line(now_unix_ms: u128) -> Option<DoctorCheck> {
    let receiver = receiver().lock().unwrap_or_else(PoisonError::into_inner);
    let listen = receiver.listen.as_deref()?;
    let last = match receiver.last_verified_unix_ms {
        Some(at) => format!(
            "last verified delivery {} ({}s ago)",
            chrono::DateTime::from_timestamp_millis(i64::try_from(at).unwrap_or(i64::MAX))
                .map_or_else(|| at.to_string(), |time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            now_unix_ms.saturating_sub(at) / 1_000,
        ),
        None => "no verified delivery since start".into(),
    };
    let counts = [Outcome::Refreshed, Outcome::Ignored, Outcome::Unauthorized, Outcome::Unavailable, Outcome::Invalid]
        .map(|outcome| format!("{} {}", outcome.as_str(), receiver.counts.get(&outcome).copied().unwrap_or(0)))
        .join(", ");
    Some(DoctorCheck {
        name: "github-webhook".into(),
        status: if matches!(receiver.last, Some(Outcome::Unauthorized | Outcome::Unavailable)) { "warn" } else { "pass" }.into(),
        message: format!("listening at http://{listen}{ROUTE}; {last}; deliveries since start: {counts}"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn spec(provider: &str, locator: &str) -> ObserverSpec {
        ObserverSpec {
            resource: "resource/github/acme/garden".into(),
            provider: provider.into(),
            locator: locator.into(),
            fields: Vec::new(),
            every_ms: None,
            stopped: false,
        }
    }

    #[test]
    fn only_events_that_change_observed_facts_have_a_scope() {
        let repository = json!({"full_name": "Acme/Garden"});
        let garden = |branch: Option<&str>| Some(Scope { repository: "acme/garden".into(), branch: branch.map(Into::into) });
        assert_eq!(scope("pull_request", &json!({"action": "opened", "repository": repository})), Ok(garden(None)));
        assert_eq!(scope("push", &json!({"ref": "refs/heads/main", "repository": repository})), Ok(garden(Some("main"))));
        assert_eq!(scope("push", &json!({"ref": "refs/tags/v1", "repository": repository})), Ok(garden(None)));
        assert_eq!(scope("create", &json!({"ref": "feature/x", "ref_type": "branch", "repository": repository})), Ok(garden(Some("feature/x"))));
        assert_eq!(scope("delete", &json!({"ref": "v1", "ref_type": "tag", "repository": repository})), Ok(garden(None)));
        for event in ["check_run", "check_suite", "workflow_run"] {
            assert_eq!(scope(event, &json!({"action": "created", "repository": repository})), Ok(None), "{event}");
            assert_eq!(scope(event, &json!({"action": "completed", "repository": repository})), Ok(garden(None)), "{event}");
        }
        for event in ["ping", "star", "fork", "deployment", "package"] {
            assert_eq!(scope(event, &json!({"repository": repository})), Ok(None), "{event}");
        }
        assert!(scope("issues", &json!({"action": "opened"})).is_err());
        assert!(scope("issues", &json!({"repository": {"full_name": "garden"}})).is_err());
        assert_eq!(event_label("check_run"), "check_run");
        assert_eq!(event_label("ping"), "ping");
        assert_eq!(event_label("attacker-chosen"), "other");
    }

    #[test]
    fn repository_observers_match_in_any_case_and_ref_observers_only_their_branch() {
        let main = Scope { repository: "acme/garden".into(), branch: Some("main".into()) };
        assert!(refreshes(&spec("github.repository", "Acme/Garden"), &main));
        assert!(!refreshes(&spec("github.repository", "acme/greenhouse"), &main));
        assert!(refreshes(&spec("github.ref", "ACME/garden@main"), &main));
        assert!(!refreshes(&spec("github.ref", "acme/garden@Main"), &main));
        assert!(!refreshes(&spec("github.ref", "acme/garden@release"), &main));
        let unbranched = Scope { branch: None, ..main };
        assert!(!refreshes(&spec("github.ref", "acme/garden@main"), &unbranched));
        assert!(!refreshes(&spec("command.json", "acme/garden"), &unbranched));
    }

    #[test]
    fn the_attempt_binds_the_delivery_and_the_observer() {
        let attempt_one = attempt("delivery-1", "observer/github/acme/garden");
        assert_eq!(attempt_one, format!(
            "github-webhook:{}",
            hex::encode(Sha256::digest(b"fractal.github-webhook.v1\0delivery-1\0observer/github/acme/garden")),
        ));
        assert_ne!(attempt_one, attempt("delivery-2", "observer/github/acme/garden"));
        assert_ne!(attempt_one, attempt("delivery-1", "observer/garden-ref"));
    }

    #[test]
    fn doctor_reports_the_receiver_once_it_listens() {
        assert!(doctor_line(0).is_none(), "a daemon without a receiver shows no line");
        listening("127.0.0.1:41871");
        record(Outcome::Unauthorized, 1_000);
        let line = doctor_line(2_000).unwrap();
        assert_eq!((line.name.as_str(), line.status.as_str()), ("github-webhook", "warn"));
        assert!(line.message.contains("http://127.0.0.1:41871/fractal/github-webhook; no verified delivery since start"), "{}", line.message);
        record(Outcome::Refreshed, 1_762_000_000_000);
        let line = doctor_line(1_762_000_030_000).unwrap();
        assert_eq!(line.status, "pass");
        assert!(line.message.contains("last verified delivery 2025-11-01T12:26:40Z (30s ago)"), "{}", line.message);
        assert!(line.message.ends_with("refreshed 1, ignored 0, unauthorized 1, unavailable 0, invalid 0"), "{}", line.message);
    }
}
