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
//! on `rule/run-authority`) when the daemon's `[programs] authority` says `audit`. Whatever it is
//! granted, and in either mode, a run touches only itself and the runs it started, transitively.
use super::*;
use crate::config::RunAuthorityMode;
use crate::model::{AuthorityGrant, AuthorityVerb};
use std::sync::atomic::{AtomicBool, Ordering};

/// The audit records' rule name: `rule.audited` on `rule/run-authority`.
pub const AUDIT_RULE: &str = "run-authority";

/// Run starters nest at most this deep; a deeper chain grants nothing. A run reaches its own
/// descendants no deeper.
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

/// The run a run-owned subject belongs to: `mission-run/RUN`, its generations, their step runs
/// and its revision proposals.
fn owning_run_tx(transaction: &Transaction<'_>, subject: &str) -> Result<Option<String>, FractalError> {
    if let Some(run) = subject.strip_prefix("mission-run/") {
        return Ok(Some(run.to_owned()));
    }
    if let Some(proposal) = subject.strip_prefix("revision-proposal/") {
        return transaction.query_row("SELECT run_id FROM revision_proposals WHERE id=?1", [proposal], |row| row.get(0))
            .optional().map_err(internal);
    }
    let generation = subject.strip_prefix("run-generation/")
        .or_else(|| subject.strip_prefix("step-run/").and_then(|rest| rest.split('/').next()));
    let Some(generation) = generation else { return Ok(None) };
    transaction.query_row("SELECT run_id FROM run_generations WHERE id=?1", [generation], |row| row.get(0))
        .optional().map_err(internal)
}

