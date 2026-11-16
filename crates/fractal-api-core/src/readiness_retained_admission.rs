//! Durable shared readiness cases and phase-specific notice delivery.
use super::*;
use crate::model::ClaimInput;
#[cfg(test)]
use crate::model::IntentInput;
use std::collections::BTreeMap;

#[path = "readiness_case_keys.rs"]
mod keys;

const CASE_JOURNAL_MAX_BYTES: u64 = 16 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    member: String,
    executor: String,
    through: u64,
    case: Option<Opening>,
    #[serde(default)]
    weekly: Weekly,
    #[serde(default)]
    notices: Vec<Notice>,
    #[serde(default)]
    legacy_pending: Option<Reading>,
    #[serde(default)]
    legacy_imported: bool,
    #[serde(default)]
    legacy_draining: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Notice {
    episode: String,
    owner: String,
    phase: String,
    recipient: String,
    failure: String,
    held: bool,
    recorded: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Opening {
    episode: String,
    reviewer: String,
    cut: u64,
    failed: usize,
    known: usize,
    recorded: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Weekly {
    through: u64,
    escalated: bool,
    recovered: bool,
    pending: Option<WeeklyAction>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WeeklyAction {
    Escalate { cut: u64 },
    Recover { cut: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "version")]
enum Journal<T> {
    #[serde(rename = "1")]
    V1(T),
}

impl Admission {
    fn load(path: &Path, config: &ReadinessSloConfig) -> Result<Self> {
        match File::open(path) {
            Ok(file) => {
                let Journal::V1(admission): Journal<Self> = read_json_bounded(file, CASE_JOURNAL_MAX_BYTES)?;
                ensure!(
                    admission.member == config.member && admission.executor == config.executor,
                    "early readiness journal has a different member or executor"
                );
                Ok(admission)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                member: config.member.clone(),
                executor: config.executor.clone(),
                through: 0,
                case: None,
                weekly: Weekly::default(),
                legacy_pending: None,
                legacy_imported: false,
                legacy_draining: false,
                notices: Vec::new(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    fn persist(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("early admission has no state directory")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        // The existing weekly checkpoint is intentionally separate.
        serde_json::to_writer(&mut temporary, &Journal::V1(self))?;
        temporary.flush()?;
        ensure!(temporary.as_file().metadata()?.len() <= CASE_JOURNAL_MAX_BYTES);
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn reset_recovered(&mut self) {
        if self.weekly.recovered {
            self.case = None;
            self.weekly = Weekly { through: self.weekly.through, ..Weekly::default() };
            self.notices.retain(|notice| !notice.recorded && !notice.held);
        }
    }

    fn plan(&mut self, config: &ReadinessSloConfig, summary: ShortSummary) -> bool {
        if self.legacy_draining || summary.end_ms <= self.through {
            return false;
        }
        self.through = summary.end_ms;
        self.reset_recovered();
        // Once an early case opens, a later clear or Unknown never closes it.
        if self.case.is_none() && summary.known >= 4 && summary.failed >= 3 {
            self.case = Some(Opening {
                episode: format!(
                    "slo-objective/readiness-case/{}/{OBJECTIVE}/{}/{}",
                    self.executor, self.member, summary.end_ms
                ),
                reviewer: config.reviewer.clone(),
                cut: summary.end_ms,
                failed: summary.failed,
                known: summary.known,
                recorded: false,
            });
        }
        true
    }

    fn reconcile(
        &mut self,
        path: &Path,
        config: &ReadinessSloConfig,
        next: impl FnOnce() -> Result<Vec<ShortSummary>>,
        dispatch: impl Fn(&Opening) -> Result<bool>,
    ) -> Result<()> {
        self.replay(path, &dispatch)?;
        for summary in next()? {
            let mut admitted = self.clone();
            if admitted.plan(config, summary) {
                admitted.persist(path)?;
                *self = admitted;
                self.replay(path, &dispatch)?;
            }
        }
        Ok(())
    }

    fn weekly(
        &mut self,
        path: &Path,
        config: &ReadinessSloConfig,
        store: &Store,
        reading: &Reading,
        now: u64,
        dispatch: impl Fn(&Opening, &WeeklyAction) -> Result<bool>,
    ) -> Result<()> {
        self.replay(path, |opening| record(store, config, opening))?;
        self.replay_weekly(path, &dispatch)?;
        // An unresolved recovery receipt keeps its original admitted cut on replay.
        // Other drain items run independently; fresh input cannot discard this work.
        if matches!(self.weekly.pending, Some(WeeklyAction::Recover { .. })) {
            return Ok(());
        }
        reading.check(&self.member, now)?;
        if self.legacy_draining || reading.end_ms <= self.weekly.through {
            return Ok(());
        }
        // Weekly-only cases must restart even when no retained short cuts are available.
        self.reset_recovered();
        if self.case.is_none() && reading.status() == "missed" {
            self.case = Some(Opening {
                episode: format!("slo-objective/readiness-case/{}/{OBJECTIVE}/{}/{}",
                    self.executor, self.member, reading.end_ms),
                reviewer: config.reviewer.clone(), cut: reading.end_ms,
                failed: 0, known: 0, recorded: true,
            });
        }
        if self.case.is_none() {
            return Ok(());
        }
        let mut admitted = self.clone();
        admitted.weekly.through = reading.end_ms;
        admitted.weekly.pending = match reading.status() {
            "missed" if !admitted.weekly.escalated => Some(WeeklyAction::Escalate {
                cut: reading.end_ms,
            }),
            "met" if !admitted.weekly.recovered
                && admitted.case.as_ref().is_some_and(|opening| reading.end_ms > opening.cut) => {
                Some(WeeklyAction::Recover {
                    cut: reading.end_ms,
                })
            }
            _ => None,
        };
        admitted.persist(path)?;
        *self = admitted;
        self.replay_weekly(path, &dispatch)
    }

    fn replay_weekly(
        &mut self,
        path: &Path,
        dispatch: impl FnOnce(&Opening, &WeeklyAction) -> Result<bool>,
    ) -> Result<()> {
        let Some(action) = self.weekly.pending.as_ref() else {
            return Ok(());
        };
        let opening = self.case.as_ref().context("weekly phase has no case")?;
        if !dispatch(opening, action)? { return Ok(()); }
        let mut settled = self.clone();
        match action {
            WeeklyAction::Escalate { .. } => settled.weekly.escalated = true,
            WeeklyAction::Recover { .. } => {
                settled.weekly.recovered = true;
                // Companion cuts at/before measured recovery belong to the closed case.
                settled.through = settled.through.max(settled.weekly.through);
                // Informational delivery never survives the case it describes.
                settled.notices.retain(|notice| !notice.held || notice.episode != opening.episode);
            }
        }
        settled.weekly.pending = None;
        settled.persist(path)?;
        *self = settled;
        Ok(())
    }

    fn replay(&mut self, path: &Path, dispatch: impl FnOnce(&Opening) -> Result<bool>) -> Result<()> {
        let mut settled = self.clone();
        if let Some(opening) = settled.case.as_mut().filter(|opening| !opening.recorded) {
            if !dispatch(opening)? { return Ok(()); }
            opening.recorded = true;
            settled.persist(path)?;
            *self = settled;
        }
        Ok(())
    }
}

fn phase_receipt(store: &Store, opening: &Opening, phase: &str) -> Result<keys::Lookup> {
    let root = format!("{}:{phase}", opening.episode);
    let source = format!("daemon/{}", store.origin());
    let (severity, title) = if phase == "early" {
        ("warning", "API readiness early warning")
    } else {
        ("error", "API readiness weekly breach")
    };
    keys::lookup(store, &root, &opening.reviewer, |receipt, _key| {
        receipt.kind == "operational.failure"
            && receipt.actor.as_deref() == Some("daemon/runtime")
            && receipt.origin == store.origin() && receipt.subject == source
            && receipt.body["fields"]["episode"] == opening.episode
            && receipt.body["fields"]["reviewer"] == opening.reviewer
            && receipt.body["fields"]["condition"] == crate::store::API_READINESS_SLO_MISSED
            && receipt.body["fields"]["severity"] == severity
            && receipt.body["fields"]["title"] == title
    })
}

fn record(store: &Store, config: &ReadinessSloConfig, opening: &Opening) -> Result<bool> {
    ensure!(
        store.origin() == config.executor,
        "early readiness store is not the executor"
    );
    let key = format!("{}:early", opening.episode);
    match phase_receipt(store, opening, "early")? {
        keys::Lookup::Recorded(_) => return Ok(true),
        keys::Lookup::Blocked => return Ok(false),
        keys::Lookup::Vacant(_) => {},
    }
    ensure!(
        store.fault_reviewer_is_declared(&opening.reviewer)?,
        "early readiness requires the originally configured active reviewer"
    );
    let source = format!("daemon/{}", store.origin());
    let mut input = ClaimInput {
        subject: source.clone(),
        kind: "operational.failure".into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("episode".into(), json!(opening.episode)),
            (
                "condition".into(),
                json!(crate::store::API_READINESS_SLO_MISSED),
            ),
            ("reviewer".into(), json!(opening.reviewer)),
            ("severity".into(), json!("warning")),
            ("title".into(), json!("API readiness early warning")),
            (
                "reason".into(),
                json!(format!(
                    "{} failed of {} completed minutes at cut {}",
                    opening.failed, opening.known, opening.cut
                )),
            ),
            ("targets".into(), json!([source])),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key),
    };
    Ok(keys::append(store, &mut input, &opening.reviewer)?.is_some())
}

fn record_weekly(store: &Store, opening: &Opening, action: &WeeklyAction) -> Result<bool> {
    let source = format!("daemon/{}", store.origin());
    let early = phase_receipt(store, opening, "early")?;
    let weekly_key = format!("{}:weekly", opening.episode);
    match action {
        WeeklyAction::Escalate { cut } => {
            match phase_receipt(store, opening, "weekly")? {
                keys::Lookup::Recorded(_) => return Ok(true),
                keys::Lookup::Blocked => return Ok(false),
                keys::Lookup::Vacant(_) => {},
            }
            ensure!(
                store.fault_reviewer_is_declared(&opening.reviewer)?,
                "original reviewer is unavailable"
            );
            let mut input = ClaimInput {
                subject: source.clone(),
                kind: "operational.failure".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("episode".into(), json!(opening.episode)),
                    (
                        "condition".into(),
                        json!(crate::store::API_READINESS_SLO_MISSED),
                    ),
                    ("reviewer".into(), json!(opening.reviewer)),
                    ("title".into(), json!("API readiness weekly breach")),
                    (
                        "reason".into(),
                        json!(format!("Weekly readiness breached at cut {cut}")),
                    ),
                    ("severity".into(), json!("error")),
                    ("targets".into(), json!([source])),
                ]),
                evidence: match early {
                    keys::Lookup::Recorded(claim) => vec![claim.id],
                    keys::Lookup::Vacant(_) | keys::Lookup::Blocked => Vec::new(),
                },
                expected_subject: None,
                idempotency_key: Some(weekly_key),
            };
            Ok(keys::append(store, &mut input, &opening.reviewer)?.is_some())
        }
        WeeklyAction::Recover { cut } => {
            let weekly = phase_receipt(store, opening, "weekly")?;
            // A bounded lookup can conceal a previously accepted continuation.
            // Blocked is unknown occupancy, never proof that no failure exists.
            let mut complete = true;
            for phase in [early, weekly] {
                let prior = match phase {
                    keys::Lookup::Recorded(claim) => claim.id,
                    keys::Lookup::Blocked => {
                        complete = false;
                        continue;
                    },
                    keys::Lookup::Vacant(_) => continue,
                };
                let key = format!("{prior}:recovered");
                let mut input = ClaimInput {
                    subject: source.clone(),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(opening.episode)),
                        ("failure".into(), json!(prior)),
                        (
                            "reason".into(),
                            json!(format!("Measured weekly recovery at cut {cut}")),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(key),
                };
                complete &= keys::append(store, &mut input, &opening.reviewer)?.is_some();
            }
            Ok(complete)
        }
    }
}

fn notice_input(notice: &Notice) -> ClaimInput {
    use sha2::Digest as _;
    let key = if notice.held {
        format!("{}:{}:{}", notice.episode, notice.phase, notice.recipient)
    } else {
        format!("{}:notice:{}", notice.episode, notice.phase)
    };
    ClaimInput {
        subject: format!("message/{}", &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16]),
        kind: "message.sent".into(), actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("from".into(), json!("daemon/runtime")),
            ("to".into(), json!(notice.recipient)),
            ("title".into(), json!(format!("API readiness {}", notice.phase))),
            ("content".into(), json!(format!(
                "{}Readiness case {} is owned by {}. Inspect failure {}.",
                if notice.held { "Informational only. " } else { "Action required. " },
                notice.episode, notice.owner, notice.failure
            ))),
            ("status".into(), json!("sent")),
            ("tags".into(), if notice.held { json!(["st3-silent"]) } else { json!([]) }),
        ]),
        evidence: vec![notice.failure.clone()], expected_subject: None, idempotency_key: Some(key),
    }
}

fn send_notice(store: &Store, notice: &Notice) -> Result<Option<crate::model::ClaimRecord>> {
    let mut input = notice_input(notice);
    match keys::lookup(store, input.idempotency_key.as_deref().unwrap(), &notice.owner,
        |receipt, key| keys::matches(store, &input, receipt, key))? {
        keys::Lookup::Recorded(receipt) => return Ok(Some(*receipt)),
        keys::Lookup::Blocked => return Ok(None),
        keys::Lookup::Vacant(_) => {},
    }
    // A held recipient cannot block or replace the canonical owner.
    if !store.fault_reviewer_is_declared(&notice.recipient)? {
        if notice.held { return Ok(None); }
        anyhow::bail!("original readiness owner is unavailable");
    }
    keys::append(store, &mut input, &notice.owner)
}

#[cfg(test)]
fn send_backup(
    store: &Store, opening: &Opening, failure: &str, phase: &str, backup: Option<&str>,
) -> Result<Option<String>> {
    let Some(backup) = backup else { return Ok(None) };
    ensure!(backup != opening.reviewer && store.fault_reviewer_is_declared(backup)?,
        "backup must be a distinct declared seat");
    let notice = Notice {
        episode: opening.episode.clone(), owner: opening.reviewer.clone(),
        phase: phase.into(), recipient: backup.into(), failure: failure.into(),
        held: true, recorded: false,
    };
    Ok(send_notice(store, &notice)?.map(|receipt| receipt.id))
}

fn drain_notices(admission: &mut Admission, journal: &Path, store: &Store, backup: Option<&str>) -> Result<()> {
    let mut queued = admission.clone();
    if let Some(opening) = admission.case.as_ref() {
    for phase in ["early", "weekly"] {
        let failure = match phase_receipt(store, opening, phase)? {
            keys::Lookup::Recorded(claim) => claim.id,
            keys::Lookup::Vacant(_) | keys::Lookup::Blocked => continue,
        };
        let recipients = std::iter::once((opening.reviewer.as_str(), false))
            .chain(backup.filter(|backup| *backup != opening.reviewer).map(|backup| (backup, true)));
        for (recipient, held) in recipients {
            if held && admission.weekly.recovered { continue; }
            if !queued.notices.iter().any(|notice| notice.episode == opening.episode
                && notice.phase == phase && (notice.recipient == recipient || (held && notice.held))) {
                queued.notices.push(Notice {
                    episode: opening.episode.clone(), owner: opening.reviewer.clone(),
                    phase: phase.into(), recipient: recipient.into(), failure: failure.clone(),
                    held, recorded: false,
                });
            }
        }
    }
    }
    if queued.notices.len() == admission.notices.len() && queued.notices.iter().all(|notice| notice.recorded) {
        return Ok(());
    }
    queued.persist(journal)?;
    for notice in &mut queued.notices {
        if notice.recorded {
            continue;
        }
        notice.recorded = send_notice(store, notice)?.is_some();
    }
    queued.persist(journal)?;
    *admission = queued;
    Ok(())
}

/// Replay already-admitted legacy work, leaving its writer as the sole notifier.
fn import_legacy(admission: &mut Admission, policy: &Policy, store: &Store, journal: &Path) -> Result<()> {
    if !admission.legacy_imported && admission.legacy_pending.is_none() {
        if let Some(Checkpoint::Cut { reading, pending: true }) = policy.read_checkpoint()? {
            admission.legacy_pending = Some(reading);
        }
        admission.persist(journal)?;
    }
    if let Some(reading) = admission.legacy_pending.clone() {
        drain_legacy(admission, policy, store, journal, &reading, reading.observed_at_ms)?;
    }
    if !admission.legacy_imported {
        let episode = crate::objective_faults::episode(
            store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member),
        );
        admission.legacy_draining = store.operational_failure(&episode)?
            .is_some_and(|failure| failure.status == "pending");
        admission.legacy_imported = true;
        admission.persist(journal)?;
    }
    Ok(())
}

/// Drain-before-enable: old episodes never acquire a shared-case primary notifier.
fn drain_legacy(admission: &mut Admission, policy: &Policy, store: &Store, journal: &Path,
    reading: &Reading, now: u64) -> Result<Value> {
    reading.check(&policy.config.member, now)?;
    admission.legacy_pending = Some(reading.clone());
    admission.persist(journal)?;
    let row = policy.route(store, reading)?;
    let episode = crate::objective_faults::episode(
        store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member),
    );
    admission.legacy_draining = store.operational_failure(&episode)?
        .is_some_and(|failure| failure.status == "pending");
    // Retained observations from the draining interval cannot open a later shared case.
    admission.through = admission.through.max(reading.end_ms);
    admission.weekly.through = admission.weekly.through.max(reading.end_ms);
    admission.legacy_pending = None;
    admission.persist(journal)?;
    Ok(row)
}

