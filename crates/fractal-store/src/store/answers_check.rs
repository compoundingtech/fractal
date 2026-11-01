//! Two members that answer differently from the same history have found a bug in trimming,
//! replication or a reader, never something to repair from a peer. The daily answers check
//! (`smallclaims::sync`) hands what it found here. Each distinct kind/key/answer signature opens
//! an operational fault on this member's `daemon/NODE` subject. Repeats record local diagnostic
//! occurrences on that fault without sending another message. A later agreeing check ends the
//! pair's open faults.
use super::*;
use smallclaims::answers::{AnswersCheck, KindReport};
use smallclaims::store::checkpoint::claims_of_kind_in_order;

pub const ANSWERS_CONDITION: &str = "answers-disagreement";
/// Every answers episode starts with this, so its failure and recovery read apart from the
/// daemon's other faults.
pub const EPISODE_PREFIX: &str = "answers-disagreement/";

/// `[trim] notify`, set at startup: the agent that owns a fault local trimming raises, such as an
/// answers disagreement.
#[derive(Default)]
pub struct TrimSettings {
    pub notify: Mutex<Option<String>>,
}

/// The stable prefix shared by all disagreement signatures of a member pair.
pub fn episode(local: &str, peer: &str) -> String {
    let (first, second) = if local <= peer { (local, peer) } else { (peer, local) };
    let digest = hex::encode(Sha256::digest(format!("{first}\n{second}").as_bytes()));
    format!("{EPISODE_PREFIX}{}", &digest[..32])
}

fn answer_signature(local: &str, peer: &str, differences: &[KindReport]) -> Result<String> {
    let swap = local > peer;
    let mut kinds = Vec::with_capacity(differences.len());
    for report in differences {
        let mut keys = report.keys.iter().map(|key| {
            let (ours, theirs) = (key.ours.as_deref(), key.theirs.as_deref());
            let (first, second) = if swap { (theirs, ours) } else { (ours, theirs) };
            (key.subject.as_str(), key.origin.as_str(), first, second)
        }).collect::<Vec<_>>();
        keys.sort_unstable();
        let (ours, theirs) = (
            report.ours.as_ref().map(|answer| answer.digest.as_str()),
            report.theirs.as_ref().map(|answer| answer.digest.as_str()),
        );
        let (first, second) = if swap { (theirs, ours) } else { (ours, theirs) };
        kinds.push((report.kind.as_str(), first, second, keys));
    }
    kinds.sort_unstable();
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&kinds)?)))
}

struct OpenAnswersFailure {
    id: String,
    episode: String,
    signature: Option<String>,
    initial_cut: Option<u128>,
}

/// The subjects whose answers fold claims rather than read one: seats (harness state, status
/// history, working since) and steps (timing).
const FOLDED_SUBJECTS: [&str; 2] = ["agent/", "step-run/"];

/// Only production-reader-complete replacements may suppress a kind's inventory comparison.
/// Other kinds keep their conservative inventory check, even when history counts can differ.
const SLOT_READERS: [&str; 1] = ["observer.observed"];

fn observation_slot(subject: &str, kind: &str) -> Result<String> {
    // The observer production readers are subject-level, spanning every writer.
    Ok(serde_json::to_string(&[subject, kind])?)
}

static ACTUAL_MOVED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
    "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1
     AND CAST(accepted_at_unix_ms AS INTEGER)>=?2 AND {ACTUAL_STATE_CLAIM}
     AND {UNREPAIRED_CLAIM})",
));

fn actual_moved(connection: &Connection, subject: &str, cut: u128) -> Result<bool> {
    // Bounded subject/cut keys; the result is one EXISTS boolean, never claim JSON.
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hba6a871032cc owner=fractal_store::store::answers_check TODO=trace-statement-and-value-representation */ &ACTUAL_MOVED)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h5cede5425453 owner=fractal_store::store::answers_check TODO=trace-statement-and-value-representation */ params![subject, i64::try_from(cut)?], |row| row.get(0))?)
}

