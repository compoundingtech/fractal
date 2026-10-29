//! Two members that answer differently from the same history have found a bug in trimming,
//! replication or a reader, never something to repair from a peer. The daily answers check
//! (`smallclaims::sync`) hands what it found here. A difference is recorded once per pair of
//! members, as an operational fault on this member's `daemon/NODE` subject that names the kinds
//! and keys, for the agent `[trim] notify` names. Each difference becomes a failing test and a
//! fix. A later check of the same pair that agrees ends the fault.
use super::*;
use smallclaims::answers::AnswersCheck;

pub const ANSWERS_CONDITION: &str = "answers-disagreement";
/// Every answers episode starts with this, so its failure and recovery read apart from the
/// daemon's other faults.
pub const EPISODE_PREFIX: &str = "answers-disagreement/";

/// Who owns an answers fault: `[trim] notify`, set at startup.
#[derive(Default)]
pub struct AnswersSettings {
    pub notify: Mutex<Option<String>>,
}

/// One episode per member pair, whichever member records it first.
pub fn episode(local: &str, peer: &str) -> String {
    let (first, second) = if local <= peer { (local, peer) } else { (peer, local) };
    let digest = hex::encode(Sha256::digest(format!("{first}\n{second}").as_bytes()));
    format!("{EPISODE_PREFIX}{}", &digest[..32])
}

impl Store {
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
             member counts, per kind, the newest claim of every subject and writer, which no \
             trim deletes, so one of them holds a claim the other lacks or has deleted one it \
             needs: turn each key into a failing test, then fix it. The reproduction names every \
             kind and key. Nothing repairs it from a peer.",
            named.join("; ")
        );
        let reviewer = self
            .extension::<AnswersSettings>()
            .notify
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| "person/operator".into());
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
        *store.extension::<AnswersSettings>().notify.lock().unwrap() =
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