/// Fresh input must advance the drain watermark. Exact pending replay bypasses this check.
fn drain_fresh_legacy(
    admission: &mut Admission,
    policy: &Policy,
    store: &Store,
    journal: &Path,
    reading: &Reading,
    now: u64,
) -> Result<Value> {
    if reading.end_ms <= admission.weekly.through {
        return Ok(policy.row(None));
    }
    drain_legacy(admission, policy, store, journal, reading, now)
}

/// The enabled shared route replaces (and never calls) the legacy weekly dispatcher.
pub(super) fn run_sampler(policy: &Policy, store: &Store, now: u64) -> Result<Value> {
    ensure!(policy.config.shared_case, "shared readiness routing is disabled");
    let config = &policy.config;
    let retained_path = config.retained_pairs_file.as_ref().context("retained pair path missing")?;
    let journal = policy.checkpoint.with_file_name("readiness-case.json");
    let mut admission = Admission::load(&journal, config)?;
    import_legacy(&mut admission, policy, store, &journal)?;
    if admission.legacy_draining {
        let (reading, _) = policy.checked_reading(now)?;
        return drain_fresh_legacy(&mut admission, policy, store, &journal, &reading, now);
    }
    // Admitted work remains replayable even while the publisher is unavailable.
    admission.replay(&journal, |opening| record(store, config, opening))?;
    admission.replay_weekly(&journal, |opening, action| record_weekly(store, opening, action))?;
    drain_notices(&mut admission, &journal, store, config.backup.as_deref())?;
    let mut anchor = None;
    admission.reconcile(
        &journal,
        config,
        || {
            let (weekly, _) = policy.checked_reading(now)?;
            let retained = read_json_bounded::<RetainedPairs>(trusted_file(retained_path)?, RETAINED_MAX_BYTES)?;
            let summaries = retained.fresh_through(&weekly, now)?;
            anchor = Some(weekly);
            Ok(summaries)
        },
        |opening| record(store, config, opening),
    )?;
    drain_notices(&mut admission, &journal, store, config.backup.as_deref())?;
    let weekly = anchor.context("shared case has no weekly cut")?;
    admission.weekly(&journal, config, store, &weekly, now, |opening, action| {
        record_weekly(store, opening, action)
    })?;
    drain_notices(&mut admission, &journal, store, config.backup.as_deref())?;
    Ok(policy.with_binding(store, policy.row(Some(&weekly))).0)
}