/// Only slots touched by post-cut claims move; another writer must not hide a stable slot.
pub fn slot_reader_movement(
    connection: &Connection, subjects: &BTreeSet<String>, cut: u128,
) -> Result<smallclaims::answers::ReaderMoves> {
    let mut moved = smallclaims::answers::ReaderMoves::new();
    let mut cursor = connection.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='observer.observed'
         AND CAST(accepted_at_unix_ms AS INTEGER)>=?2)",
    )?;
    for subject in subjects {
        if !subject.starts_with("observer/") { continue; }
        let kind = "observer.observed";
        // Bounded subject/cut keys and a single EXISTS boolean.
        let cursor_moved: bool = cursor.query_row(
            /* sqlite-large-value: allow-for-now lv-h4bf037bf5361 owner=fractal_store::store::answers_check TODO=trace-statement-and-value-representation */ params![subject, i64::try_from(cut)?], |row| row.get(0),
        )?;
        if actual_moved(connection, subject, cut)? || cursor_moved {
            moved.entry(format!("reader/slots/{kind}")).or_default()
                .entry(subject.clone()).or_default().insert(observation_slot(subject, kind)?);
        }
    }
    Ok(moved)
}

fn slot_reader_answers(
    connection: &Connection,
    cut: u128,
) -> Result<BTreeMap<String, Vec<smallclaims::answers::AnswerKey>>> {
    let mut readers = BTreeMap::new();
    for kind in SLOT_READERS {
        let subjects = connection.prepare_cached(
            "SELECT DISTINCT subject FROM claims WHERE kind=?1 ORDER BY subject",
        )?.query_map([kind], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut keys = Vec::new();
        for subject in subjects {
            let history = claims_of_kind_in_order(connection, &subject, kind)?;
            let cursor_moving = history.last().is_some_and(|claim| claim.accepted_at_unix_ms >= cut);
            let moving = cursor_moving || actual_moved(connection, &subject, cut)?;
            let state = latest_actual(connection, &subject)?;
            let cursor = resources::claimed_observer_cursor_at(connection, &subject)?;
            if !moving && state.is_none() && cursor.is_none() { continue; }
            let answer = json!({ "state": state, "cursor": cursor });
            let canonical = serde_json::to_string(&smallclaims::hash::canonical_json_value(&answer))?;
            keys.push(smallclaims::answers::AnswerKey {
                subject: subject.clone(),
                origin: observation_slot(&subject, kind)?,
                claim: hex::encode(Sha256::digest(canonical.as_bytes())),
                moving,
            });
        }
        keys.sort();
        // Empty entries advertise coverage, including when one member has no claims of the kind.
        readers.insert(format!("slots/{kind}"), keys);
    }
    Ok(readers)
}

/// Every fold reader's answers at `cut`, for [`smallclaims::answers::FoldReaders`]. For each
/// seat and step whose newest claim is older than the cut, it reads what the checkpoint proof
/// reads (`checkpoint_rules::subject_answers`: current harness, status history, actual state and
/// source, incarnations, reconcile decisions, usage, loop and timing) and reports each field as
/// its own reader, keyed by subject. A subject still written at the cut is left out, as a claim
/// kind's moving keys are, since its answers differ between members only by replication lag.
pub fn fold_reader_answers(
    connection: &Connection,
    cut: u128,
) -> Result<BTreeMap<String, Vec<smallclaims::answers::AnswerKey>>> {
    let mut next = connection.prepare_cached(
        "SELECT MIN(subject) FROM claims INDEXED BY claims_subject_index WHERE subject>?1 AND subject<?2",
    )?;
    let mut newest = connection.prepare_cached(
        "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_accepted_index WHERE subject=?1
         ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
    )?;
    let mut readers: BTreeMap<String, Vec<smallclaims::answers::AnswerKey>> = BTreeMap::new();
    let mut moving = Vec::new();
    for prefix in FOLDED_SUBJECTS {
        // Every subject that starts with `prefix` sorts after it and before the prefix with its
        // last character raised.
        let end = format!("{}{}", &prefix[..prefix.len() - 1], char::from(prefix.as_bytes()[prefix.len() - 1] + 1));
        let mut after = prefix.to_owned();
        loop {
            let subject = next
                .query(params![after, end])?
                .next()?
                .map(|row| row.get::<_, Option<String>>(0))
                .transpose()?
                .flatten();
            let Some(subject) = subject else { break };
            after = subject.clone();
            let accepted = newest
                .query([&subject])?
                .next()?
                .map(|row| row.get::<_, String>(0))
                .transpose()?
                .unwrap_or_default();
            if accepted.parse::<u128>().map_or(true, |accepted| accepted >= cut) {
                moving.push(subject);
                continue;
            }
            let Value::Object(answers) = checkpoint_rules::subject_answers(connection, &subject, cut)? else {
                continue;
            };
            for (reader, answer) in answers {
                let canonical = serde_json::to_string(&smallclaims::hash::canonical_json_value(&answer))?;
                readers.entry(reader).or_default().push(smallclaims::answers::AnswerKey {
                    subject: subject.clone(),
                    origin: String::new(),
                    claim: hex::encode(Sha256::digest(canonical.as_bytes())),
                    moving: false,
                });
            }
        }
    }
    // A subject that moved past the cut moves in every reader, so a member that computed before
    // its newer claim arrived is no difference.
    for keys in readers.values_mut() {
        keys.extend(moving.iter().map(|subject| smallclaims::answers::AnswerKey {
            subject: subject.clone(),
            origin: String::new(),
            claim: String::new(),
            moving: true,
        }));
        keys.sort();
    }
    readers.extend(slot_reader_answers(connection, cut)?);
    Ok(readers)
}

impl Store {
    /// The `reviewer` of a fault local trimming raises: the agent `[trim] notify` names, which
    /// then owns the fault, or the operator, which leaves it to the fleet's fault agent.
    pub fn trim_fault_reviewer(&self) -> String {
        self.trim_notify().unwrap_or_else(|| "person/operator".into())
    }

    /// The agent `[trim] notify` names, if any.
    pub fn trim_notify(&self) -> Option<String> {
        self.extension::<TrimSettings>()
            .notify
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Open each distinct disagreement once, record repeat occurrences locally, or recover all
    /// the pair's open faults when the answers agree.
    pub fn record_answers_check(&self, check: &AnswersCheck) -> Result<(), St3Error> {
        let local = self.origin().to_owned();
        let subject = format!("daemon/{local}");
        let pair = episode(&local, &check.peer);
        let open = self.open_answers_failures(&subject, &pair, &local, &check.peer).map_err(internal)?;
        if check.differences.is_empty() {
            for failure in open {
                self.append_claim(&ClaimInput {
                    subject: subject.clone(),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(failure.episode)),
                        ("failure".into(), json!(failure.id)),
                        (
                            "reason".into(),
                            json!(format!("{local} and {} answer the same again", check.peer)),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("{}:recovered", failure.id)),
                })?;
            }
            return Ok(());
        }
        let signature = answer_signature(&local, &check.peer, &check.differences).map_err(internal)?;
        if let Some(failure) = open.iter().find(|failure| failure.signature.as_deref() == Some(&signature)) {
            if failure.initial_cut.is_some_and(|cut| check.cut_unix_ms <= cut) { return Ok(()); }
            // Existing local-only kind and log: no replicated protocol or schema addition.
            self.append_claim(&ClaimInput {
                subject,
                kind: "daemon.diagnostic".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("answers-disagreement-occurrence")),
                    ("reason".into(), json!(serde_json::to_string(&json!({
                        "failure": failure.id, "signature": signature, "peer": check.peer,
                        "cut_unix_ms": check.cut_unix_ms.to_string(),
                    })).map_err(internal)?)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("{}:occurrence:{}", failure.id, check.cut_unix_ms)),
            })?;
            return Ok(());
        }
        let episode = format!("{pair}/{signature}");
        let peer = check.peer.as_str();
        let cut = chrono::DateTime::from_timestamp_millis(check.cut_unix_ms as i64)
            .map_or_else(|| check.cut_unix_ms.to_string(), |cut| cut.to_rfc3339());
        let kinds = check
            .differences
            .iter()
            .map(|report| report.kind.as_str())
            .collect::<Vec<_>>();
        let named = check
            .differences
            .iter()
            .map(|report| {
                let keys = report
                    .keys
                    .iter()
                    .take(3)
                    .map(|key| format!("{} ({})", key.subject, key.origin))
                    .collect::<Vec<_>>();
                let more = if report.complete && report.keys.len() <= 3 { "" } else { ", …" };
                if keys.is_empty() {
                    report.kind.clone()
                } else {
                    format!("{}: {}{more}", report.kind, keys.join(", "))
                }
            })
            .collect::<Vec<_>>();
        let reason = format!(
            "{local} and {peer} answer differently from the claims before {cut} in {}. Each \
             member compares folded reader values where both support them, otherwise claim \
             inventories. A differing inventory alone does not prove a differing reader answer. \
             Turn each key into a failing test, then fix the cause. The reproduction names every \
             kind and key. Nothing repairs it from a peer.",
            named.join("; ")
        );
        let reviewer = self.trim_fault_reviewer();
        self.append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("condition".into(), json!(ANSWERS_CONDITION)),
                ("reviewer".into(), json!(reviewer)),
                (
                    "title".into(),
                    json!(format!(
                        "{local} and {peer} answer differently in {}",
                        if kinds.len() > 3 {
                            format!("{} and {} more kinds", kinds[..3].join(", "), kinds.len() - 3)
                        } else {
                            kinds.join(", ")
                        }
                    )),
                ),
                ("reason".into(), json!(reason)),
                ("severity".into(), json!("error")),
                ("targets".into(), json!([subject, format!("daemon/{peer}")])),
                (
                    "reproduction".into(),
                    json!({
                        "members": [local, peer],
                        "cut_unix_ms": check.cut_unix_ms.to_string(),
                        "signature": signature,
                        "differences": check.differences,
                    }),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("{episode}:{local}:{}", check.cut_unix_ms)),
        })?;
        Ok(())
    }

    /// Open signatures, including pre-signature failures whose reproduction supplies the identity.
    fn open_answers_failures(
        &self, subject: &str, pair: &str, local: &str, peer: &str,
    ) -> Result<Vec<OpenAnswersFailure>> {
        let connection = self.readers.get();
        let mut query = connection.prepare(
            "SELECT id, json_extract(body,'$.fields.episode'), json_extract(body,'$.fields.reproduction.signature'),
                    json_extract(body,'$.fields.reproduction.differences'),
                    json_extract(body,'$.fields.reproduction.cut_unix_ms')
             FROM claims failure WHERE subject=?1 AND kind='operational.failure'
               AND (json_extract(body,'$.fields.episode')=?2
                    OR substr(json_extract(body,'$.fields.episode'),1,length(?2)+1)=?2||'/')
               AND NOT EXISTS(SELECT 1 FROM claims recovered
                              WHERE recovered.subject=failure.subject AND recovered.kind='operational.recovered'
                                AND json_extract(recovered.body,'$.fields.failure')=failure.id)
             ORDER BY id",
        )?;
        let rows = query.query_map(params![subject, pair], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        )))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(id, episode, signature, reproduction, cut)| {
            let signature = match (signature, reproduction) {
                (Some(signature), _) => Some(signature),
                (None, Some(reproduction)) => Some(answer_signature(
                    local, peer, &serde_json::from_str::<Vec<KindReport>>(&reproduction)?,
                )?),
                (None, None) => None,
            };
            Ok(OpenAnswersFailure { id, episode, signature, initial_cut: cut.map(|cut| cut.parse()).transpose()? })
        }).collect()
    }
}

