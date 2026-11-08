//! Durable shared readiness cases and phase-specific notice delivery.
use super::*;
use crate::model::ClaimInput;
#[cfg(test)]
use crate::model::IntentInput;
use std::collections::BTreeMap;

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
    #[serde(default)]
    adopted: Option<String>,
    #[serde(default)]
    adopted_episode: Option<String>,
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

    fn plan(&mut self, config: &ReadinessSloConfig, summary: ShortSummary) -> bool {
        if summary.end_ms <= self.through {
            return false;
        }
        self.through = summary.end_ms;
        if self.weekly.recovered {
            self.case = None;
            self.weekly = Weekly::default();
            self.notices.retain(|notice| !notice.recorded && !notice.held);
        }
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
                adopted: None,
                adopted_episode: None,
            });
        }
        true
    }

    fn reconcile(
        &mut self,
        path: &Path,
        config: &ReadinessSloConfig,
        next: impl FnOnce() -> Result<Vec<ShortSummary>>,
        dispatch: impl Fn(&Opening) -> Result<()>,
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
        dispatch: impl Fn(&Opening, &WeeklyAction) -> Result<()>,
    ) -> Result<()> {
        self.replay(path, |opening| record(store, config, opening))?;
        self.replay_weekly(path, &dispatch)?;
        reading.check(&self.member, now)?;
        if self.case.is_none() && reading.status() == "missed" {
            self.case = Some(Opening {
                episode: format!("slo-objective/readiness-case/{}/{OBJECTIVE}/{}/{}",
                    self.executor, self.member, reading.end_ms),
                reviewer: config.reviewer.clone(), cut: reading.end_ms,
                failed: 0, known: 0, recorded: true, adopted: None,
                adopted_episode: None,
            });
        }
        if reading.end_ms <= self.weekly.through || self.case.is_none() {
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
        dispatch: impl FnOnce(&Opening, &WeeklyAction) -> Result<()>,
    ) -> Result<()> {
        let Some(action) = self.weekly.pending.as_ref() else {
            return Ok(());
        };
        let opening = self.case.as_ref().context("weekly phase has no case")?;
        dispatch(opening, action)?;
        let mut settled = self.clone();
        match action {
            WeeklyAction::Escalate { .. } => settled.weekly.escalated = true,
            WeeklyAction::Recover { .. } => {
                settled.weekly.recovered = true;
                // Informational delivery never survives the case it describes.
                settled.notices.retain(|notice| !notice.held || notice.episode != opening.episode);
            }
        }
        settled.weekly.pending = None;
        settled.persist(path)?;
        *self = settled;
        Ok(())
    }

    fn replay(&mut self, path: &Path, dispatch: impl FnOnce(&Opening) -> Result<()>) -> Result<()> {
        let mut settled = self.clone();
        if let Some(opening) = settled.case.as_mut().filter(|opening| !opening.recorded) {
            dispatch(opening)?;
            opening.recorded = true;
            settled.persist(path)?;
            *self = settled;
        }
        Ok(())
    }
}

fn record(store: &Store, config: &ReadinessSloConfig, opening: &Opening) -> Result<()> {
    ensure!(
        store.origin() == config.executor,
        "early readiness store is not the executor"
    );
    let key = format!("{}:early", opening.episode);
    if store.operation_claim(&key)?.is_some() {
        return Ok(());
    }
    ensure!(
        store.fault_reviewer_is_declared(&opening.reviewer)?,
        "early readiness requires the originally configured active reviewer"
    );
    let source = format!("daemon/{}", store.origin());
    store.append_claim(&ClaimInput {
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
    })?;
    Ok(())
}