#[test]
fn skipped_retained_miss_is_admitted_before_clear_and_replayed_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("graph.db");
    let journal = dir.path().join("early.json");
    let member = "member-a";
    let end = (MINUTES + 5) * MINUTE_MS;
    let pair = |end: u64, failures: u64| {
        let weekly = Reading {
            version: 1,
            objective: OBJECTIVE.into(),
            member: member.into(),
            start_ms: end - MINUTES * MINUTE_MS,
            end_ms: end,
            observed_at_ms: end,
            expected: MINUTES,
            known: MINUTES,
            good: MINUTES - failures,
        };
        json!({"weekly": weekly, "short": {
            "version": 1, "objective": OBJECTIVE, "member": member,
            "end_ms": end, "observed_at_ms": end,
            "observations": (1..=4).rev().map(|offset| json!({
                "at_ms": end - offset * MINUTE_MS + 1, "ready": offset > failures
            })).collect::<Vec<_>>()
        }})
    };
    let missed = pair(end, 3);
    let clear = pair(end + MINUTE_MS, 2);
    let anchor: Reading = serde_json::from_value(clear["weekly"].clone()).unwrap();
    let retained: RetainedPairs =
        serde_json::from_value(json!({"version":1,"pairs":[missed,clear]})).unwrap();
    let policy = Policy {
        config: ReadinessSloConfig {
            executor: "fixture".into(),
            member: member.into(),
            reviewer: "agent/fixture.owner".into(),
            evidence_file: dir.path().join("unused.json"),
            retained_pairs_file: None,
            shared_case: false,
            backup: None,
        },
        checkpoint: dir.path().join("weekly.json"),
    };
    // Old raw weekly v1 checkpoint must stay readable and byte-identical.
    let weekly_bytes = serde_json::to_vec(&anchor).unwrap();
    fs::write(&policy.checkpoint, &weekly_bytes).unwrap();
    assert!(matches!(
        policy.read_checkpoint().unwrap(),
        Some(Checkpoint::Legacy(_))
    ));
    let store = Store::open(&db, "fixture").unwrap();
    let declarations = "version 2\nagent \"fixture.owner\" { command \"true\" }\nagent \"fixture.generic\" { command \"true\"; handles-faults }\nagent \"fixture.backup\" { command \"true\" }\n";
    let intent = crate::graph::parse_intent(declarations, "fixture").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: declarations.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(
            &intent,
            &preview.subject_tokens,
            "retained-admission-owners",
        )
        .unwrap();

    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let source = || retained.fresh_through(&anchor, anchor.end_ms);
    assert!(
        admission
            .reconcile(&journal, &policy.config, source, |_| anyhow::bail!(
                "crash before append"
            ))
            .is_err()
    );
    let opening = admission.case.as_ref().unwrap();
    assert_eq!(opening.cut, end);
    let mut wrong_binding = policy.config.clone();
    wrong_binding.member = "member-b".into();
    assert!(Admission::load(&journal, &wrong_binding).is_err());
    let episode = opening.episode.clone();
    assert!(store.operational_failure(&episode).unwrap().is_none());
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    // Crash again, this time after native acceptance and before the journal receipt.
    assert!(
        admission
            .reconcile(
                &journal,
                &policy.config,
                || panic!("pending first"),
                |opening| {
                    record(&store, &policy.config, opening)?;
                    anyhow::bail!("crash after append")
                }
            )
            .is_err()
    );
    let first = store.operational_failure(&episode).unwrap().unwrap();
    assert_eq!(first.reviewer, policy.config.reviewer);
    let index = store.index().unwrap();
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    assert!(!admission.case.as_ref().unwrap().recorded);
    let drain = |opening: &Opening| record(&store, &policy.config, opening);
    // The old miss is now stale. Its admitted operation still replays before a fresh clear.
    let later = pair(end + 4 * MINUTE_MS, 2);
    let next_anchor: Reading = serde_json::from_value(later["weekly"].clone()).unwrap();
    let next: RetainedPairs = serde_json::from_value(json!({"version":1,"pairs":[later]})).unwrap();
    admission
        .reconcile(
            &journal,
            &policy.config,
            || next.fresh_through(&next_anchor, next_anchor.end_ms),
            drain,
        )
        .unwrap();
    assert_eq!(admission.through, next_anchor.end_ms);
    assert!(admission.case.as_ref().unwrap().recorded);
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(
        store
            .operational_failure(&episode)
            .unwrap()
            .unwrap()
            .request,
        first.request
    );
    assert_eq!(fs::read(&policy.checkpoint).unwrap(), weekly_bytes);
    let opening = admission.case.as_ref().unwrap();
    let backup = "agent/fixture.backup";
    assert_eq!(
        send_backup(&store, opening, &first.request, "early", None).unwrap(),
        None
    );
    assert!(
        send_backup(
            &store,
            opening,
            &first.request,
            "early",
            Some(&opening.reviewer)
        )
        .is_err()
    );
    assert!(
        send_backup(
            &store,
            opening,
            &first.request,
            "early",
            Some("agent/fixture.absent")
        )
        .is_err()
    );
    assert!(store.messages(Some(backup), true).unwrap().is_empty());
    let receipt = send_backup(&store, opening, &first.request, "early", Some(backup))
        .unwrap()
        .unwrap();
    let after_send = store.index().unwrap();
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    assert_eq!(
        send_backup(&store, opening, &first.request, "early", Some(backup))
            .unwrap()
            .as_deref(),
        Some(receipt.as_str())
    );
    assert_eq!(store.index().unwrap(), after_send);
    let messages = store.messages(Some(backup), true).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].from, "daemon/runtime");
    assert_eq!(messages[0].to, backup);
    assert!(crate::silent::is_held(&messages[0]));
    let faults = store.fault_snapshot(u128::from(now_ms()) + 1).unwrap();
    let matching: Vec<_> = faults
        .iter()
        .filter(|fault| fault.item.subject == "daemon/fixture")
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(
        matching[0].owner.as_deref(),
        Some(opening.reviewer.as_str())
    );

    assert_eq!(matching[0].item.episode, episode);

    // The weekly path sees an early receipt still pending locally. It replays that first.
    admission.case.as_mut().unwrap().recorded = false;
    admission.persist(&journal).unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let missed: Reading =
        serde_json::from_value(pair(end + 5 * MINUTE_MS, 11)["weekly"].clone()).unwrap();
    assert!(
        admission
            .weekly(
                &journal,
                &policy.config,
                &store,
                &missed,
                missed.end_ms,
                |opening, action| {
                    record_weekly(&store, opening, action)?;
                    anyhow::bail!("crash after weekly append")
                }
            )
            .is_err()
    );
    let after_escalation = store.index().unwrap();
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let mut unknown: Reading =
        serde_json::from_value(pair(end + 6 * MINUTE_MS, 0)["weekly"].clone()).unwrap();
    unknown.known = 0;
    unknown.good = 0;
    admission
        .weekly(
            &journal,
            &policy.config,
            &store,
            &unknown,
            unknown.end_ms,
            |opening, action| record_weekly(&store, opening, action),
        )
        .unwrap();
    assert_eq!(store.index().unwrap(), after_escalation);
    let weekly = store.operational_failure(&episode).unwrap().unwrap();
    assert_eq!(weekly.status, "pending");
    assert_eq!(weekly.reviewer, policy.config.reviewer);
    assert_ne!(weekly.request, first.request);
    let faults: Vec<_> = store
        .fault_snapshot(u128::from(now_ms()) + 1)
        .unwrap()
        .into_iter()
        .filter(|fault| fault.item.subject == "daemon/fixture")
        .collect();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].item.episode, episode);
    assert_eq!(
        faults[0].owner.as_deref(),
        Some(policy.config.reviewer.as_str())
    );
    let opening = admission.case.as_ref().unwrap();
    let weekly_notice = send_backup(&store, opening, &weekly.request, "weekly", Some(backup))
        .unwrap()
        .unwrap();
    assert_ne!(weekly_notice, receipt);
    let messages = store.messages(Some(backup), true).unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(crate::silent::is_held));

    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    let owners = store.messages(Some(&policy.config.reviewer), true).unwrap();
    assert_eq!(owners.len(), 2, "one actionable notice per phase");
    assert!(owners.iter().all(|message| !crate::silent::is_held(message)));
    assert_eq!(store.messages(Some(backup), true).unwrap().len(), 2);
    let after_notices = store.index().unwrap();
    // Simulate native acceptance before the local notice receipt was persisted.
    for notice in &mut admission.notices { notice.recorded = false; }
    admission.persist(&journal).unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    assert_eq!(store.index().unwrap(), after_notices);

    let met: Reading =
        serde_json::from_value(pair(end + 7 * MINUTE_MS, 0)["weekly"].clone()).unwrap();
    assert!(
        admission
            .weekly(
                &journal,
                &policy.config,
                &store,
                &met,
                met.end_ms,
                |_, _| { anyhow::bail!("crash before recovery append") }
            )
            .is_err()
    );
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    assert!(
        admission
            .weekly(
                &journal,
                &policy.config,
                &store,
                &met,
                met.end_ms,
                |opening, action| {
                    record_weekly(&store, opening, action)?;
                    anyhow::bail!("crash after recovery append")
                }
            )
            .is_err()
    );
    let after_recovery = store.index().unwrap();
    drop(store);

    let store = Store::open(&db, "fixture").unwrap();
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    admission
        .weekly(
            &journal,
            &policy.config,
            &store,
            &met,
            met.end_ms,
            |opening, action| record_weekly(&store, opening, action),
        )
        .unwrap();
    assert_eq!(store.index().unwrap(), after_recovery);
    assert!(admission.weekly.recovered);
    assert_eq!(
        store.operational_failure(&episode).unwrap().unwrap().status,
        "resolved"
    );
    assert!(
        store
            .fault_snapshot(u128::from(now_ms()) + 1)
            .unwrap()
            .iter()
            .all(|fault| fault.item.subject != "daemon/fixture")
    );
    for failure in [&first.request, &weekly.request] {
        assert!(
            store
                .operation_claim(&format!("{failure}:recovered"))
                .unwrap()
                .is_some()
        );
    }
    let opening = admission.case.as_ref().unwrap();
    assert_eq!(
        send_backup(&store, opening, &weekly.request, "weekly", Some(backup)).unwrap(),
        Some(weekly_notice)
    );
    assert_eq!(store.index().unwrap(), after_recovery);
    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);
    assert_eq!(store.messages(Some(backup), true).unwrap().len(), 2);
    assert_eq!(store.index().unwrap(), after_recovery);
    assert_eq!(fs::read(&policy.checkpoint).unwrap(), weekly_bytes);
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn sampler_replays_pending_legacy_cut_without_shared_primary_notice() {
        let (dir, policy, store) = fixture();
        let end = (MINUTES + 10) * MINUTE_MS;
        policy.checkpoint(&reading(end, 11), true).unwrap();
        // The real sampler must replay admitted legacy work before fresh evidence.
        assert!(run_sampler(&policy, &store, end + MINUTE_MS).is_err());
        let admission = Admission::load(
            &dir.path().join("readiness-case.json"), &policy.config,
        ).unwrap();
        assert!(admission.legacy_draining);
        assert!(admission.case.is_none());
        assert!(store.messages(Some(&policy.config.reviewer), true).unwrap().is_empty());
        assert!(!store.fault_delivery_snapshot(u128::from(now_ms()) + 1).unwrap().is_empty());
    }

    #[test]
    fn ordinary_marker_and_default_off_keep_normal_fault_delivery() {
        let (_dir, policy, store) = fixture();
        let end = (MINUTES + 10) * MINUTE_MS;
        policy.route(&store, &reading(end, 11)).unwrap();
        let legacy = crate::objective_faults::episode(
            store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member),
        );
        let failure = store.operational_failure(&legacy).unwrap().unwrap();
        store.append_claim(&ClaimInput {
            subject: "custom/fixture/record".into(), kind: "custom.fixture.record".into(),
            actor: Some(policy.config.reviewer.clone()),
            fields: BTreeMap::from([("episode".into(), json!("slo-objective/readiness-case/example"))]),
            evidence: vec![failure.request.clone()], expected_subject: None,
            idempotency_key: Some(format!("{}:readiness-case", failure.request)),
        }).unwrap();
        let faults = store.fault_delivery_snapshot(u128::from(now_ms()) + 1).unwrap();
        assert!(faults.iter().any(|fault| fault.item.episode == failure.request));
    }

    pub(super) fn fixture() -> (tempfile::TempDir, Policy, Store) {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy {
            config: serde_json::from_value(json!({
                "executor": "fixture", "member": "member-a",
                "reviewer": "agent/fixture.owner",
                "evidence_file": "/run/fractal-slo-evidence/weekly.json",
                "retained_pairs_file": "/run/fractal-slo-evidence/retained.json",
                "shared_case": true
            })).unwrap(),
            checkpoint: dir.path().join("weekly.json"),
        };
        let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
        let declarations = "version 2\nagent \"fixture.owner\" { command \"true\" }\n";
        let intent = crate::graph::parse_intent(declarations, "fixture").unwrap();
        let preview = store.mission(&intent, IntentInput {
            kdl: declarations.into(), source_name: None,
        }).unwrap();
        store.apply(&intent, &preview.subject_tokens, "migration-owner").unwrap();
        (dir, policy, store)
    }

    pub(super) fn reading(end_ms: u64, failed: u64) -> Reading {
        Reading {
            version: 1, objective: OBJECTIVE.into(), member: "member-a".into(),
            start_ms: end_ms - MINUTES * MINUTE_MS, end_ms, observed_at_ms: end_ms,
            expected: MINUTES, known: MINUTES, good: MINUTES - failed,
        }
    }

    #[test]
    fn legacy_open_at_enable_drains_before_new_phase_notices() {
        let (dir, policy, store) = fixture();
        let end = (MINUTES + 10) * MINUTE_MS;
        policy.route(&store, &reading(end, 11)).unwrap();
        let journal = dir.path().join("case.json");
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut admission, &policy, &store, &journal).unwrap();
        assert!(admission.legacy_draining);
        assert!(!admission.plan(&policy.config, ShortSummary {
            end_ms: end, known: 5, failed: 3,
        }));
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        assert!(store.messages(Some(&policy.config.reviewer), true).unwrap().is_empty());
        let met = reading(end + MINUTE_MS, 0);
        drain_legacy(&mut admission, &policy, &store, &journal, &met, met.end_ms).unwrap();
        assert!(!admission.legacy_draining);
        assert!(!admission.plan(&policy.config, ShortSummary {
            end_ms: end, known: 5, failed: 3,
        }));
        assert!(admission.plan(&policy.config, ShortSummary {
            end_ms: end + 2 * MINUTE_MS, known: 5, failed: 3,
        }));
        admission.persist(&journal).unwrap();
        admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        let missed = reading(end + 3 * MINUTE_MS, 11);
        admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);
    }

    #[test]
    fn weekly_only_case_has_one_phase_notice_and_no_generic_delivery() {
        let (dir, policy, store) = fixture();
        let journal = dir.path().join("case.json");
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        let missed = reading((MINUTES + 10) * MINUTE_MS, 11);
        admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        drain_notices(&mut admission, &journal, &store, Some("agent/fixture.unavailable")).unwrap();
        drain_notices(&mut admission, &journal, &store, Some("agent/fixture.unavailable")).unwrap();
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
        assert!(store.fault_delivery_snapshot(u128::from(now_ms()) + 1).unwrap().is_empty());
        let first_episode = admission.case.as_ref().unwrap().episode.clone();
        let met = reading(missed.end_ms + MINUTE_MS, 0);
        admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        assert!(admission.weekly.recovered);
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        let next_miss = reading(met.end_ms + MINUTE_MS, 11);
        // No plan/reconcile call or short summary occurs anywhere in this lifecycle.
        admission.weekly(&journal, &policy.config, &store, &next_miss, next_miss.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        assert_ne!(admission.case.as_ref().unwrap().episode, first_episode);
        assert!(admission.weekly.escalated);
        assert!(!admission.weekly.recovered);
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);
    }

    #[test]
    fn pending_legacy_miss_is_imported_before_fresh_recovery() {
        let (dir, policy, store) = fixture();
        let end = (MINUTES + 10) * MINUTE_MS;
        policy.checkpoint(&reading(end, 11), true).unwrap();
        let legacy_bytes = fs::read(&policy.checkpoint).unwrap();
        let journal = dir.path().join("case.json");
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut admission, &policy, &store, &journal).unwrap();
        let legacy_episode = crate::objective_faults::episode(
            store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member),
        );
        assert!(store.operational_failure(&legacy_episode).unwrap().is_some());
        assert!(admission.case.is_none());
        assert!(admission.legacy_draining);
        assert!(admission.legacy_imported);
        let met = reading(end + MINUTE_MS, 0);
        drain_legacy(&mut admission, &policy, &store, &journal, &met, met.end_ms).unwrap();
        let mut restarted = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut restarted, &policy, &store, &journal).unwrap();
        assert!(!restarted.legacy_draining);
        assert!(store.messages(Some(&policy.config.reviewer), true).unwrap().is_empty());
        assert_eq!(store.operational_failure(&legacy_episode).unwrap().unwrap().status, "resolved");
        assert_eq!(fs::read(&policy.checkpoint).unwrap(), legacy_bytes);
    }

    #[test]
    fn legacy_drain_rejects_fresh_rewind_and_rewritten_equal_cut_before_routing() {
        let (dir, policy, store) = fixture();
        let start = (MINUTES + 10) * MINUTE_MS;
        let legacy = reading(start, 11);
        policy.checkpoint(&legacy, false).unwrap();
        policy.route(&store, &legacy).unwrap();
        let legacy_bytes = fs::read(&policy.checkpoint).unwrap();
        let episode = crate::objective_faults::episode(
            store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member),
        );
        let journal = dir.path().join("case.json");
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut admission, &policy, &store, &journal).unwrap();

        let newest = reading(start + 2 * MINUTE_MS, 11);
        drain_fresh_legacy(&mut admission, &policy, &store, &journal, &newest, newest.end_ms).unwrap();
        let committed = fs::read(&journal).unwrap();
        let index = store.index().unwrap();
        for end in [start + MINUTE_MS, newest.end_ms] {
            let stale_met = reading(end, 0);
            assert!(policy.admit(&stale_met, newest.end_ms).unwrap());
            let row = drain_fresh_legacy(
                &mut admission, &policy, &store, &journal, &stale_met, newest.end_ms,
            ).unwrap();
            assert_eq!(row["status"], "no-data");
            assert!(admission.legacy_draining);
            assert_eq!(store.index().unwrap(), index);
            assert_eq!(fs::read(&journal).unwrap(), committed);
            assert_eq!(store.operational_failure(&episode).unwrap().unwrap().status, "pending");
        }

        let duplicate = drain_fresh_legacy(
            &mut admission, &policy, &store, &journal, &newest, newest.end_ms,
        ).unwrap();
        assert_eq!(duplicate["status"], "no-data");
        assert_eq!(store.index().unwrap(), index);
        assert_eq!(fs::read(&journal).unwrap(), committed);
        assert_eq!(store.operational_failure(&episode).unwrap().unwrap().status, "pending");

        let next = reading(newest.end_ms + MINUTE_MS, 0);
        admission.legacy_pending = Some(next);
        admission.persist(&journal).unwrap();
        let mut restarted = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut restarted, &policy, &store, &journal).unwrap();
        assert!(restarted.legacy_pending.is_none());
        assert!(!restarted.legacy_draining);
        assert_eq!(store.operational_failure(&episode).unwrap().unwrap().status, "resolved");
        assert_eq!(fs::read(&policy.checkpoint).unwrap(), legacy_bytes);
    }

    fn retained_lag_after_weekly_recovery(restart: bool) {
        let (dir, policy, store) = fixture();
        let journal = dir.path().join("case.json");
        let end = (MINUTES + 10) * MINUTE_MS;
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        let miss = |end_ms| ShortSummary { end_ms, known: 5, failed: 3 };
        admission.reconcile(&journal, &policy.config, || Ok(vec![miss(end)]),
            |opening| record(&store, &policy.config, opening)).unwrap();
        let weekly_miss = reading(end, 11);
        admission.weekly(&journal, &policy.config, &store, &weekly_miss, end,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        // Weekly publication advances to T2 while its short companion is still at T0.
        let recovered = reading(end + 2 * MINUTE_MS, 0);
        admission.weekly(&journal, &policy.config, &store, &recovered, recovered.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        assert!(admission.weekly.recovered);
        assert_eq!(admission.through, recovered.end_ms);
        if restart {
            admission = Admission::load(&journal, &policy.config).unwrap();
        }
        let committed = fs::read(&journal).unwrap();
        let index = store.index().unwrap();
        // Delayed retained publication catches up with still-fresh T1 and T2 misses.
        admission.reconcile(&journal, &policy.config,
            || Ok(vec![miss(end + MINUTE_MS), miss(recovered.end_ms)]),
            |_| panic!("late companion cannot reopen the recovered case")).unwrap();
        assert_eq!(fs::read(&journal).unwrap(), committed);
        assert_eq!(store.index().unwrap(), index);
        assert!(admission.weekly.recovered);
        let fresh = recovered.end_ms + MINUTE_MS;
        admission.reconcile(&journal, &policy.config, || Ok(vec![miss(fresh)]),
            |opening| record(&store, &policy.config, opening)).unwrap();
        assert_eq!(admission.case.as_ref().unwrap().cut, fresh);
        assert!(!admission.weekly.recovered);
    }

    #[test]
    fn delayed_retained_cuts_before_or_at_recovery_cannot_reopen() {
        retained_lag_after_weekly_recovery(false);
    }

    #[test]
    fn recovery_short_watermark_survives_restart_before_retained_catchup() {
        retained_lag_after_weekly_recovery(true);
    }

    #[test]
    fn unavailable_backup_does_not_accumulate_across_recovered_cases() {
        let (dir, policy, store) = fixture();
        let journal = dir.path().join("case.json");
        let backup = Some("agent/fixture.unavailable");
        let mut admission = Admission::load(&journal, &policy.config).unwrap();
        for episode in 0..64 {
            let end = (MINUTES + 10 + episode * 3) * MINUTE_MS;
            assert!(admission.plan(&policy.config, ShortSummary {
                end_ms: end, known: 5, failed: 3,
            }));
            admission.persist(&journal).unwrap();
            admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
            drain_notices(&mut admission, &journal, &store, backup).unwrap();
            assert_eq!(admission.notices.iter().filter(|notice| notice.held).count(), 1);
            let met = reading(end + MINUTE_MS, 0);
            admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
                |opening, action| record_weekly(&store, opening, action)).unwrap();
            drain_notices(&mut admission, &journal, &store, backup).unwrap();
            assert!(admission.notices.iter().all(|notice| !notice.held));
            assert!(fs::metadata(&journal).unwrap().len() < CASE_JOURNAL_MAX_BYTES);
            admission = Admission::load(&journal, &policy.config).unwrap();
        }
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 64);
        assert!(store.messages(backup, true).unwrap().is_empty());
    }
}

#[cfg(test)]
#[path = "readiness_case_key_tests.rs"]
mod key_tests;
