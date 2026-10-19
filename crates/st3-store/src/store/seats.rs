use super::*;

impl Store {
    #[cfg(any(test, feature = "test-support"))]
    pub fn agent_resources_builds_for_test(&self) -> usize {
        self.smalltalk.agent_resources_builds.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The most cards one projection call folded since the store opened.
    #[cfg(any(test, feature = "test-support"))]
    pub fn agent_resources_largest_fold_for_test(&self) -> usize {
        self.smalltalk.agent_resources_largest_fold.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The seat declaration that the stop selected for `subject` ended, or `None` when the
    /// selected declaration is not a stop. A stop that replaced another stop is followed back
    /// to the declaration they ended; a merge of several is ambiguous and also gives `None`.
    pub fn declaration_ended_by_stop(&self, subject: &str) -> Result<Option<EndedDeclaration>> {
        let connection = self.readers.get();
        let Some((stop, writer)) = connection
            .query_row(
                "SELECT desired.subject, desired.kind, desired.body, desired.member,
                        desired.owner_run, desired.owner_generation, desired.owner_step,
                        desired.claim_id, claims.actor
                 FROM desired LEFT JOIN claims ON claims.id = desired.claim_id
                 WHERE desired.subject = ?1 AND desired.kind = 'stop'",
                [subject],
                |row| {
                    Ok((
                        (desired_from_row(row)?, row.get::<_, String>(7)?),
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let (stop, token) = stop;
        let mut claim = claim_by_id_tx(&connection, &token)?;
        // A seat is stopped and started a few times at most; the bound only guards a cycle.
        for _ in 0..64 {
            let Some(current) = claim else {
                return Ok(None);
            };
            let desired: DesiredSubject = serde_json::from_value(current.body)?;
            if desired.kind == "agent" {
                return Ok(Some(EndedDeclaration {
                    stop,
                    token,
                    writer,
                    declaration: desired,
                }));
            }
            let [predecessor] = current.predecessors.as_slice() else {
                return Ok(None);
            };
            if desired.kind != "stop" {
                return Ok(None);
            }
            claim = claim_by_id_tx(&connection, predecessor)?;
        }
        Ok(None)
    }

    /// Suspension ancestry selected at a client snapshot; stopped declarations have none.
    pub fn suspension_lineage_at(&self, subject: &str, at: Option<u64>) -> Result<Vec<String>> {
        let connection = self.readers.get();
        let Some(row) = desired_row_at(&connection, subject, at)?.filter(|row| row.kind == "agent") else {
            return Ok(Vec::new());
        };
        // Replication can deliver a predecessor after its successor; the snapshot walks only the
        // ancestry it held.
        desired_lineage_from_at_tx(&connection, row.claim_id, same_declared_launch, at)
    }

    /// Suspension ancestry from `token`, over the claims accepted at or before `at`.
    pub fn suspension_lineage_from_at(&self, token: &str, at: Option<u64>) -> Result<Vec<String>> {
        desired_lineage_from_at_tx(&self.readers.get(), token.to_owned(), same_declared_launch, at)
    }

    /// The kind of the declaration selected at a client snapshot.
    pub fn selected_desired_kind_at(&self, subject: &str, at: Option<u64>) -> Result<Option<String>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        desired_row_at(&connection, subject, at).map(|row| row.map(|row| row.kind))
    }

    /// `latest_actual_value` at a client snapshot.
    pub fn latest_actual_value_at(&self, subject: &str, at: Option<u64>) -> Result<Option<Value>> {
        if at.is_none() {
            return self.latest_actual_value(subject);
        }
        smallclaims::touched::note_read(|| subject.to_owned());
        latest_actual_at(&self.readers.get(), subject, at)
    }

    pub fn current_harness(
        &self,
        subject: &str,
    ) -> Result<Option<crate::model::CurrentHarnessView>> {
        // Its work claims and progress live on step subjects, written with this agent as actor.
        smallclaims::touched::note_read(|| subject.to_owned());
        smallclaims::touched::note_read(|| format!("actor:{subject}"));
        let connection = self.readers.get();
        current_harness_at(&connection, subject, None)
    }

}

/// A selected stop and the seat declaration it ended.
#[derive(Clone, Debug)]
pub struct EndedDeclaration {
    /// The selected stop.
    pub stop: DesiredSubject,
    /// The stop's claim, the seat's selected desired token.
    pub token: String,
    /// Who published the stop; the daemon's own stops record no writer.
    pub writer: Option<String>,
    /// The agent declaration the stop ended.
    pub declaration: DesiredSubject,
}

