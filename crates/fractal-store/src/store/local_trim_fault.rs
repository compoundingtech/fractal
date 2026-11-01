//! A local trim that stops, because its plan would drop the newest claim of a key or a delete
//! would change the graph, has found a bug in a retention rule or the planner. It halts until
//! resumed, and the stop is an operational fault on this member's `daemon/NODE` subject for the
//! agent `[trim] notify` names, like an answers disagreement. Each stop becomes a failing test and
//! a fix. Resuming ends the fault.
use super::*;
use smallclaims::store::checkpoint_agreement::CheckpointAction;

pub const LOCAL_TRIM_STOP_CONDITION: &str = "local-trim-stopped";
/// Every stop episode starts with this, one per checkpoint.
pub const LOCAL_TRIM_STOP_PREFIX: &str = "local-trim-stopped/";

impl Store {
    /// Open a fault for each local trim the step stopped. Shared checkpoints keep their own
    /// diagnostics and wait for a person.
    pub fn record_local_trim_stops(&self, actions: &[CheckpointAction]) -> Result<(), St3Error> {
        if !self.trims_locally() {
            return Ok(());
        }
        let local = self.origin().to_owned();
        let subject = format!("daemon/{local}");
        for action in actions {
            let CheckpointAction::TrimGraphChanged { checkpoint } = action else {
                continue;
            };
            let episode = format!("{LOCAL_TRIM_STOP_PREFIX}{checkpoint}");
            let reviewer = self.trim_fault_reviewer();
            self.append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: "operational.failure".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("episode".into(), json!(episode)),
                    ("condition".into(), json!(LOCAL_TRIM_STOP_CONDITION)),
                    ("reviewer".into(), json!(reviewer)),
                    ("title".into(), json!(format!("{local} stopped its local trim of {checkpoint}"))),
                    (
                        "reason".into(),
                        json!(format!(
                            "the local trim of {checkpoint} would have dropped the newest claim of \
                             a key or changed the graph, so it stopped and no local trim runs until \
                             it is resumed. The diagnostic on {subject} names the claim or chunk: \
                             turn it into a failing test, then fix the rule. Resume with `st \
                             replication checkpoint resume --reason TEXT`; the cut is set aside."
                        )),
                    ),
                    ("severity".into(), json!("error")),
                    ("targets".into(), json!([subject])),
                    ("reproduction".into(), json!({"member": local, "checkpoint": checkpoint})),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("{episode}:{local}")),
            })?;
        }
        Ok(())
    }

    /// Resume stopped checkpoints and end every open local trim stop. A person may, and on a
    /// member that trims locally so may the agent `[trim] notify` names, and no other agent.
    pub fn resume_stopped_trims(&self, actor: &str, reason: &str) -> Result<(), St3Error> {
        self.resume_checkpoints_allowing(actor, reason, self.trim_notify().as_deref())?;
        self.recover_local_trim_stops(actor, reason)
    }

    /// End every open local trim stop, after a resume.
    pub fn recover_local_trim_stops(&self, actor: &str, reason: &str) -> Result<(), St3Error> {
        let subject = format!("daemon/{}", self.origin());
        let open: Vec<(String, String)> = {
            let connection = self.readers.get();
            let mut query = connection
                .prepare(
                    "SELECT failure.id, json_extract(failure.body,'$.fields.episode')
                     FROM claims AS failure
                     WHERE failure.subject=?1 AND failure.kind='operational.failure'
                       AND json_extract(failure.body,'$.fields.condition')=?2
                       AND NOT EXISTS(SELECT 1 FROM claims AS recovered
                                      WHERE recovered.subject=failure.subject
                                        AND recovered.kind='operational.recovered'
                                        AND json_extract(recovered.body,'$.fields.failure')=failure.id)",
                )
                .map_err(internal)?;
            query
                .query_map(params![subject, LOCAL_TRIM_STOP_CONDITION], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(internal)?
                .collect::<rusqlite::Result<_>>()
                .map_err(internal)?
        };
        for (failure, episode) in open {
            self.append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: "operational.recovered".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("episode".into(), json!(episode)),
                    ("failure".into(), json!(failure)),
                    ("reason".into(), json!(format!("resumed by {actor}: {reason}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("{failure}:recovered")),
            })?;
        }
        Ok(())
    }
}