fn record_weekly(store: &Store, opening: &Opening, action: &WeeklyAction) -> Result<()> {
    let source = format!("daemon/{}", store.origin());
    let early = store.operation_claim(&format!("{}:early", opening.episode))?;
    let weekly_key = format!("{}:weekly", opening.episode);
    match action {
        WeeklyAction::Escalate { cut } => {
            if store.operation_claim(&weekly_key)?.is_some() {
                return Ok(());
            }
            ensure!(
                store.fault_reviewer_is_declared(&opening.reviewer)?,
                "original reviewer is unavailable"
            );
            store.append_claim(&ClaimInput {
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
                evidence: early.into_iter().map(|claim| claim.id).collect(),
                expected_subject: None,
                idempotency_key: Some(weekly_key),
            })?;
        }
        WeeklyAction::Recover { cut } => {
            let weekly = opening.adopted.clone().or(
                store.operation_claim(&weekly_key)?.map(|claim| claim.id)
            );
            for prior in early.into_iter().map(|claim| claim.id).chain(weekly) {
                let key = format!("{prior}:recovered");
                store.append_claim(&ClaimInput {
                    subject: source.clone(),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(opening.adopted_episode.as_deref().unwrap_or(&opening.episode))),
                        ("failure".into(), json!(prior)),
                        (
                            "reason".into(),
                            json!(format!("Measured weekly recovery at cut {cut}")),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(key),
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn send_backup(
    store: &Store,
    opening: &Opening,
    failure: &str,
    phase: &str,
    backup: Option<&str>,
) -> Result<Option<String>> {
    use sha2::Digest as _;
    let Some(backup) = backup else {
        return Ok(None);
    };
    ensure!(
        backup != opening.reviewer && store.fault_reviewer_is_declared(backup)?,
        "backup must be a distinct declared seat"
    );
    let key = format!("{}:{phase}:{backup}", opening.episode);
    let message = crate::store::message_routing::send(
        store,
        &ClaimInput {
            subject: format!(
                "message/{}",
                &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16]
            ),
            kind: "message.sent".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("daemon/runtime")),
                ("to".into(), json!(backup)),
                ("title".into(), json!(format!("API readiness {phase}"))),
                (
                    "content".into(),
                    json!(format!(
                        "Informational only. Readiness case {} is owned by {}.",
                        opening.episode, opening.reviewer
                    )),
                ),
                ("status".into(), json!("sent")),
                ("tags".into(), json!(["st3-silent"])),
            ]),
            evidence: vec![failure.into()],
            expected_subject: None,
            idempotency_key: Some(key),
        },
    )?;
    Ok(Some(message.id))
}

fn drain_notices(admission: &mut Admission, journal: &Path, store: &Store, backup: Option<&str>) -> Result<()> {
    use sha2::Digest as _;
    let mut queued = admission.clone();
    if let Some(opening) = admission.case.as_ref() {
    for phase in ["early", "weekly"] {
        let failure = if phase == "weekly" {
            opening.adopted.clone().or(store.operation_claim(&format!("{}:{phase}", opening.episode))?.map(|claim| claim.id))
        } else {
            store.operation_claim(&format!("{}:{phase}", opening.episode))?.map(|claim| claim.id)
        };
        let Some(failure) = failure else { continue };
        let recipients = std::iter::once((opening.reviewer.as_str(), false))
            .chain(backup.filter(|backup| *backup != opening.reviewer).map(|backup| (backup, true)));
        for (recipient, held) in recipients {
            if held && admission.weekly.recovered { continue; }
            if !queued.notices.iter().any(|notice| notice.episode == opening.episode
                && notice.phase == phase && (notice.recipient == recipient || (held && notice.held))) {
                let legacy_notice = if !held && phase == "weekly" {
                    if let Some(receipt) = &opening.adopted {
                        let key = format!("st3-fault:{}:daemon/{}:{receipt}", opening.reviewer, store.origin());
                        store.message_sent(&format!("message/{}", &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16]))?
                    } else { false }
                } else { false };
                queued.notices.push(Notice {
                    episode: opening.episode.clone(), owner: opening.reviewer.clone(),
                    phase: phase.into(), recipient: recipient.into(), failure: failure.clone(),
                    held, recorded: legacy_notice,
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
        let key = if notice.held {
            format!("{}:{}:{}", notice.episode, notice.phase, notice.recipient)
        } else {
            format!("{}:notice:{}", notice.episode, notice.phase)
        };
        if let Some(receipt) = store.operation_claim(&key)? {
            ensure!(receipt.kind == "message.sent"
                && receipt.body["fields"]["from"] == "daemon/runtime"
                && receipt.body["fields"]["to"] == notice.recipient,
                "readiness notice receipt does not match its intent");
            notice.recorded = true;
            continue;
        }
        // A held recipient cannot block or replace the canonical owner.
        if !store.fault_reviewer_is_declared(&notice.recipient)? {
            if notice.held { continue; }
            anyhow::bail!("original readiness owner is unavailable");
        }
        let tags = if notice.held { vec!["st3-silent"] } else { vec![] };
        crate::store::message_routing::send(store, &ClaimInput {
            subject: format!("message/{}", &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16]),
            kind: "message.sent".into(),
            actor: Some("daemon/runtime".into()),
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
                ("tags".into(), json!(tags)),
            ]),
            evidence: vec![notice.failure.clone()],
            expected_subject: None,
            idempotency_key: Some(key),
        })?;
        notice.recorded = true;
    }
    queued.persist(journal)?;
    *admission = queued;
    Ok(())
}

/// Transfer admitted legacy work without rewriting its checkpoint or reading a newer cut.
fn import_legacy(admission: &mut Admission, policy: &Policy, store: &Store, journal: &Path) -> Result<()> {
    if !admission.legacy_imported {
        if admission.legacy_pending.is_none() {
            if let Some(Checkpoint::Cut { reading, pending: true }) = policy.read_checkpoint()? {
                admission.legacy_pending = Some(reading);
            }
            admission.persist(journal)?;
        }
        if let Some(reading) = &admission.legacy_pending {
            reading.check(&policy.config.member, reading.observed_at_ms)?;
            // Only the already-admitted legacy transition is replayed, never fresh dispatch.
            policy.route(store, reading)?;
        }
    }
    if admission.case.is_none() {
        let legacy_episode = crate::objective_faults::episode(
            store.origin(), &format!("{OBJECTIVE}/{}", policy.config.member)
        );
        if let Some(failure) = store.operational_failure(&legacy_episode)?
            .filter(|failure| failure.status == "pending")
        {
            admission.case = Some(Opening {
                episode: format!("slo-objective/readiness-case/adopted/{}", failure.request),
                reviewer: failure.reviewer,
                cut: 0, failed: 0, known: 0, recorded: true,
                adopted: Some(failure.request),
                adopted_episode: Some(legacy_episode),
            });
            admission.weekly.escalated = true;
        }
    }
    if !admission.legacy_imported {
        admission.legacy_pending = None;
        admission.legacy_imported = true;
        admission.persist(journal)?;
    }
    Ok(())
}

/// The enabled shared route replaces (and never calls) the legacy weekly dispatcher.
pub(super) fn run_sampler(policy: &Policy, store: &Store, now: u64) -> Result<Value> {
    ensure!(policy.config.shared_case, "shared readiness routing is disabled");
    store.readiness_shared_case.store(true, Ordering::Relaxed);
    let config = &policy.config;
    let retained_path = config.retained_pairs_file.as_ref().context("retained pair path missing")?;
    let journal = policy.checkpoint.with_file_name("readiness-case.json");
    let mut admission = Admission::load(&journal, config)?;
    import_legacy(&mut admission, policy, store, &journal)?;
    if let Some(opening) = &admission.case
        && let Some(receipt) = &opening.adopted
    {
            store.append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin()),
                kind: "readiness.case.adopted".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([("episode".into(), json!(opening.episode))]),
                evidence: vec![receipt.clone()], expected_subject: None,
                idempotency_key: Some(format!("{receipt}:readiness-case")),
            })?;
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);

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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    store.readiness_shared_case.store(true, Ordering::Relaxed);
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
    fn sampler_adopts_pending_legacy_failure_through_registered_kind() {
        let (dir, policy, store) = fixture();
        let end = (MINUTES + 10) * MINUTE_MS;
        policy.checkpoint(&reading(end, 11), true).unwrap();
        // Fresh root-controlled publisher evidence is deliberately unavailable.
        // The real sampler must still finish native adoption and notice replay first.
        assert!(run_sampler(&policy, &store, end + MINUTE_MS).is_err());
        let admission = Admission::load(
            &dir.path().join("readiness-case.json"), &policy.config,
        ).unwrap();
        let opening = admission.case.as_ref().unwrap();
        let receipt = opening.adopted.as_ref().unwrap();
        let adoption = store.operation_claim(&format!("{receipt}:readiness-case")).unwrap().unwrap();
        assert_eq!(adoption.kind, "readiness.case.adopted");
        assert_eq!(adoption.actor.as_deref(), Some("daemon/runtime"));
        let faults = store.fault_snapshot(u128::from(now_ms()) + 1).unwrap();
        assert!(faults.iter().any(|fault| fault.item.episode == opening.episode));
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
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
        store.readiness_shared_case.store(true, Ordering::Relaxed);
        let faults = store.fault_snapshot(u128::from(now_ms()) + 1).unwrap();
        assert!(faults.iter().any(|fault| fault.item.episode == failure.request));
        assert!(faults.iter().all(|fault| !store.readiness_case_manages(&fault.item.episode)));
        store.readiness_shared_case.store(false, Ordering::Relaxed);
        assert!(!store.readiness_case_manages("slo-objective/readiness-case/example"));
    }

    fn fixture() -> (tempfile::TempDir, Policy, Store) {
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

    fn reading(end_ms: u64, failed: u64) -> Reading {
        Reading {
            version: 1, objective: OBJECTIVE.into(), member: "member-a".into(),
            start_ms: end_ms - MINUTES * MINUTE_MS, end_ms, observed_at_ms: end_ms,
            expected: MINUTES, known: MINUTES, good: MINUTES - failed,
        }
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
        let failure = store.operational_failure(&legacy_episode).unwrap().unwrap();
        assert_eq!(admission.case.as_ref().unwrap().adopted.as_deref(), Some(failure.request.as_str()));
        assert!(admission.legacy_imported);
        drain_notices(&mut admission, &journal, &store, None).unwrap();
        let met = reading(end + MINUTE_MS, 0);
        admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
            |opening, action| record_weekly(&store, opening, action)).unwrap();
        let mut restarted = Admission::load(&journal, &policy.config).unwrap();
        import_legacy(&mut restarted, &policy, &store, &journal).unwrap();
        drain_notices(&mut restarted, &journal, &store, None).unwrap();
        assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
        assert_eq!(store.operational_failure(&legacy_episode).unwrap().unwrap().status, "resolved");
        assert_eq!(fs::read(&policy.checkpoint).unwrap(), legacy_bytes);
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
