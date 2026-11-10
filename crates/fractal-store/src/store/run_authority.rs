//! Run authority (docs/st3/program-engine.md §3): what `mission-run/RUN` may write. One decision
//! at the store's write admission, beside the rules gate, for every claim a run writes.
//!
//! A run holds the `authority` of its current generation's revision, intersected with its
//! starter's authority. The starter is the actor of the run's creation: its requester, or the
//! run whose program started it, whose own effective authority is then intersected in turn. A
//! person, agent or daemon starter holds everything in free mode except what the rules restrict,
//! so each write is also decided by the rules for that starter.
//!
//! A write outside the authority is refused (`run-authority-denied`), or audited (`rule.audited`
//! on `rule/run-authority`) when the daemon's `[programs] authority` says `audit`.
use super::*;
use crate::config::RunAuthorityMode;
use crate::model::{AuthorityGrant, AuthorityVerb};
use std::sync::atomic::{AtomicBool, Ordering};

/// The audit records' rule name: `rule.audited` on `rule/run-authority`.
pub const AUDIT_RULE: &str = "run-authority";

/// Run starters nest at most this deep; a deeper chain grants nothing.
const MAX_STARTERS: usize = 32;

/// Enforced until the daemon's config says otherwise, as `[programs] authority` defaults.
static ENFORCE: AtomicBool = AtomicBool::new(true);

/// Set what a write outside a run's authority does, from the daemon's `[programs] authority`.
pub fn set_run_authority_mode(mode: RunAuthorityMode) {
    ENFORCE.store(mode == RunAuthorityMode::Enforce, Ordering::Relaxed);
}

pub fn run_authority_mode() -> RunAuthorityMode {
    if ENFORCE.load(Ordering::Relaxed) { RunAuthorityMode::Enforce } else { RunAuthorityMode::Audit }
}

/// Whether `pattern` covers `target`, itself exact or a terminal `/*` namespace.
pub fn covers(pattern: &str, target: &str) -> bool {
    pattern == target
        || pattern.strip_suffix("/*").is_some_and(|namespace| {
            target.strip_prefix(namespace).is_some_and(|rest| rest.starts_with('/'))
        })
}

/// What both rule sets grant. With exact-or-namespace patterns this is exact: for each pair with
/// one verb, the narrower pattern when one covers the other.
pub fn intersect(left: &[AuthorityGrant], right: &[AuthorityGrant]) -> Vec<AuthorityGrant> {
    let mut both = BTreeSet::new();
    for one in left {
        for other in right.iter().filter(|other| other.verb == one.verb) {
            if covers(&one.pattern, &other.pattern) {
                both.insert(other.clone());
            } else if covers(&other.pattern, &one.pattern) {
                both.insert(one.clone());
            }
        }
    }
    both.into_iter().collect()
}

