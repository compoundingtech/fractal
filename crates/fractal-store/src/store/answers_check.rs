//! Two members that answer differently from the same history have found a bug in trimming,
//! replication or a reader, never something to repair from a peer. The daily answers check
//! (`smallclaims::sync`) hands what it found here. A difference is recorded once per pair of
//! members, as an operational fault on this member's `daemon/NODE` subject that names the kinds
//! and keys, for the agent `[trim] notify` names. Each difference becomes a failing test and a
//! fix. A later check of the same pair that agrees ends the fault.
use super::*;
use smallclaims::answers::AnswersCheck;
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

/// One episode per member pair, whichever member records it first.
pub fn episode(local: &str, peer: &str) -> String {
    let (first, second) = if local <= peer { (local, peer) } else { (peer, local) };
    let digest = hex::encode(Sha256::digest(format!("{first}\n{second}").as_bytes()));
    format!("{EPISODE_PREFIX}{}", &digest[..32])
}

/// The subjects whose answers fold claims rather than read one: seats (harness state, status
/// history, working since) and steps (timing).
const FOLDED_SUBJECTS: [&str; 2] = ["agent/", "step-run/"];

/// Claim kinds whose retention slots are the unit of observation, not the originating writer.
const SLOT_READERS: [&str; 7] = [
    "daemon.diagnostic", "harness.observed", "harness.todo.observed", "harness.usage",
    "observer.observed", "resource.observed", "transport.observed",
];

fn observation_slot(claim: &ClaimRecord) -> Vec<String> {
    // Resource receipts and poller observations feed one subject-level projection and recorder.
    if claim.kind == "resource.observed" { return vec![claim.subject.clone(), claim.kind.clone()]; }
    checkpoint_rules::slot_of(claim).map_or_else(
        || vec![claim.subject.clone(), claim.kind.clone(), claim.origin.clone()],
        |(_, slot)| slot,
    )
}

/// Only slots touched by post-cut claims move; another writer must not hide a stable slot.
pub fn slot_reader_movement(
    connection: &Connection, subjects: &BTreeSet<String>, cut: u128,
) -> Result<smallclaims::answers::ReaderMoves> {
    let mut moved = smallclaims::answers::ReaderMoves::new();
    let mut query = connection.prepare_cached(&format!(
        "SELECT {} FROM claims WHERE subject=?1 AND CAST(accepted_at_unix_ms AS INTEGER)>=?2
         AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)",
        smallclaims::store::CLAIM_COLUMNS,
    ))?;
    for subject in subjects {
        for claim in query.query_map(params![subject, i64::try_from(cut)?], smallclaims::store::claim_from_row)? {
            let claim = claim?;
            let (kind, slot) = if claim.kind == "vcs.item.state" {
                ("resource.observed", vec![subject.clone(), "resource.observed".into()])
            } else if SLOT_READERS.contains(&claim.kind.as_str()) {
                (claim.kind.as_str(), observation_slot(&claim))
            } else { continue };
            moved.entry(format!("reader/slots/{kind}")).or_default()
                .entry(subject.clone()).or_default().insert(serde_json::to_string(&slot)?);
        }
    }
    Ok(moved)
}

