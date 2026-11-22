//! Person asks are runtime steps. The immutable asking claim supplies their ownership fence;
//! step_runs supplies their state. No separate attention record is created.
use super::*;
use crate::model::{PersonAskRequest, PersonRenameRequest};

use crate::person_request::StructuredRequest;

#[cfg(test)]
mod reconcile_candidates_tests;
#[cfg(test)]
mod revision_provenance_tests;
pub mod continuity;
pub use continuity::{AskContinuityBinding, continuity_step_binding_tx, step_answers_bound_tx, step_answers_eligible_tx};

/// The structured request an ask claim carries. An ask from a newer build whose request this
/// build cannot read is refused rather than answered as free text.
pub fn structured_request(ask: &ClaimRecord) -> Result<Option<StructuredRequest>, FractalError> {
    let Some(request) = ask.body["fields"].get("request").filter(|r| !r.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value::<StructuredRequest>(request.clone())
        .ok()
        .filter(|request| request.version == crate::person_request::REQUEST_VERSION)
        .map(Some)
        .ok_or_else(|| {
            FractalError::new(
                "unsupported-person-request",
                "this ask's request needs a newer st to answer",
            )
        })
}

/// How long a person has to answer an ask that names no deadline, unless `[asks]
/// default_deadline` says otherwise.
pub const DEFAULT_ASK_DEADLINE_MS: u64 = 24 * 60 * 60 * 1000;

/// The `meta` key holding when this node first counted ask deadlines, the first reconcile pass
/// after `[asks] deadlines` was turned on. An ask made before then carries none, so its deadline
/// counts from then rather than from when it was asked.
pub const ASK_DEADLINES_SINCE: &str = "person_ask_deadlines_since_unix_ms";

/// When this node started counting ask deadlines; zero before it has.
pub fn ask_deadlines_since(connection: &Connection) -> Result<u128> {
    Ok(connection
        .query_row("SELECT value FROM meta WHERE key=?1", [ASK_DEADLINES_SINCE], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0))
}

/// The `meta` key holding this node's `[asks] default_deadline`, so that transaction code, such
/// as a person's home, reads the same default the reconciler resolves asks with.
pub const ASK_DEFAULT_DEADLINE: &str = "person_ask_default_deadline_ms";

/// When this node started counting ask deadlines, and its default deadline.
pub fn ask_deadline_policy(connection: &Connection) -> Result<(u128, u64)> {
    let default = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [ASK_DEFAULT_DEADLINE],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ASK_DEADLINE_MS);
    Ok((ask_deadlines_since(connection)?, default))
}

/// Records, once, that this node counts ask deadlines from `now`. Returns whether it had not.
pub fn start_counting_ask_deadlines(connection: &Connection, now: u128) -> Result<bool> {
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h3beacc1959d7 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,?2)",
        params![ASK_DEADLINES_SINCE, now.to_string()],
    )? > 0)
}