/// The run one claim changes: the run its subject belongs to, or the run a generation or a
/// revision proposal names.
fn touched_run_tx(
    transaction: &Transaction<'_>,
    kind: &str,
    subject: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<Option<String>, FractalError> {
    let field = |name: &str| fields.get(name).and_then(Value::as_str).unwrap_or_default();
    match kind {
        "run-generation.created" | "revision-proposal.created" => owning_run_tx(transaction, field("run")),
        "revision-proposal.applied" => owning_run_tx(transaction, field("successor_generation")),
        _ => owning_run_tx(transaction, subject),
    }
}

/// Whether `target` is `run` or a run it started, transitively: a run's parent is the run of the
/// step it was started under, the tree a cancellation cascades down.
fn own_run_tx(transaction: &Transaction<'_>, run: &str, target: &str) -> Result<bool, FractalError> {
    let mut current = target.to_owned();
    for _ in 0..=MAX_STARTERS {
        if current == run {
            return Ok(true);
        }
        let parent = transaction.query_row(
            "SELECT step.run_id FROM mission_runs child JOIN step_runs step ON step.subject=child.parent_step_run
             WHERE child.id=?1",
            [&current],
            |row| row.get::<_, String>(0),
        ).optional().map_err(internal)?;
        let Some(parent) = parent else { return Ok(false) };
        current = parent;
    }
    Ok(false)
}

/// Refuse `run` touching `target` unless it is the run or one it started, whatever it is granted
/// and in either mode: a `write "mission-run/*"` reaches no sibling, parent or unrelated run.
fn refuse_foreign_run_tx(transaction: &Transaction<'_>, run: &str, target: &str, action: &str) -> Result<(), FractalError> {
    if own_run_tx(transaction, run, target)? {
        return Ok(());
    }
    Err(FractalError::new(
        "run-authority-denied",
        format!("mission-run/{run} may not {action}: mission-run/{target} is neither it nor a run it started"),
    ))
}

/// Admit `actor` acting on `mission-run/RUN` where nothing it writes names the run as its
/// actor: a `mission-run` declaration it applies, whose creation and cancellations are written
/// as the daemon, or a cancellation it requests. A run reaches only itself and the runs it
/// started, and never creates a run this way.
pub fn admit_reach_tx(
    transaction: &Transaction<'_>,
    actor: Option<&str>,
    run_subject: &str,
    action: &str,
) -> Result<(), FractalError> {
    let Some(run) = actor.and_then(|actor| actor.strip_prefix("mission-run/")) else { return Ok(()) };
    let target = run_subject.strip_prefix("mission-run/").unwrap_or(run_subject);
    refuse_foreign_run_tx(transaction, run, target, &format!("{action} `mission-run/{target}`"))
}

/// The grant one claim needs, or none for the run's own journal. A start or a revision writes
/// several claims as one operation; each names the operation's grant on the mission of the run
/// it changes, never a `write`, so the operation's verb alone admits it.
pub(super) fn needed_tx(
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
    let field = |name: &str| fields.get(name).and_then(Value::as_str);
    let grant = |verb, pattern: String| Ok(Some(AuthorityGrant { verb, pattern }));
    let revise = |owner: Option<String>| match owner {
        Some(owner) => grant(AuthorityVerb::Revise, mission_of(&owner)?),
        None => grant(AuthorityVerb::Write, subject.into()),
    };
    match kind {
        // A run always journals its own program steps.
        "program.step" => {
            if owning_run_tx(transaction, subject)?.as_deref() == Some(run) {
                Ok(None)
            } else {
                grant(AuthorityVerb::Write, subject.into())
            }
        }
        "mission.published" => grant(AuthorityVerb::Publish, subject.trim_start_matches("mission/").into()),
        "mission-run.created" => grant(AuthorityVerb::Start, field("mission").unwrap_or_default().trim_start_matches("mission/").into()),
        // Only a revision's generation has a predecessor; its free-text reason decides nothing.
        "run-generation.created" => {
            let verb = if field("predecessor").is_some() { AuthorityVerb::Revise } else { AuthorityVerb::Start };
            grant(verb, mission_of(field("run").unwrap_or_default())?)
        }
        // What a revision writes on the runs it changes: its proposal, carried and terminalized
        // steps, the superseded generation, the run's and its descendants' states.
        "step-run.carried" | "step-run.state" | "run-generation.superseded" | "run-generation.state"
        | "mission-run.state" | "revision-proposal.created" | "revision-proposal.applied" => {
            revise(touched_run_tx(transaction, kind, subject, fields)?)
        }
        _ => grant(AuthorityVerb::Write, subject.into()),
    }
}

/// A run's effective authority, and the first starter in its chain that is not a run. Each level
/// reads only the revision's `authority` member, not the whole mission spec.
fn effective_tx(
    transaction: &Transaction<'_>,
    run: &str,
) -> Result<(Vec<AuthorityGrant>, Option<String>), FractalError> {
    let mut granted: Option<Vec<AuthorityGrant>> = None;
    let mut current = run.to_owned();
    for _ in 0..MAX_STARTERS {
        let Some((declared, requester, starter)) = transaction.query_row(
            "SELECT json_extract(m.body, '$.authority'), r.requester,
                    (SELECT actor FROM claims WHERE subject='mission-run/'||r.id AND kind='mission-run.created' LIMIT 1)
             FROM mission_runs r
             JOIN run_generations g ON g.id=r.current_generation_id
             LEFT JOIN mission_revisions m ON m.mission_id=r.mission_id AND m.revision=g.revision
             WHERE r.id=?1",
            [&current],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
        ).optional().map_err(internal)? else {
            return Ok((Vec::new(), None));
        };
        let declared: Vec<AuthorityGrant> = declared
            .map(|declared| serde_json::from_str(&declared))
            .transpose()
            .map_err(internal)?
            .unwrap_or_default();
        granted = Some(match granted {
            Some(granted) => intersect(&granted, &declared),
            None => declared,
        });
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
    if let Some(target) = touched_run_tx(transaction, kind, subject, fields)? {
        refuse_foreign_run_tx(transaction, run, &target, &format!("write {kind} on `{subject}`"))?;
    }
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