fn slot_reader_answers(
    connection: &Connection,
    cut: u128,
) -> Result<BTreeMap<String, Vec<smallclaims::answers::AnswerKey>>> {
    let mut readers = BTreeMap::new();
    let repaired = connection.prepare_cached("SELECT id FROM projection_digest_repaired_claims")?
        .query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<BTreeSet<_>>>()?;
    for kind in SLOT_READERS {
        let subjects = connection.prepare_cached(
            "SELECT DISTINCT subject FROM claims WHERE kind=?1 ORDER BY subject",
        )?.query_map([kind], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut keys = Vec::new();
        for subject in subjects {
            let claims = claims_of_kind_in_order(connection, &subject, kind)?;
            let mut slots: BTreeMap<Vec<String>, Vec<&ClaimRecord>> = BTreeMap::new();
            for claim in claims.iter().filter(|claim| !repaired.contains(&claim.id)) {
                let slot = observation_slot(claim);
                slots.entry(slot).or_default().push(claim);
            }
            for (slot, claims) in slots {
                let moving = claims.iter().any(|claim| claim.accepted_at_unix_ms >= cut)
                    || (kind == "resource.observed" && resources::newest_item_state(connection, &subject)?
                        .is_some_and(|(_, _, at)| at.parse::<u128>().map_or(true, |at| at >= cut)));
                let answer = if kind == "harness.usage" {
                    json!(Store::usage_summary_from_rows(claims.iter().map(|claim| Ok((
                        claim.store_index, serde_json::to_string(&claim.body).unwrap_or_default(),
                        claim.accepted_at_unix_ms.to_string(),
                    ))), None)?)
                } else if kind == "resource.observed" {
                    // Read the production projection: its first opener, attribution-only rows
                    // and compact item-state overlays depend on observation acceptance times.
                    let row: Option<String> = connection.query_row(
                        "SELECT json_object('kind',r.kind,'facts',json(r.facts),'observed_at',r.observed_at,
                         'opened_by',r.opened_by,'opened_by_run',r.opened_by_run,
                         'observer',json_extract(c.body,'$.fields.observer'))
                         FROM resource_observations r JOIN claims c ON c.id=r.claim_id WHERE r.subject=?1",
                        [&subject], |row| row.get(0),
                    ).optional()?;
                    row.map(|row| serde_json::from_str(&row)).transpose()?.unwrap_or(Value::Null)
                } else {
                    fold_latest_values(claims.iter().map(|claim| (kind.to_owned(), claim.body.clone())))?
                        .unwrap_or(Value::Null)
                };
                let canonical = serde_json::to_string(&smallclaims::hash::canonical_json_value(&answer))?;
                keys.push(smallclaims::answers::AnswerKey {
                    subject: subject.clone(),
                    // The wire's secondary key carries the explicit retention slot, not an
                    // invented writer dimension. Include the rule's full slot to avoid collisions.
                    origin: serde_json::to_string(&slot)?,
                    claim: hex::encode(Sha256::digest(canonical.as_bytes())),
                    moving,
                });
            }
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

    /// Open a fault for a check that found differences, unless the pair's fault is still open,
    /// or end the pair's open fault for a check that agrees.
    pub fn record_answers_check(&self, check: &AnswersCheck) -> Result<(), St3Error> {
        let local = self.origin().to_owned();
        let subject = format!("daemon/{local}");
        let episode = episode(&local, &check.peer);
        let open = self.open_answers_failures(&subject, &episode).map_err(internal)?;
        if check.differences.is_empty() {
            for failure in open {
                self.append_claim(&ClaimInput {
                    subject: subject.clone(),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(episode)),
                        ("failure".into(), json!(failure)),
                        (
                            "reason".into(),
                            json!(format!("{local} and {} answer the same again", check.peer)),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("{failure}:recovered")),
                })?;
            }
            return Ok(());
        }
        if !open.is_empty() {
            return Ok(());
        }
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

    /// The failures of `episode` on `subject` that no recovery names.
    fn open_answers_failures(&self, subject: &str, episode: &str) -> Result<Vec<String>> {
        let connection = self.readers.get();
        let mut query = connection.prepare(
            "SELECT id, kind, json_extract(body,'$.fields.failure') FROM claims
             WHERE subject=?1 AND kind IN ('operational.failure','operational.recovered')
               AND json_extract(body,'$.fields.episode')=?2",
        )?;
        let rows = query
            .query_map(params![subject, episode], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let recovered = rows
            .iter()
            .filter(|(_, kind, _)| kind == "operational.recovered")
            .filter_map(|(.., failure)| failure.as_deref())
            .collect::<BTreeSet<_>>();
        let mut open = rows
            .iter()
            .filter(|(id, kind, _)| kind == "operational.failure" && !recovered.contains(id.as_str()))
            .map(|(id, ..)| id.clone())
            .collect::<Vec<_>>();
        open.sort();
        Ok(open)
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
}