/// Forgets when this node started counting, on a pass that sees deadlines off: an ask made
/// while they were off then counts from when they are next turned on, not from the first time.
/// Returns whether it had been counting.
pub fn stop_counting_ask_deadlines(connection: &Connection) -> Result<bool> {
    Ok(connection.execute(/* sqlite-large-value: allow-for-now lv-hdb4786de6904 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [ASK_DEADLINES_SINCE])? > 0)
}

/// Which of an ask's responses stand once a deadline has applied its default. Members resolve
/// independently, so a person may answer on another member before the deadline while the
/// ask's own member applies the default: a person's answer accepted before the cutoff (the
/// ask's deadline with the person's deadline clock applied, [`answer_cutoff`], or when the
/// default was applied for an ask made without one) wins, and one accepted at or after it does
/// not. Every member holds the same claims with the same accept times, so every member keeps
/// the same responses. A validated explicit override wins even after the cutoff; the last
/// one in canonical order stands. Without a deadline response ordinary responses all stand.
pub fn standing_responses<T>(
    responses: Vec<T>,
    by_deadline: impl Fn(&T) -> bool,
    is_person_answer: impl Fn(&T) -> bool,
    is_override: impl Fn(&T) -> bool,
    accepted_at: impl Fn(&T) -> u128,
    deadline: Option<u128>,
) -> (Vec<T>, bool) {
    if let Some(latest) = responses.iter().rposition(&is_override) {
        return (responses.into_iter().enumerate().filter_map(|(index, response)| {
            (index == latest || (!by_deadline(&response) && !is_person_answer(&response) && !is_override(&response)))
                .then_some(response)
        }).collect(), true);
    }
    let Some(applied) = responses.iter().find(|response| by_deadline(response)) else {
        return (responses, false);
    };
    let cutoff = deadline.unwrap_or_else(|| accepted_at(applied));
    let answered_in_time = responses
        .iter()
        .any(|response| is_person_answer(response) && accepted_at(response) < cutoff);
    let standing = responses
        .into_iter()
        .filter(|response| {
            if by_deadline(response) {
                !answered_in_time
            } else {
                !is_person_answer(response) || accepted_at(response) < cutoff
            }
        })
        .collect();
    (standing, answered_in_time)
}

/// `deadline`, which `ask` counts toward from `asked_at`, once the asked person's deadline clock
/// applies: plus every interval the person held the clock since (`hold-all` holds every ask,
/// `hold-routine` only one that is not urgent). `None` while a hold that applies is open with no
/// end, so nothing resolves the ask. A pairing request is a capability request: it always
/// expires on time.
pub fn clocked_deadline(connection: &Connection, ask: &ClaimRecord, asked_at: u128, deadline: u128) -> rusqlite::Result<Option<u128>> {
    let fields = &ask.body["fields"];
    let Some(person) = fields["person"].as_str().filter(|_| ask.actor.as_deref() != Some("daemon/browser-pairing")) else {
        return Ok(Some(deadline));
    };
    let holds = super::availability::deadline_clock_holds_in(connection, person, asked_at)?;
    Ok(super::availability::effective_deadline(asked_at, deadline, super::availability::urgent_ask(fields), &holds))
}

/// When an ask resolves by itself if the person has not answered: the deadline it was asked
/// with, or, for an ask made before deadlines, `default_ms` after it was asked or after this
/// node began counting (`since`), whichever is later; then moved past the time the asked
/// person held their deadline clock ([`clocked_deadline`]). `None` for an update, which waits
/// for nobody, and while the clock is held.
pub fn effective_ask_deadline(connection: &Connection, ask: &ClaimRecord, since: u128, default_ms: u64) -> rusqlite::Result<Option<u128>> {
    if is_update(ask) {
        return Ok(None);
    }
    let (asked_at, deadline) = match ask.body["fields"]["deadline_unix_ms"].as_u64() {
        Some(deadline) => (ask.accepted_at_unix_ms, u128::from(deadline)),
        None => {
            let from = ask.accepted_at_unix_ms.max(since);
            (from, from + u128::from(default_ms))
        }
    };
    clocked_deadline(connection, ask, asked_at, deadline)
}

/// The time before which a person's answer to `ask` beats a default its deadline applied
/// (`standing_responses`): its stored deadline with the person's deadline clock applied.
/// `None` for an ask asked without one, or one still held: when the default was applied decides.
pub fn answer_cutoff(connection: &Connection, ask: &ClaimRecord) -> rusqlite::Result<Option<u128>> {
    match ask.body["fields"]["deadline_unix_ms"].as_u64() {
        Some(deadline) => clocked_deadline(connection, ask, ask.accepted_at_unix_ms, u128::from(deadline)),
        None => Ok(None),
    }
}

/// The answer id that applies at an ask's deadline, if it names one.
pub fn ask_default_answer(ask: &ClaimRecord) -> Option<&str> {
    ask.body["fields"]["default_answer"].as_str()
}

impl Store {
    /// Sets how long a person has to answer an ask that names no deadline. It is kept in `meta`
    /// too, written only when it changes, for the asks that carry no deadline of their own.
    pub fn set_default_ask_deadline_ms(&self, deadline_ms: u64) -> Result<()> {
        self.default_ask_deadline_ms
            .store(deadline_ms, std::sync::atomic::Ordering::Relaxed);
        if ask_deadline_policy(&self.readers.get())?.1 == deadline_ms {
            return Ok(());
        }
        self.connection
            .batched(|tx| {
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hcb7506bd8ba2 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
                    params![ASK_DEFAULT_DEADLINE, deadline_ms.to_string()],
                )
                .map(drop)
            })
            .map_err(anyhow::Error::msg)??;
        self.ask_deadline_policy_changed();
        Ok(())
    }

    /// The ask deadline policy in `meta` changed. A person's home shows deadlines by it, and
    /// no claim tells the attention list refresher, so wake it: its next refresh sees the
    /// publication's policy differ and folds again.
    pub fn ask_deadline_policy_changed(&self) {
        if let Some(wake) = self.fractal.attention_list.refresher.get() {
            wake.notify_one();
        }
    }

    pub fn default_ask_deadline_ms(&self) -> u64 {
        self.default_ask_deadline_ms
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turns ask deadlines on or off (`[asks] deadlines`).
    pub fn set_ask_deadlines(&self, enabled: bool) {
        self.ask_deadlines
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn ask_deadlines(&self) -> bool {
        self.ask_deadlines.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub fn validate_delegated_answer(
    ask: &ClaimRecord,
    fields: &BTreeMap<String, Value>,
) -> Result<(), FractalError> {
    if fields.get("status").and_then(Value::as_str) != Some("completed")
        || fields
            .get("summary")
            .and_then(Value::as_str)
            .is_none_or(|summary| summary.trim().is_empty())
    {
        return Err(FractalError::new(
            "delegation-refused",
            "a completion needs completed status and a summary",
        ));
    }
    if let Some(request) = structured_request(ask)? {
        let submitted = fields.get("answer").ok_or_else(|| {
            FractalError::new("delegation-refused", "the structured answer is missing")
        })?;
        let input = crate::person_request::AnswerInput {
            id: submitted
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            text: submitted
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned),
            option_notes: submitted
                .get("option_notes")
                .map(|notes| serde_json::from_value(notes.clone()))
                .transpose()
                .map_err(|_| FractalError::new(
                    "delegation-refused",
                    "the answer contains invalid option notes",
                ))?
                .unwrap_or_default(),
        };
        if request.answer(Some(&input), fields["summary"].as_str().unwrap())? != *submitted {
            return Err(FractalError::new(
                "delegation-refused",
                "the answer does not match the offered structured answer",
            ));
        }
        // Generic claim validation cannot rewrite the submitted fields. Require the same
        // text-only history that the person-answer API appends before persisting a claim.
        let mut notes_summary = String::new();
        crate::person_request::append_option_notes_summary(&mut notes_summary, submitted);
        if !fields["summary"]
            .as_str()
            .unwrap()
            .contains(notes_summary.trim_start())
        {
            return Err(FractalError::new(
                "delegation-refused",
                "the summary must include every option note in answer history format",
            ));
        }
    } else if fields.contains_key("answer") {
        return Err(FractalError::new(
            "delegation-refused",
            "a free-text ask has no named answer",
        ));
    }
    Ok(())
}

const STEP_QUERY: &str = "SELECT subject, run_id, step_path, definition_hash, status, attempt,
 assignee, available_to, agentless, title, goals, worker_reported, lease_owner,
 lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms,
 created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
 FROM step_runs WHERE subject=?1";

pub fn step(connection: &Connection, subject: &str) -> Result<Option<StepRunView>> {
    smallclaims::touched::note_read(|| subject.to_owned());
    memoized(connection, |memo| &mut memo.steps, subject.to_owned(), || Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h77019adc789b owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ STEP_QUERY)?.query_row( /* sqlite-large-value: allow-for-now lv-hfc8b121073e7 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [subject], step_run_from_row)
        .optional()?))
}

/// What one fault snapshot reads again for every failure under the same run, step or
/// declaration: thousands of failures share a few hundred runs. Each answer holds only for the
/// read cut it came from, so the memo lives exactly as long as [`with_cut_memo`], and answers
/// only reads on the connection that holds that cut.
#[derive(Default)]
struct CutMemo {
    connection: Option<usize>,
    liveness: HashMap<LivenessKey, Remembered<std::result::Result<(), String>>>,
    declarations: HashMap<String, Remembered<bool>>,
    headers: HashMap<String, Remembered<Option<MissionRunView>>>,
    steps: HashMap<String, Remembered<Option<StepRunView>>>,
    desired: HashMap<String, Remembered<Option<DesiredRow>>>,
    claims: HashMap<String, Remembered<Option<ClaimRecord>>>,
    extensions: HashMap<(String, u32, u128), Remembered<u64>>,
}

/// A run, the generation it must be on, and whether a failure keeps it live.
type LivenessKey = (String, Option<String>, bool);

/// An answer, and the keys reading it noted (`smallclaims::touched`).
type Remembered<V> = (V, BTreeSet<String>);

thread_local! {
    static CUT_MEMO: std::cell::RefCell<Option<CutMemo>> = const { std::cell::RefCell::new(None) };
}

/// Run `read` with repeated run, step, declaration and claim reads answered once. The caller
/// must hold one read cut for all of `read` (`read_snapshot`) and write nothing inside it.
pub fn with_cut_memo<T>(read: impl FnOnce() -> T) -> T {
    struct Scope(bool);
    impl Drop for Scope {
        fn drop(&mut self) {
            if self.0 {
                CUT_MEMO.with(|memo| memo.borrow_mut().take());
            }
        }
    }
    let _scope = Scope(CUT_MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        let outermost = memo.is_none();
        if outermost {
            *memo = Some(CutMemo::default());
        }
        outermost
    }));
    read()
}

fn memoized<K: Eq + std::hash::Hash, V: Clone>(
    connection: &Connection,
    table: fn(&mut CutMemo) -> &mut HashMap<K, Remembered<V>>,
    key: K,
    read: impl FnOnce() -> Result<V>,
) -> Result<V> {
    // The first read names the cut's connection. A read on another one, such as a writer's
    // transaction, is not in the cut.
    let at = std::ptr::from_ref(connection) as usize;
    let found = CUT_MEMO.with(|memo| {
        in_cut(&mut memo.borrow_mut(), at).and_then(|memo| table(memo).get(&key).cloned())
    });
    // A remembered answer notes again what reading it noted, so a caller that records its
    // reads sees the same keys either way.
    if let Some((found, reads)) = found {
        for read in reads {
            smallclaims::touched::note_read(|| read);
        }
        return Ok(found);
    }
    let (value, reads) = smallclaims::touched::record(read);
    let value = value?;
    CUT_MEMO.with(|memo| {
        if let Some(memo) = in_cut(&mut memo.borrow_mut(), at) {
            table(memo).insert(key, (value.clone(), reads));
        }
    });
    Ok(value)
}

fn in_cut(memo: &mut Option<CutMemo>, at: usize) -> Option<&mut CutMemo> {
    let memo = memo.as_mut()?;
    (*memo.connection.get_or_insert(at) == at).then_some(memo)
}

/// The claim `id`, read once per cut inside [`with_cut_memo`].
pub fn claim_in_cut(
    connection: &Connection,
    id: &str,
    read: impl FnOnce() -> Result<Option<ClaimRecord>>,
) -> Result<Option<ClaimRecord>> {
    memoized(connection, |memo| &mut memo.claims, id.to_owned(), read)
}

/// [`step_timeout_extension_at`], read once per cut inside [`with_cut_memo`].
pub fn step_timeout_extension_in_cut(
    connection: &Connection,
    subject: &str,
    attempt: u32,
    as_of: u128,
) -> Result<u64> {
    smallclaims::touched::note_read(|| subject.to_owned());
    memoized(connection, |memo| &mut memo.extensions, (subject.to_owned(), attempt, as_of), || {
        Ok(step_timeout_extension_at(connection, subject, attempt, as_of)?)
    })
}

fn run_header(connection: &Connection, run: &str) -> Result<Option<MissionRunView>> {
    smallclaims::touched::note_read(|| format!("mission-run/{run}"));
    smallclaims::touched::note_read(|| format!("generations:mission-run/{run}"));
    memoized(connection, |memo| &mut memo.headers, run.to_owned(), || {
        Ok(mission_run_header_tx(connection, run).optional()?)
    })
}

fn desired_row(connection: &Connection, subject: &str) -> Result<Option<DesiredRow>> {
    smallclaims::touched::note_read(|| subject.to_owned());
    memoized(connection, |memo| &mut memo.desired, subject.to_owned(), || current_desired_row(connection, subject))
}

pub fn request(connection: &Connection, subject: &str) -> Result<Option<ClaimRecord>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| canonical_sql(
        "SELECT id, store_index, batch_id, subject, kind,
         origin, actor, body, predecessors, accepted_at_unix_ms FROM claims
         WHERE subject=?1 AND kind='work.person-asked' ORDER BY CANONICAL_ASC(claims) LIMIT 1",
    ));
    Ok(connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h7ba5b1a68d8c owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &QUERY,
        )?.query_row(
            /* sqlite-large-value: allow-for-now lv-hc73724d99cfd owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [subject],
            claim_from_row,
        )
        .optional()?)
}

pub fn declaration_live(connection: &Connection, subject: &str) -> Result<bool> {
    memoized(connection, |memo| &mut memo.declarations, subject.to_owned(), || declaration_live_uncached(connection, subject))
}

fn declaration_live_uncached(connection: &Connection, subject: &str) -> Result<bool> {
    let Some(row) = desired_row(connection, subject)? else { return Ok(false) };
    if !declaration_present(&row)? {
        return Ok(false);
    }
    if let Some(run) = row.owner_run.as_deref() {
        if !run_live(connection, run, row.owner_generation.as_deref(), false)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A declared requester still exists even when its owning work has ended.
pub fn requester_declared(connection: &Connection, subject: &str) -> Result<bool> {
    let Some(row) = desired_row(connection, subject)? else { return Ok(false) };
    declaration_present(&row)
}

fn declaration_present(row: &DesiredRow) -> Result<bool> {
    let body: Value = serde_json::from_str(&row.body)?;
    if row.kind == "stop"
        || body
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop")
    {
        return Ok(false);
    }
    Ok(true)
}

pub fn run_live(
    connection: &Connection,
    run: &str,
    generation: Option<&str>,
    failure: bool,
) -> Result<bool> {
    Ok(run_liveness(connection, run, generation, failure)?.is_ok())
}

/// Whether `run` still matters, or why not: it and each run above it exist, are open and on
/// the generation their parent step belongs to, each parent step is still open, and a
/// subscription or schedule that delivered it still runs. `failure` keeps a failed run or
/// parent step live, for the fault that names it.
pub fn run_liveness(
    connection: &Connection,
    run: &str,
    generation: Option<&str>,
    failure: bool,
) -> Result<std::result::Result<(), String>> {
    memoized(
        connection,
        |memo| &mut memo.liveness,
        (run.to_owned(), generation.map(str::to_owned), failure),
        || run_liveness_uncached(connection, run, generation, failure),
    )
}

fn run_liveness_uncached(
    connection: &Connection,
    run: &str,
    generation: Option<&str>,
    failure: bool,
) -> Result<std::result::Result<(), String>> {
    let mut run = run.strip_prefix("mission-run/").unwrap_or(run).to_owned();
    let mut expected_generation = generation.map(str::to_owned);
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(run.clone()) {
            return Ok(Err(format!(
                "mission run `mission-run/{run}` is its own ancestor"
            )));
        }
        let header = run_header(connection, &run)?;
        let Some(header) = header else {
            return Ok(Err(format!("mission run `mission-run/{run}` is gone")));
        };
        if let Some(expected) = expected_generation
            .as_ref()
            .filter(|expected| *expected != &header.generation)
        {
            return Ok(Err(format!(
                "mission run `{}` moved on from {expected} to {}",
                header.subject, header.generation
            )));
        }
        if is_terminal_run_state(&header.status) && !(failure && header.status == "failed") {
            return Ok(Err(format!(
                "mission run `{}` is {}",
                header.subject, header.status
            )));
        }
        if let Some(parent) = header.parent_step_run.as_deref() {
            let Some(parent) = step(connection, parent)? else {
                // Subscription and schedule deliveries use their declaration as the parent,
                // normalized with a step-run prefix. They do not create a synthetic work step.
                let subject = parent.strip_prefix("step-run/").unwrap_or(parent);
                let Some(owner) = desired_row(connection, subject)? else {
                    return Ok(Err(format!("`{subject}`, which started it, is gone")));
                };
                let body: Value = serde_json::from_str(&owner.body)?;
                if !matches!(owner.kind.as_str(), "subscription" | "schedule") {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, is not a step, subscription or schedule"
                    )));
                }
                if body
                    .get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop")
                {
                    return Ok(Err(format!("`{subject}`, which started it, is stopped")));
                }
                let Some(owner_run) = owner.owner_run else {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, has no owning run"
                    )));
                };
                let owner_header = run_header(connection, owner_run.trim_start_matches("mission-run/"))?;
                if owner_header
                    .is_none_or(|owner| owner.root_mission_run != header.root_mission_run)
                {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, now belongs to another run"
                    )));
                }
                run = owner_run.trim_start_matches("mission-run/").into();
                expected_generation = owner.owner_generation;
                continue;
            };
            if matches!(parent.status.as_str(), "completed" | "cancelled")
                || (parent.status == "failed" && !failure)
            {
                return Ok(Err(format!(
                    "its parent step `{}` is {}",
                    parent.subject, parent.status
                )));
            }
            run = parent.run.trim_start_matches("mission-run/").into();
            expected_generation = Some(parent.generation);
        } else {
            let root = run_header(connection, header.root_mission_run.trim_start_matches("mission-run/"))?;
            return Ok(match root {
                None => Err(format!(
                    "its root mission run `{}` is gone",
                    header.root_mission_run
                )),
                Some(root)
                    if is_terminal_run_state(&root.status)
                        && !(failure && root.status == "failed") =>
                {
                    Err(format!(
                        "its root mission run `{}` is {}",
                        root.subject, root.status
                    ))
                }
                Some(_) => Ok(()),
            });
        }
    }
}

pub fn episode_owner(connection: &Connection, ask: &ClaimRecord) -> Result<Option<(String, Option<String>)>> {
    let fields = &ask.body["fields"];
    if fields.get("owner_run").is_some() {
        return Ok(fields["owner_run"].as_str().map(|run| (
            run.to_owned(), fields["owner_generation"].as_str().map(str::to_owned),
        )));
    }
    let requester = ask.actor.as_deref().unwrap_or_default();
    if !requester.starts_with("agent/") {
        return Ok(None);
    }
    Ok(canonical_desired_row_at_claim(connection, requester, &ask.id)?.and_then(|desired|
        desired.owner_run.map(|run| (run, desired.owner_generation))))
}

pub fn current(connection: &Connection, ask: &ClaimRecord, as_of: u128) -> Result<bool> {
    if ask.accepted_at_unix_ms > as_of {
        return Ok(false);
    }
    let fields = &ask.body["fields"];
    // Pairing is a five-minute capability request even when ordinary ask deadlines are off.
    if ask.actor.as_deref() == Some("daemon/browser-pairing")
        && fields["deadline_unix_ms"].as_u64().is_none_or(|deadline| u128::from(deadline) <= as_of)
    {
        return Ok(false);
    }
    let Some(view) = step(connection, &ask.subject)? else {
        return Ok(false);
    };
    // Resolution claims are terminal authority, even if a stale projection looks open.
    let resolved: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1
         AND kind IN ('work.person-done','work.person-cancelled')
         AND CAST(accepted_at_unix_ms AS INTEGER)<=?2)",
        params![ask.subject, as_of as i64], |row| row.get(0))?;
    if resolved {
        return Ok(false);
    }
    let requester = ask.actor.as_deref().unwrap_or_default();
    // Unowned asks survive runtime turns; an episode-owned ask keeps its original fence.
    if let Some((owner, generation)) = episode_owner(connection, ask)?
        && !run_live(connection, &owner, generation.as_deref(), false)?
    {
        return Ok(false);
    }
    if is_update(ask) {
        let about = fields["request"]["about"].as_str().unwrap_or_default();
        if about.starts_with("mission-run/") && !run_live(connection, about, None, false)? {
            return Ok(false);
        }
        if about.starts_with("step-run/")
            && let Some(origin) = step(connection, about)?
            && (matches!(origin.status.as_str(), "completed" | "cancelled" | "failed")
                || !run_live(connection, &origin.run, Some(&origin.generation), false)?)
        {
            return Ok(false);
        }
    }
    let independent = view.run.starts_with("mission-run/person-ask/") && fields["origin_step"].is_null();
    // Updates need no live poster, but their owning episode still fences them.
    let unfenced = requester.starts_with("daemon/") || is_update(ask);
    if !matches!(view.status.as_str(), "pending" | "ready")
        || (!independent && !run_live(connection, &view.run, Some(&view.generation), false)?)
    {
        return Ok(false);
    }
    let requester_live = unfenced || if independent {
        requester_declared(connection, requester)?
    } else {
        declaration_live(connection, requester)?
    };
    let retiring = !independent && !requester_live && super::rollouts::retiring_ask_live(connection, ask)?;
    if !requester_live && !retiring {
        return Ok(false);
    }
    if unfenced {
        return Ok(true);
    }
    let mut declarations = connection.prepare(/* sqlite-large-value: allow-for-now lv-hbc46e8c53f31 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='intent.desired' ORDER BY CANONICAL_ASC(claims)"))?;
    let ask_key = canonical::claim_key(connection, &ask.id)?;
    for declaration in declarations.query_map([requester], claim_from_row)? {
        let declaration = declaration?;
        if canonical::claim_key(connection, &declaration.id)? <= ask_key {
            continue;
        }
        if !retiring
            && (declaration.body["kind"] == "stop"
                || declaration.body["desired"]
                    .get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop"))
        {
            return Ok(false);
        }
    }
    if independent {
        return Ok(true);
    }
    // A top-level seat's ask outlives its restarts and harness changes: only stopping or retiring
    // the seat (above) or cancelling the ask ends it, so a person never loses it to a restart. A
    // mission's seat lives and dies with its declaration, and so does its ask.
    if let Some(declaration) = fields["requester_declaration"].as_str()
        && !fields["owner_run"].is_null()
        && current_desired_row(connection, requester)?.is_none_or(|row| row.claim_id != declaration)
    {
        return Ok(false);
    }
    if let Some(owner) = fields["owner_run"].as_str() {
        if !run_live(
            connection,
            owner,
            fields["owner_generation"].as_str(),
            false,
        )? {
            return Ok(false);
        }
    }
    if let Some(origin) = fields["origin_step"].as_str() {
        let Some(origin) = step(connection, origin)? else {
            return Ok(false);
        };
        if origin.attempt as u64 != fields["origin_attempt"].as_u64().unwrap_or(0)
            || origin.status != "waiting-person"
            || origin.blocked_reason.as_deref() != Some(ask.subject.as_str())
            || origin.generation != view.generation
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn is_update(ask: &ClaimRecord) -> bool {
    ask.body["fields"]["request"]["type"] == "update"
}

/// Adds person responses to a step view: the responses to asks this attempt made, as
/// constraints and as data, or a person ask's own response as data.
/// Seek ask origins through the durable expression index before reading resolution history;
/// the source subjects also admit an ask's own responses, including asks without an origin.
pub fn enrich_responses(
    connection: &Connection,
    view: &mut StepRunView,
) -> rusqlite::Result<()> {
    enrich_responses_with_sql(connection, view, RESPONSES_SQL)
}

const RESPONSES_SQL: &str = "WITH RECURSIVE sources(subject,attempt) AS (
          SELECT ?1,?2 UNION
          SELECT json_extract(carried.body,'$.fields.source_step_run'),
            coalesce(json_extract(carried.body,'$.fields.source_attempt'),sources.attempt)
          FROM sources CROSS JOIN claims carried INDEXED BY claims_subject_kind_index
          ON carried.subject=sources.subject
            AND carried.kind='step-run.carried'
            AND json_extract(carried.body,'$.fields.attempt')=sources.attempt
        ), candidates(subject) AS (
          SELECT subject FROM sources
          UNION
          SELECT request.subject FROM sources
          CROSS JOIN claims request INDEXED BY claims_person_ask_origin_index
          WHERE request.kind='work.person-asked'
            AND json_extract(request.body,'$.fields.origin_step')=sources.subject
            AND json_extract(request.body,'$.fields.origin_attempt')=sources.attempt
        )
        SELECT resolution.subject, resolution.actor, resolution.body, resolution.accepted_at_unix_ms,
          EXISTS(SELECT 1 FROM sources WHERE sources.subject=json_extract(request.body,'$.fields.origin_step')), resolution.kind,
          json_extract(request.body,'$.fields.deadline_unix_ms')
        FROM candidates CROSS JOIN claims resolution INDEXED BY claims_subject_kind_index
        LEFT JOIN claims request
        ON request.subject=resolution.subject AND request.kind='work.person-asked'
        WHERE resolution.subject=candidates.subject
          AND resolution.kind IN ('work.person-done','work.person-cancelled','work.person-override')
          AND (resolution.kind!='work.person-override' OR (
            json_extract(resolution.body,'$.fields.episode')=request.id
            AND resolution.actor=json_extract(request.body,'$.fields.person')
            AND json_type(resolution.body,'$.fields.answer')='object'
            AND EXISTS (SELECT 1 FROM claims deadline
              WHERE deadline.id=json_extract(resolution.body,'$.fields.overrides')
                AND deadline.subject=request.subject AND deadline.kind='work.person-done'
                AND json_extract(deadline.body,'$.fields.resolved_by')='deadline'
                AND json_extract(deadline.body,'$.fields.episode')=request.id)))
          AND EXISTS(SELECT 1 FROM sources WHERE
            (json_extract(request.body,'$.fields.origin_step')=sources.subject
              AND json_extract(request.body,'$.fields.origin_attempt')=sources.attempt)
            OR (resolution.subject=sources.subject AND
              (json_extract(resolution.body,'$.fields.attempt')=sources.attempt
               OR (resolution.kind='work.person-override'
                 AND json_extract(request.body,'$.fields.attempt')=sources.attempt))))
        ORDER BY CANONICAL_ASC(resolution)";

fn enrich_responses_with_sql(
    connection: &Connection,
    view: &mut StepRunView,
    sql: &str,
) -> rusqlite::Result<()> {
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8c2bec39d0b7 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql(sql))?;
    let responses = query
        .query_map(params![view.subject, view.attempt], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Each ask's responses, in canonical order, keeping only those that stand after a deadline.
    let mut asks: Vec<(String, Option<u128>, Vec<_>)> = Vec::new();
    for (ask, actor, body, accepted, from_origin, kind, deadline) in responses {
        let Ok(body) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        let response = (actor, body, accepted, from_origin, kind);
        match asks.iter_mut().find(|(subject, _, _)| *subject == ask) {
            Some((_, _, responses)) => {
                // Offline members can admit the same logical key independently. Its first
                // canonical receipt fixes that operation; a retry must not reselect an old answer.
                if response.4 == "work.person-override"
                    && response.1["fields"]["key"].as_str().is_some_and(|key| responses.iter().any(
                        |(_, previous, _, _, kind): &(Option<String>, Value, String, bool, String)|
                            kind == "work.person-override" && previous["fields"]["key"] == key)) {
                    continue;
                }
                responses.push(response);
            }
            None => asks.push((ask, deadline.and_then(|deadline| u128::try_from(deadline).ok()), vec![response])),
        }
    }
    let by_deadline = |(_, body, _, _, _): &(Option<String>, Value, String, bool, String)| body["fields"]["resolved_by"] == "deadline";
    for (ask, deadline, responses) in asks {
        // A held clock moves the cutoff past the stored deadline; the person's clock is read
        // only for an ask a deadline resolved.
        let cutoff = match deadline {
            Some(_) if responses.iter().any(by_deadline) => match request(connection, &ask)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?
            {
                Some(claim) => answer_cutoff(connection, &claim)?,
                None => deadline,
            },
            _ => deadline,
        };
        let (standing, overridden) = standing_responses(
            responses,
            by_deadline,
            |(_, body, _, _, kind)| kind == "work.person-done" && body["fields"]["resolved_by"] != "deadline",
            |(_, _, _, _, kind)| kind == "work.person-override",
            |(_, _, accepted, _, _)| accepted.parse().unwrap_or(0),
            cutoff,
        );
        if overridden && standing.iter().any(|(_, _, _, from_origin, _)| *from_origin) {
            let explanation = if standing.iter().any(|(_, _, _, _, kind)| kind == "work.person-override") {
                "the person explicitly overrode the default the deadline applied; already-taken effects are not rolled back"
            } else {
                "the person answered before its deadline, so their answer replaces the default the deadline applied"
            };
            view.constraints.push(format!("Person response to {ask}: {explanation}"));
        }
        for (actor, body, accepted, from_origin, _) in standing {
            enrich_response(view, ask.clone(), actor, &body, accepted, from_origin);
        }
    }
    Ok(())
}

/// Adds one standing response to the view, as a constraint for its asker and as data.
fn enrich_response(
    view: &mut StepRunView,
    ask: String,
    actor: Option<String>,
    body: &Value,
    accepted: String,
    from_origin: bool,
) {
    {
        let fields = &body["fields"];
        let Some(summary) = fields["summary"].as_str() else {
            return;
        };
        if from_origin {
            let mut constraint = format!("Person response: {summary}");
            if let Some(sender_class) = fields["sender_class"].as_str() {
                constraint.push_str(" [sender_class: ");
                constraint.push_str(sender_class);
                constraint.push(']');
            }
            if let Some(grant) = fields["assistant_grant_id"].as_str() {
                constraint.push_str(" [assistant_grant_id: ");
                constraint.push_str(grant);
                constraint.push(']');
            }
            view.constraints.push(constraint);
        }
        view.person_answers.push(crate::model::PersonAnswerView {
            acted_for: fields
                .get("acted_for")
                .and_then(Value::as_str)
                .map(str::to_owned),
            sender_class: fields
                .get("sender_class")
                .and_then(Value::as_str)
                .map(str::to_owned),
            assistant_grant_id: fields
                .get("assistant_grant_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            assistant_owner: fields.get("assistant_owner").and_then(Value::as_str).map(str::to_owned),
            assistant_session_actor: fields.get("assistant_session_actor").and_then(Value::as_str).map(str::to_owned),
            ask,
            status: fields["status"].as_str().unwrap_or("completed").into(),
            summary: summary.into(),
            respondent: actor.unwrap_or_default(),
            answered_at_unix_ms: accepted.parse().unwrap_or(0),
            answer: fields
                .get("answer")
                .filter(|answer| answer.is_object())
                .cloned(),
            evidence: serde_json::from_value(body["evidence"].clone()).unwrap_or_default(),
            resolved_by: fields
                .get("resolved_by")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
}


/// Reassignment captures only the current open episode. The original ask remains immutable,
/// including its requester and originally addressed person.
pub(super) fn reassign_person_steps_tx(
    tx: &Transaction<'_>,
    origin: &str,
    input: &PersonRenameRequest,
) -> Result<Vec<String>, FractalError> {
    let as_of = now_ms();
    let mut query = tx.prepare(
        "SELECT subject FROM step_runs WHERE assignee=?1 AND status IN ('ready','pending') ORDER BY subject",
    ).map_err(internal)?;
    let subjects = query.query_map([&input.old_person], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
    let mut reassigned = Vec::new();
    for subject in subjects {
        let Some(mut view) = step(tx, &subject).map_err(internal)? else { continue };
        apply_effective_step_state(tx, &mut view, as_of).map_err(internal)?;
        if !matches!(view.status.as_str(), "ready" | "pending")
            || !run_live(tx, &view.run, Some(&view.generation), false).map_err(internal)?
        {
            continue;
        }
        let ask = request(tx, &subject).map_err(internal)?;
        if let Some(ask) = &ask
            && !current(tx, ask, as_of).map_err(internal)?
        {
            continue;
        }
        let episode = ask.as_ref().map(|ask| ask.id.clone()).unwrap_or_else(|| {
            format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch)
        });
        let evidence = ask.map(|ask| vec![ask.id]).unwrap_or_default();
        let claim = append_claim_tx(tx, origin, &subject, "work.person-reassigned", Some(&input.actor),
            &json!({"fields": {"previous_person": input.old_person, "person": input.new_person,
                "run": view.run, "generation": view.generation, "attempt": view.attempt, "episode": episode}}),
            &evidence, None).map_err(claim_append_error)?;
        project(tx, &claim)?;
        reassigned.push(subject);
    }
    Ok(reassigned)
}

/// Shared projection for a one-step runtime run, used by asks and spontaneous work.
pub fn project_minimal_run(
    tx: &Transaction<'_>,
    claim: &ClaimRecord,
    mission: &MissionSpec,
    run: &str,
    generation: &str,
    root: &str,
    reason: &str,
) -> Result<(), FractalError> {
    let at = claim.accepted_at_unix_ms.to_string();
    tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hca41158cfea1 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_revisions(mission_id,revision,state,body,claim_id,created_index)
                VALUES(?1,?2,'ready',?3,?4,?5)").map_err(internal)?.execute( /* sqlite-large-value: allow-for-now lv-h02e44b418bcb owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![mission.id, mission.revision, serde_json::to_string(&mission).map_err(internal)?, claim.id, claim.store_index]).map_err(internal)?;
    // Independent daemons can create the same update revision. Retained revision history
    // survives replay, so its source must follow canonical claim order rather than arrival.
    let previous: String = tx
        .prepare_cached(
            "SELECT claim_id FROM mission_revisions WHERE mission_id=?1 AND revision=?2").map_err(internal)?.query_row(
            /* sqlite-large-value: allow-for-now lv-hd58e59ffadfb owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![mission.id, mission.revision],
            |row| row.get(0),
        )
        .map_err(internal)?;
    if previous != claim.id
        && canonical::claim_key(tx, &claim.id).map_err(internal)?
            < canonical::claim_key(tx, &previous).map_err(internal)?
    {
        tx.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h4249b590ecda owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_revisions SET claim_id=?3 WHERE mission_id=?1 AND revision=?2").map_err(internal)?.execute(
            /* sqlite-large-value: allow-for-now lv-h7acb721a77a0 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![mission.id, mission.revision, claim.id],
        )
        .map_err(internal)?;
    }
    tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc66679f685d8 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_definitions(mission_id,revision,state,claim_id) VALUES(?1,?2,'ready',?3)").map_err(internal)?.execute( /* sqlite-large-value: allow-for-now lv-h63c7229392d4 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![mission.id, mission.revision, claim.id]).map_err(internal)?;
    // A run-tree rebuild retains this definition too, and a full replay keeps the canonically
    // first claim's. Independent daemons can write different revisions of one internal mission:
    // each words an ended ask's reason from its own view. So the canonically first claim gives
    // the revision as well as the source, over a definition another minimal run wrote; this
    // does not change definition state, and a published definition, of any revision, stays.
    let definition_source: Option<(String, bool)> = tx
        .prepare_cached(
            "SELECT claim_id, EXISTS(SELECT 1 FROM claims
                 WHERE claims.id=mission_definitions.claim_id
                 AND claims.kind IN ('work.person-asked','mission-run.created'))
             FROM mission_definitions WHERE mission_id=?1").map_err(internal)?.query_row(
            /* sqlite-large-value: allow-for-now lv-hc243e0d38752 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [&mission.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(internal)?;
    if let Some((previous, replaceable)) = definition_source
        && replaceable
        && previous != claim.id
        && canonical::claim_key(tx, &claim.id).map_err(internal)?
            < canonical::claim_key(tx, &previous).map_err(internal)?
    {
        tx.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h5397dfc4c7e7 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_definitions SET revision=?2, claim_id=?3 WHERE mission_id=?1").map_err(internal)?.execute(
            /* sqlite-large-value: allow-for-now lv-h8833f2091d49 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![mission.id, mission.revision, claim.id],
        )
        .map_err(internal)?;
    }
    tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h540720b8262b owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,
                workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
                VALUES(?1,?2,?3,?4,?3,?5,'.',?6,'{}','run','running','normal',?7,?7)").map_err(internal)?.execute(
                /* sqlite-large-value: allow-for-now lv-hf999d2f67df9 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![run, mission.id, mission.revision, generation, root, claim.actor, at]).map_err(internal)?;
    tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-he893baab2fb0 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO run_generations(id,run_id,revision,status,actor,reason,created_at_unix_ms,updated_at_unix_ms)
                VALUES(?1,?2,?3,'running',?4,?6,?5,?5)").map_err(internal)?.execute( /* sqlite-large-value: allow-for-now lv-h45dacb7b8a3f owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![generation, run, mission.revision, claim.actor, at, reason]).map_err(internal)?;
    Ok(())
}

/// Whether the origin's immutable revision gives it a typed program handler.
pub fn runs_program(tx: &Transaction<'_>, step: &StepRunView) -> Result<bool> {
    let body: Option<String> = tx.query_row(
        "SELECT r.body FROM run_generations g
         JOIN mission_runs m ON m.id=?2
         JOIN mission_revisions r ON r.mission_id=m.mission_id AND r.revision=g.revision
         WHERE g.id=?1",
        params![generation_id_from_subject(&step.generation), step.run.strip_prefix("mission-run/").unwrap_or(&step.run)],
        |row| row.get(0),
    ).optional()?;
    let Some(body) = body else { return Ok(false) };
    let mission: MissionSpec = serde_json::from_str(&body)?;
    Ok(flatten_mission_step_specs(&mission).into_iter()
        .any(|spec| spec.path == step.step && spec.program.is_some()))
}

pub fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<bool, FractalError> {
    let fields = &claim.body["fields"];
    if claim.kind == "work.person-reassigned" {
        static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
            "SELECT {} FROM claims reassignment WHERE reassignment.id=?1",
            person_rename::accepted_reassignment_sql("reassignment", "previous_person", "person"),
        ));
        let accepted: bool = tx.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h41b006e9c464 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */
            &QUERY).map_err(internal)?.query_row(
            /* sqlite-large-value: allow-for-now lv-h89ab0ff0fd4b owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [&claim.id], |row| row.get(0)).map_err(internal)?;
        if !accepted { return Ok(true); }
        let Some(view) = step(tx, &claim.subject).map_err(internal)? else {
            return Ok(true);
        };
        let ask = request(tx, &claim.subject).map_err(internal)?;
        let episode = ask.map(|ask| ask.id).unwrap_or_else(|| {
            format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch)
        });
        if fields["run"].as_str() != Some(view.run.as_str())
            || fields["generation"].as_str() != Some(view.generation.as_str())
            || fields["attempt"].as_u64() != Some(u64::from(view.attempt))
            || fields["episode"].as_str() != Some(episode.as_str())
        {
            return Ok(true);
        }
        tx.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hf6f5b79d7be9 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */
            "UPDATE step_runs SET assignee=?2,updated_at_unix_ms=?3
             WHERE subject=?1 AND assignee=?4 AND status IN ('ready','pending')").map_err(internal)?.execute(
            /* sqlite-large-value: allow-for-now lv-h7942ba074b64 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![claim.subject, fields["person"].as_str(), claim.accepted_at_unix_ms.to_string(),
                fields["previous_person"].as_str()],
        ).map_err(internal)?;
        return Ok(true);
    }
    if claim.kind == "work.person-asked" {
        let run = fields["run"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("mission-run/");
        let generation = fields["generation"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("run-generation/");
        if let Some(spec) = fields.get("mission_spec") {
            let mission: MissionSpec = serde_json::from_value(spec.clone()).map_err(internal)?;
            let root = fields["owner_run"]
                .as_str()
                .unwrap_or(run)
                .trim_start_matches("mission-run/");
            project_minimal_run(tx, claim, &mission, run, generation, root, "person ask")?;
        }
        tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hfddcac254de6 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,
            attempt,assignee,available_to,agentless,title,goals,created_at_unix_ms,updated_at_unix_ms,constraints)
            VALUES(?1,?2,?3,?4,?5,'ready',1,?6,'[]',0,?7,?8,?9,?9,'[]')").map_err(internal)?.execute(
            /* sqlite-large-value: allow-for-now lv-hcd263a451d78 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![claim.subject, run, generation, claim.subject.rsplit('/').next(), claim.id,
                fields["person"].as_str(), fields["title"].as_str(), serde_json::to_string(&vec![fields["reason"].as_str().unwrap_or_default()]).map_err(internal)?, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
        if step_owner_is_terminal_tx(tx, &claim.subject)? {
            tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-he796c14d0fd2 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled',blocked_reason='the owning run ended' WHERE subject=?1").map_err(internal)?.execute( /* sqlite-large-value: allow-for-now lv-h0310fcb0097b owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [&claim.subject]).map_err(internal)?;
            return Ok(true);
        }
        if let Some(origin) = fields["origin_step"].as_str() {
            tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h2c813e50cf88 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='waiting-person',lease_owner=NULL,lease_incarnation=NULL,
                lease_expires_at_unix_ms=NULL,blocked_reason=?3,updated_at_unix_ms=?4
                WHERE subject=?1 AND attempt=?2 AND status IN ('claimed','working','ready','blocked')").map_err(internal)?.execute(
                /* sqlite-large-value: allow-for-now lv-he6c79429ef0c owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![origin, fields["origin_attempt"].as_u64(), claim.subject, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
            tx.prepare_cached(
                /* sqlite-large-value: allow-for-now lv-h6462186c014f owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_work_lease_renewals WHERE subject=?1").map_err(internal)?.execute(
                /* sqlite-large-value: allow-for-now lv-h4fa571431ee9 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ [origin],
            )
            .map_err(internal)?;
        }
        return Ok(true);
    }
    if !matches!(
        claim.kind.as_str(),
        "work.person-done" | "work.person-cancelled"
    ) {
        return Ok(false);
    }
    tx.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-ha5f213f5faef owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2,worker_reported=1,updated_at_unix_ms=?3
        WHERE subject=?1 AND attempt=?4 AND status IN ('ready','pending')").map_err(internal)?.execute(
        /* sqlite-large-value: allow-for-now lv-h3e5d60c5f48a owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![
            claim.subject,
            fields["status"].as_str(),
            claim.accepted_at_unix_ms.to_string(),
            fields["attempt"].as_u64()
        ],
    )
    .map_err(internal)?;
    if let Some(ask) = request(tx, &claim.subject).map_err(internal)? {
        if ask.body["fields"].get("mission_spec").is_some() {
            tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbbd28d35da4a owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status=?2,phase='terminal',updated_at_unix_ms=?3 WHERE id=?1").map_err(internal)?.execute(
                /* sqlite-large-value: allow-for-now lv-he50bebd64535 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![ask.body["fields"]["run"].as_str().unwrap_or_default().trim_start_matches("mission-run/"), fields["status"].as_str(), claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
        }
        if let Some(origin) = ask.body["fields"]["origin_step"].as_str() {
            if !step_owner_is_terminal_tx(tx, origin)? {
                let cancelled = claim.kind == "work.person-cancelled";
                // The response's asking claim is the episode fence, not merely the origin
                // subject. A stale replica may cancel after another member resumed or retried.
                if cancelled {
                    let exact_episode = match fields.get("episode") {
                        Some(Value::String(episode)) => episode == &ask.id,
                        // Older automatic cancellations bind the asking claim causally
                        // instead of recording its ID in the response fields.
                        None => claim.predecessors.contains(&ask.id),
                        _ => false,
                    };
                    if !exact_episode {
                        return Ok(true);
                    }
                }
                let failed = cancelled && step(tx, origin).map_err(internal)?
                    .map(|view| runs_program(tx, &view)).transpose().map_err(internal)? == Some(false);
                let reason = failed.then(|| format!("ask cancelled by {}", claim.actor.as_deref().unwrap_or_default()));
                tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8c515cfc602a owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?5,blocked_reason=?6,
                    readiness_epoch=readiness_epoch+CASE WHEN ?5='ready' THEN 1 ELSE 0 END,
                    activated_at_unix_ms=CASE WHEN ?5='ready' THEN ?3 ELSE activated_at_unix_ms END,
                    updated_at_unix_ms=?3 WHERE subject=?1 AND attempt=?2 AND status='waiting-person' AND blocked_reason=?4").map_err(internal)?.execute(
                    /* sqlite-large-value: allow-for-now lv-h109f5388a001 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ params![origin, ask.body["fields"]["origin_attempt"].as_u64(), claim.accepted_at_unix_ms.to_string(), ask.subject,
                        if failed { "failed" } else { "ready" }, reason]).map_err(internal)?;
            }
        }
    }
    Ok(true)
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::person_work_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::person_work_tests_shared_seats::{TEST_FLEET, gateway_decision, receive_exchange};
use super::*;

    pub fn receive(source: &Store, target: &Store) {
        let exchange = source
            .export_replication_exchange_answering(
                TEST_FLEET,
                &target.replication_inventory().unwrap(),
                &[],
            )
            .unwrap();
        receive_exchange(target, &exchange);
    }


}

#[cfg(any(test, feature = "test-support"))]
pub mod reconcile_candidates_tests_shared_seats;

#[cfg(any(test, feature = "test-support"))]
pub mod revision_provenance_tests_shared_missions;

#[cfg(test)]
mod standing_responses_tests {
    use super::standing_responses;

    /// (name, by the deadline, a person's answer, accepted at)
    type Response = (&'static str, bool, bool, u128);

    fn standing(responses: Vec<Response>, deadline: Option<u128>) -> (Vec<&'static str>, bool) {
        let (kept, overridden) = standing_responses(
            responses,
            |response| response.1,
            |response| response.2,
            |_| false,
            |response| response.3,
            deadline,
        );
        (kept.into_iter().map(|response| response.0).collect(), overridden)
    }

    #[test]
    fn a_person_answer_before_the_cutoff_beats_the_default_and_a_later_one_does_not() {
        let default = ("default", true, false, 1_000);
        let in_time = ("in-time", false, true, 999);
        let late = ("late", false, true, 1_000);
        let cancelled = ("cancelled", false, false, 2_000);
        // An answer accepted before the deadline stands over the default, in any arrival order.
        assert_eq!(standing(vec![default, in_time], Some(1_000)), (vec!["in-time"], true));
        assert_eq!(standing(vec![in_time, default], Some(1_000)), (vec!["in-time"], true));
        // At or after the deadline it does not, and the default stands alone.
        assert_eq!(standing(vec![default, late], Some(1_000)), (vec!["default"], false));
        // Without a deadline on the ask, the cutoff is when the default was applied.
        assert_eq!(standing(vec![("default", true, false, 1_500), ("late", false, true, 1_500)], None), (vec!["default"], false));
        assert_eq!(standing(vec![("default", true, false, 1_500), in_time], None), (vec!["in-time"], true));
        // A cancellation is not a person's answer and always stands; without a default all do.
        assert_eq!(standing(vec![default, cancelled], Some(1_000)), (vec!["default", "cancelled"], false));
        assert_eq!(standing(vec![late, cancelled], Some(1_000)), (vec!["late", "cancelled"], false));
    }

    #[test]
    fn explicit_overrides_bypass_the_cutoff_and_only_the_latest_stands() {
        let (kept, overridden) = standing_responses(
            vec![("default", true, false, 1_000), ("early", false, true, 999),
                ("override-first", false, true, 2_000), ("override-latest", false, true, 3_000)],
            |response| response.1,
            |response| response.2,
            |response| response.0.starts_with("override-"),
            |response| response.3,
            Some(1_000),
        );
        assert!(overridden);
        assert_eq!(kept.iter().map(|response| response.0).collect::<Vec<_>>(), ["override-latest"]);
    }

    fn response_connection() -> super::Connection {
        let connection = super::Connection::open_in_memory().unwrap();
        connection.execute_batch("
            CREATE TABLE batches(id TEXT PRIMARY KEY,origin TEXT,replica_sequence INTEGER);
            CREATE TABLE claims(id TEXT UNIQUE,store_index INTEGER PRIMARY KEY AUTOINCREMENT,
                batch_id TEXT,subject TEXT,kind TEXT,actor TEXT,body TEXT,accepted_at_unix_ms TEXT,
                origin TEXT NOT NULL DEFAULT 'member-alder',predecessors TEXT NOT NULL DEFAULT '[]');
            CREATE TABLE claim_positions(claim_id TEXT,position INTEGER);
            CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
            CREATE INDEX claims_subject_kind_index ON claims(subject,kind);
        ").unwrap();
        // Install the production index instead of independently maintaining a test index.
        let index = super::super::SCHEMA.split(';').find(|statement|
            statement.contains("CREATE INDEX IF NOT EXISTS claims_person_ask_origin_index")
        ).expect("person ask origin index is installed on store open");
        connection.execute_batch(index).unwrap();
        connection
    }

    #[test]
    fn carried_response_enrichment_does_not_read_unrelated_history() {
        use super::*;
        let connection = response_connection();
        connection.execute_batch("
            CREATE INDEX claims_kind_index ON claims(kind);
            INSERT INTO batches VALUES('selected','member-alder',1);
            INSERT INTO claims(id,batch_id,subject,kind,body,accepted_at_unix_ms) VALUES
                ('carry','selected','new','step-run.carried',
                 '{\"fields\":{\"attempt\":2,\"source_step_run\":\"old\",\"source_attempt\":1}}','1'),
                ('answer','selected','old','work.person-done',
                 '{\"fields\":{\"attempt\":1,\"summary\":\"kept\"}}','2'),
                ('boundary','selected','zz-boundary','step-run.carried','{\"fields\":{}}','3');
        ").unwrap();
        // Seed a key above the measured subjects in both populations. Without it,
        // the small fixture ends its index range at EOF, while the grown fixture
        // executes one extra IdxGT to reject the next subject. Compare the same
        // range-boundary path, retaining exact VM equality rather than a tolerance.
        let sql = canonical_sql(RESPONSES_SQL);
        let measure = || {
            let mut query = connection.prepare(&sql).unwrap();
            let subjects = query.query_map(params!["new", 2], |row| row.get::<_, String>(0))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert_eq!(subjects, ["old"]);
            query.get_status(rusqlite::StatementStatus::VmStep)
        };
        let before = measure();
        connection.execute_batch("
            INSERT INTO batches VALUES('noise','member-alder',2);
            WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000)
            INSERT INTO claims(id,batch_id,subject,kind,body,accepted_at_unix_ms)
            SELECT 'carry/'||i,'noise','unrelated/'||i,'step-run.carried',
                '{\"fields\":{\"attempt\":2,\"source_step_run\":\"elsewhere\"}}','3' FROM n;
            WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000)
            INSERT INTO claims(id,batch_id,subject,kind,body,accepted_at_unix_ms)
            SELECT 'answer/'||i,'noise','unrelated/'||i,'work.person-done',
                '{\"fields\":{\"attempt\":1}}','4' FROM n;
        ").unwrap();
        let after = measure();
        assert_eq!(before, after, "unrelated carried and response history must not increase VM work");
        let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap()
            .query_map(params!["new", 2], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(plan.iter().any(|line| line.contains(
            "SEARCH carried USING INDEX claims_subject_kind_index (subject=? AND kind=?)"
        )), "{plan:?}");
    }

    fn enrich_equivalent(connection: &super::Connection, view: &mut super::StepRunView) {
        use super::*;
        // Reconstruct the previous global scan without indexed candidate selection.
        let (sources, candidates_and_select) = RESPONSES_SQL.split_once("), candidates(subject) AS (").unwrap();
        let (_, select) = candidates_and_select.split_once("\n        SELECT resolution.subject").unwrap();
        let legacy_sql = format!("{sources})\n        SELECT resolution.subject{select}")
            .replace("FROM candidates CROSS JOIN claims resolution INDEXED BY claims_subject_kind_index",
                "FROM claims resolution")
            .replace("WHERE resolution.subject=candidates.subject\n          AND resolution.kind",
                "WHERE resolution.kind");
        let mut legacy = view.clone();
        enrich_responses_with_sql(connection, &mut legacy, &legacy_sql).unwrap();
        enrich_responses(connection, view).unwrap();
        assert_eq!(serde_json::to_value(&*view).unwrap(), serde_json::to_value(legacy).unwrap());
    }

    #[test]
    fn override_projection_converges_for_reordered_history_and_fences_invalid_links() {
        use super::*;
        let history = [
            ("ask", "work.person-asked", "agent/requester", 1, json!({
                "person":"person/avery","attempt":1,"origin_step":"origin","origin_attempt":1
            })),
            ("deadline", "work.person-done", "daemon/runtime", 1_000, json!({
                "episode":"ask","attempt":1,"resolved_by":"deadline","summary":"Default"
            })),
            ("first", "work.person-override", "person/avery", 2_000, json!({
                "episode":"ask","overrides":"deadline","summary":"First","key":"person-override:ask:first",
                "answer":{"id":"first","type":"decision","outcome":"accept","label":"First"}
            })),
            ("latest", "work.person-override", "person/avery", 3_000, json!({
                "episode":"ask","overrides":"deadline","summary":"Latest","key":"person-override:ask:latest",
                "answer":{"id":"latest","type":"decision","outcome":"decline","label":"Latest"}
            })),
            ("retry-first", "work.person-override", "person/avery", 3_500, json!({
                "episode":"ask","overrides":"deadline","summary":"First","key":"person-override:ask:first",
                "answer":{"id":"first","type":"decision","outcome":"accept","label":"First"}
            })),
            ("wrong-episode", "work.person-override", "person/avery", 4_000, json!({
                "episode":"another-ask","overrides":"deadline","summary":"Wrong episode","answer":{}
            })),
            ("wrong-link", "work.person-override", "person/avery", 5_000, json!({
                "episode":"ask","overrides":"first","summary":"Not a deadline","answer":{}
            })),
            ("wrong-person", "work.person-override", "person/other", 6_000, json!({
                "episode":"ask","overrides":"deadline","summary":"Not the person","answer":{}
            })),
        ];
        for reverse in [false, true] {
            // Independent retained-history readers model opposite replication arrival orders.
            let connection = response_connection();
            for index in 0..history.len() {
                let index = if reverse { history.len() - 1 - index } else { index };
                let (id, kind, actor, at, fields) = &history[index];
                connection.execute("INSERT INTO batches VALUES(?1,'member-alder',?2)", params![id, index as i64]).unwrap();
                connection.execute(
                    "INSERT INTO claims(id,batch_id,subject,kind,actor,body,accepted_at_unix_ms)
                     VALUES(?1,?1,'ask-step',?2,?3,?4,?5)",
                    params![id, kind, actor, json!({"fields":fields}).to_string(), at.to_string()],
                ).unwrap();
            }
            for subject in ["ask-step", "origin"] {
                let mut view: StepRunView = serde_json::from_value(json!({
                    "subject":subject,"run":"mission-run/example","generation":"generation/example",
                    "step":"ask","definition_hash":"definition","status":"completed","attempt":1,
                    "agentless":false,"worker_reported":true,"readiness_epoch":1,
                    "created_at_unix_ms":1,"updated_at_unix_ms":1_000
                })).unwrap();
                enrich_equivalent(&connection, &mut view);
                assert_eq!(view.person_answers.len(), 1);
                assert_eq!(view.person_answers[0].summary, "Latest");
                assert_eq!(view.status, "completed", "an override must not reopen the origin");
            }
        }
    }

    #[test]
    fn indexed_responses_match_global_scan_across_carried_attempts_and_deadlines() {
        use super::*;
        let history = [
            ("carry-new", "new", "step-run.carried", 1, json!({
                "attempt":3,"source_step_run":"middle","source_attempt":2
            })),
            // An omitted source_attempt inherits the carried attempt.
            ("carry-middle", "middle", "step-run.carried", 2, json!({
                "attempt":2,"source_step_run":"old"
            })),
            // A cycle must terminate and must not duplicate a response.
            ("carry-cycle", "old", "step-run.carried", 3, json!({
                "attempt":2,"source_step_run":"new","source_attempt":3
            })),
            ("carry-other-attempt", "new", "step-run.carried", 4, json!({
                "attempt":4,"source_step_run":"unrelated","source_attempt":1
            })),
            ("ask", "ask-step", "work.person-asked", 10, json!({
                "attempt":1,"origin_step":"old","origin_attempt":2,"deadline_unix_ms":1000
            })),
            ("default", "ask-step", "work.person-done", 1000, json!({
                "attempt":1,"episode":"ask","resolved_by":"deadline","summary":"Default"
            })),
            ("before", "ask-step", "work.person-done", 999, json!({
                "attempt":1,"summary":"Before cutoff"
            })),
            ("late", "ask-step", "work.person-done", 1001, json!({
                "attempt":1,"summary":"Late"
            })),
            ("other-ask", "other", "work.person-asked", 11, json!({
                "attempt":1,"origin_step":"old","origin_attempt":1
            })),
            ("other-answer", "other", "work.person-done", 12, json!({
                "attempt":1,"summary":"Other attempt"
            })),
            // A self response needs neither an ask row nor an origin.
            ("orphan", "orphan-step", "work.person-cancelled", 13, json!({
                "attempt":2,"summary":"Cancelled without request"
            })),
            ("no-origin", "self", "work.person-asked", 14, json!({"attempt":2})),
            ("self-answer", "self", "work.person-done", 15, json!({
                "attempt":2,"summary":"Own answer"
            })),
        ];
        for (reverse, before_at) in [(false, 999), (true, 999), (false, 1000), (true, 1000)] {
            let connection = response_connection();
            for index in 0..history.len() {
                let index = if reverse { history.len() - 1 - index } else { index };
                let (id, subject, kind, at, fields) = &history[index];
                let at = if *id == "before" { before_at } else { *at };
                connection.execute("INSERT INTO batches VALUES(?1,'member-alder',?2)",
                    params![id, index as i64]).unwrap();
                connection.execute(
                    "INSERT INTO claims(id,batch_id,subject,kind,actor,body,accepted_at_unix_ms)
                     VALUES(?1,?1,?2,?3,'agent/requester',?4,?5)",
                    params![id, subject, kind, json!({"fields":fields}).to_string(), at.to_string()],
                ).unwrap();
            }
            let standing = Some(if before_at < 1000 { "Before cutoff" } else { "Default" });
            for (subject, attempt, summary) in [
                ("new", 3, standing),
                ("middle", 2, standing),
                ("old", 2, standing),
                ("ask-step", 1, standing),
                ("old", 1, Some("Other attempt")),
                ("orphan-step", 2, Some("Cancelled without request")),
                ("self", 2, Some("Own answer")),
                ("new", 1, None),
                ("missing", 1, None),
            ] {
                let mut view: StepRunView = serde_json::from_value(json!({
                    "subject":subject,"run":"mission-run/example","generation":"generation/example",
                    "step":"ask","definition_hash":"definition","status":"completed","attempt":attempt,
                    "agentless":false,"worker_reported":true,"readiness_epoch":1,
                    "created_at_unix_ms":1,"updated_at_unix_ms":1000
                })).unwrap();
                enrich_equivalent(&connection, &mut view);
                assert_eq!(view.person_answers.iter().map(|answer| answer.summary.as_str()).collect::<Vec<_>>(),
                    summary.into_iter().collect::<Vec<_>>(), "{subject} attempt {attempt}");
            }
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {}", canonical_sql(RESPONSES_SQL)))
                .unwrap().query_map(params!["new", 3], |row| row.get::<_, String>(3))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert!(plan.iter().any(|line| line.contains("SEARCH request USING INDEX claims_person_ask_origin_index")),
                "{plan:?}");
            assert!(plan.iter().any(|line| line.contains("SEARCH resolution USING INDEX claims_subject_kind_index")),
                "{plan:?}");
        }
    }
}