#[cfg(test)]
#[path = "answers_check_tests.rs"]
mod reader_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::answers::{KeyDifference, KindReport};

    fn check(differences: Vec<KindReport>, cut: u128) -> AnswersCheck {
        AnswersCheck { peer: "birch".into(), cut_unix_ms: cut, differences }
    }

    fn differs() -> Vec<KindReport> {
        vec![KindReport {
            kind: "example.state".into(),
            ours: None,
            theirs: None,
            keys: vec![KeyDifference {
                subject: "item/b".into(),
                origin: "birch".into(),
                ours: Some("older".into()),
                theirs: Some("newer".into()),
            }],
            complete: true,
        }]
    }

    /// (kind, reviewer, title) of each fault claim on alder's daemon subject, in order.
    fn faults(store: &Store) -> Vec<(String, Option<String>, Option<String>)> {
        let connection = store.readers.get();
        let mut query = connection
            .prepare(
                "SELECT kind, json_extract(body,'$.fields.reviewer'), json_extract(body,'$.fields.title')
                 FROM claims WHERE subject='daemon/alder'
                   AND kind IN ('operational.failure','operational.recovered') ORDER BY store_index",
            )
            .unwrap();
        query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn a_difference_opens_one_fault_for_the_notify_agent_until_the_pair_agrees() {
        let store = Store::open_memory("alder").unwrap();
        *store.extension::<TrimSettings>().notify.lock().unwrap() =
            Some("agent/example/operations".into());
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        let opened = faults(&store);
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].0, "operational.failure");
        assert_eq!(opened[0].1.as_deref(), Some("agent/example/operations"));
        assert_eq!(
            opened[0].2.as_deref(),
            Some("alder and birch answer differently in example.state")
        );
        // The next day's difference stays the one open fault.
        store.record_answers_check(&check(differs(), 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 1);

        store.record_answers_check(&check(Vec::new(), 3_000)).unwrap();
        let ended = faults(&store);
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[1].0, "operational.recovered");
        // Agreeing again ends nothing more, and a new difference opens a new fault.
        store.record_answers_check(&check(Vec::new(), 4_000)).unwrap();
        assert_eq!(faults(&store).len(), 2);
        store.record_answers_check(&check(differs(), 5_000)).unwrap();
        assert_eq!(faults(&store).len(), 3);
    }

    fn occurrences(store: &Store) -> Vec<ClaimRecord> {
        store.local_observations_after(0, 100).unwrap().into_iter()
            .filter(|claim| claim.body["fields"]["code"] == "answers-disagreement-occurrence").collect()
    }

    #[test]
    fn identical_later_cut_records_an_occurrence_on_the_open_fault() {
        let store = Store::open_memory("alder").unwrap();
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        store.record_answers_check(&check(differs(), 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 1);
        let occurrences = occurrences(&store);
        assert_eq!(occurrences.len(), 1);
        let fields: Value = serde_json::from_str(occurrences[0].body["fields"]["reason"].as_str().unwrap()).unwrap();
        assert_eq!(fields["cut_unix_ms"], "2000");
        let open = store.open_answers_failures("daemon/alder", &episode("alder", "birch"), "alder", "birch").unwrap();
        assert_eq!(fields["failure"], open[0].id);
        assert_eq!(fields["signature"], open[0].signature.as_deref().unwrap());
    }

    #[test]
    fn a_new_kind_raises_a_new_fault_while_the_first_is_open() {
        let store = Store::open_memory("alder").unwrap();
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        let mut changed = differs();
        let mut additional = changed[0].clone();
        additional.kind = "another.state".into();
        changed.push(additional);
        store.record_answers_check(&check(changed, 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 2);
        assert!(occurrences(&store).is_empty());
    }

    #[test]
    fn a_new_key_raises_a_new_fault_while_the_first_is_open() {
        let store = Store::open_memory("alder").unwrap();
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        let mut changed = differs();
        let mut additional = changed[0].keys[0].clone();
        additional.subject = "item/c".into();
        changed[0].keys.push(additional);
        store.record_answers_check(&check(changed, 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 2);
    }

    #[test]
    fn a_changed_answer_raises_a_new_fault_while_the_first_is_open() {
        let store = Store::open_memory("alder").unwrap();
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        let mut changed = differs();
        changed[0].keys[0].theirs = Some("different".into());
        store.record_answers_check(&check(changed, 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 2);
    }

    #[test]
    fn each_cut_records_at_most_one_occurrence_and_the_initial_cut_records_none() {
        let store = Store::open_memory("alder").unwrap();
        for cut in [1_000, 1_000, 2_000, 2_000] {
            store.record_answers_check(&check(differs(), cut)).unwrap();
        }
        assert_eq!(faults(&store).len(), 1);
        assert_eq!(occurrences(&store).len(), 1);
    }

    #[test]
    fn agreement_recovers_every_signature_and_a_later_recurrence_opens_again() {
        let store = Store::open_memory("alder").unwrap();
        store.record_answers_check(&check(differs(), 1_000)).unwrap();
        let mut changed = differs();
        changed[0].keys[0].theirs = None;
        store.record_answers_check(&check(changed, 2_000)).unwrap();
        store.record_answers_check(&check(Vec::new(), 3_000)).unwrap();
        assert_eq!(faults(&store).iter().filter(|(kind, ..)| kind == "operational.recovered").count(), 2);
        store.record_answers_check(&check(differs(), 4_000)).unwrap();
        assert_eq!(faults(&store).iter().filter(|(kind, ..)| kind == "operational.failure").count(), 3);
    }

    #[test]
    fn signatures_ignore_report_order_cut_and_member_orientation() {
        let mut reports = differs();
        let mut other = reports[0].clone();
        other.kind = "another.state".into();
        reports.push(other);
        let expected = answer_signature("alder", "birch", &reports).unwrap();
        reports.reverse();
        for report in &mut reports {
            std::mem::swap(&mut report.ours, &mut report.theirs);
            for key in &mut report.keys { std::mem::swap(&mut key.ours, &mut key.theirs); }
        }
        assert_eq!(answer_signature("birch", "alder", &reports).unwrap(), expected);
    }

    #[test]
    fn a_pre_signature_failure_is_reused_from_its_reproduction() {
        let store = Store::open_memory("alder").unwrap();
        let failure = store.append_claim(&ClaimInput {
            subject: "daemon/alder".into(),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode("alder", "birch"))),
                ("condition".into(), json!(ANSWERS_CONDITION)),
                ("reproduction".into(), json!({"cut_unix_ms":"1000","differences":differs()})),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.record_answers_check(&check(differs(), 2_000)).unwrap();
        assert_eq!(faults(&store).len(), 1);
        let metadata: Value = serde_json::from_str(occurrences(&store)[0].body["fields"]["reason"].as_str().unwrap()).unwrap();
        assert_eq!(metadata["failure"], failure.id);
    }

    #[test]
    fn failure_and_local_occurrence_payloads_pass_real_store_admission() {
        let store = Store::open_memory("alder").unwrap();
        let failure = store.append_claim(&ClaimInput {
            subject: "daemon/alder".into(), kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!("example")),
                ("reproduction".into(), json!({"signature":"digest","differences":differs(),"cut_unix_ms":"1000"})),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let metadata = json!({"failure":failure.id,"signature":"digest","peer":"birch","cut_unix_ms":"2000"});
        let occurrence = store.append_claim(&ClaimInput {
            subject: "daemon/alder".into(), kind: "daemon.diagnostic".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("severity".into(), json!("warning")),
                ("code".into(), json!("answers-disagreement-occurrence")),
                ("reason".into(), json!(metadata.to_string())),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(local_observation_position(&occurrence).is_some());
        assert_eq!(serde_json::from_str::<Value>(occurrence.body["fields"]["reason"].as_str().unwrap()).unwrap(), metadata);
    }
}