/// The grant one claim needs, or none for the run's own journal.
fn needed_tx(
    transaction: &Transaction<'_>,
    run: &str,
    kind: &str,
    subject: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<Option<AuthorityGrant>, FractalError> {
    let mission_of = |run: &str| -> Result<String, FractalError> {
        transaction.query_row(
            "SELECT mission_id FROM mission_runs WHERE id=?1",
            [run.trim_start_matches("mission-run/")],
            |row| row.get::<_, String>(0),
        ).optional().map_err(internal)?.ok_or_else(|| FractalError::new(
            "run-authority-denied", format!("{run} is not a mission run this host holds"),
        ))
    };
    let field = |name: &str| fields.get(name).and_then(Value::as_str).unwrap_or_default();
    let grant = |verb, pattern: String| Ok(Some(AuthorityGrant { verb, pattern }));
    match kind {
        // A run always journals its own program steps.
        "program.step" => {
            let generation = subject.strip_prefix("step-run/").and_then(|rest| rest.split('/').next());
            let owner: Option<String> = match generation {
                Some(generation) => transaction.query_row(
                    "SELECT run_id FROM run_generations WHERE id=?1", [generation], |row| row.get(0),
                ).optional().map_err(internal)?,
                None => None,
            };
            if owner.as_deref() == Some(run) { Ok(None) } else { grant(AuthorityVerb::Write, subject.into()) }
        }
        "mission.published" => grant(AuthorityVerb::Publish, subject.trim_start_matches("mission/").into()),
        "mission-run.created" => grant(AuthorityVerb::Start, field("mission").trim_start_matches("mission/").into()),
        "run-generation.created" => {
            let verb = if field("reason") == "initial mission run" { AuthorityVerb::Start } else { AuthorityVerb::Revise };
            grant(verb, mission_of(field("run"))?)
        }
        _ => grant(AuthorityVerb::Write, subject.into()),
    }
}

/// A run's effective authority, and the first starter in its chain that is not a run.
fn effective_tx(
    transaction: &Transaction<'_>,
    run: &str,
) -> Result<(Vec<AuthorityGrant>, Option<String>), FractalError> {
    let mut granted: Option<Vec<AuthorityGrant>> = None;
    let mut current = run.to_owned();
    for _ in 0..MAX_STARTERS {
        let Some((mission, revision, requester)) = transaction.query_row(
            "SELECT r.mission_id, g.revision, r.requester FROM mission_runs r
             JOIN run_generations g ON g.id=r.current_generation_id WHERE r.id=?1",
            [&current],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(internal)? else {
            return Ok((Vec::new(), None));
        };
        let declared = super::missions::mission_revision_spec_tx(transaction, &mission, &revision)?
            .map(|mission| mission.authority)
            .unwrap_or_default();
        granted = Some(match granted {
            Some(granted) => intersect(&granted, &declared),
            None => declared,
        });
        let starter: Option<String> = transaction.query_row(
            "SELECT actor FROM claims WHERE subject=?1 AND kind='mission-run.created' LIMIT 1",
            [format!("mission-run/{current}")],
            |row| row.get(0),
        ).optional().map_err(internal)?.flatten();
        let starter = starter.unwrap_or(requester);
        match starter.strip_prefix("mission-run/") {
            Some(parent) => current = parent.to_owned(),
            None => return Ok((granted.unwrap_or_default(), Some(starter))),
        }
    }
    Ok((Vec::new(), None))
}

/// Admit one claim written as `actor`: a claim a run writes outside its effective authority is
/// audited or refused. Every other writer passes untouched.
pub(super) fn admit_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    actor: Option<&str>,
    kind: &str,
    subject: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<(), FractalError> {
    let Some(actor) = actor.filter(|actor| actor.starts_with("mission-run/")) else { return Ok(()) };
    let run = actor.trim_start_matches("mission-run/");
    let Some(needed) = needed_tx(transaction, run, kind, subject, fields)? else { return Ok(()) };
    let (granted, starter) = effective_tx(transaction, run)?;
    if let Some(starter) = &starter {
        smallclaims::store::principals::rules_gate_tx(transaction, origin, starter, kind, subject)
            .map_err(smallclaims::error::typed)?;
    }
    if granted.iter().any(|grant| grant.verb == needed.verb && covers(&grant.pattern, &needed.pattern)) {
        return Ok(());
    }
    match run_authority_mode() {
        RunAuthorityMode::Enforce => Err(FractalError::new(
            "run-authority-denied",
            format!(
                "{actor} may not {} `{}` ({kind} on {subject}): its mission's authority, intersected with its starter's, does not grant it",
                needed.verb.as_str(), needed.pattern,
            ),
        )),
        RunAuthorityMode::Audit => {
            append_claim_tx(
                transaction, origin, &format!("rule/{AUDIT_RULE}"), smallclaims::rules::RULE_AUDITED, None,
                &json!({"fields": {"rule": AUDIT_RULE, "actor": actor, "action": kind, "target": subject}, "evidence": []}),
                &[], None,
            ).map_err(internal)?;
            Ok(())
        }
    }
}
