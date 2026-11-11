//! Reads over recorded source maps beyond one subject revision: step runs resolved through
//! their run's pinned revision, a subject's mapped revisions, the reverse index over live
//! subjects, and the "declared at" line agents get in failure text.
use super::*;
use super::source_maps::read;
use fractal_schema::source_map::index::{RevisionEntry, Revisions, Where, where_used};
use fractal_schema::source_map::{Claim, KIND, locate};

/// The mission revision a step run's generation pinned, and the step's path in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepTarget {
    /// Without the `mission/` prefix.
    pub mission: String,
    pub revision: String,
    pub step_path: String,
}

pub fn step_target(connection: &Connection, step: &str) -> Result<Option<StepTarget>, FractalError> {
    connection
        .query_row(
            "SELECT r.mission_id, g.revision, s.step_path FROM step_runs s
             JOIN run_generations g ON g.id=s.generation_id
             JOIN mission_runs r ON r.id=s.run_id
             WHERE s.subject=?1",
            [step],
            |row| Ok(StepTarget { mission: row.get(0)?, revision: row.get(1)?, step_path: row.get(2)? }),
        )
        .optional()
        .map_err(internal)
}

/// A mission run's mission and the revision of its current generation.
fn run_target(connection: &Connection, run: &str) -> Result<Option<(String, String)>, FractalError> {
    connection
        .query_row(
            "SELECT r.mission_id, g.revision FROM mission_runs r
             JOIN run_generations g ON g.id=r.current_generation_id
             WHERE r.id=?1",
            [run.strip_prefix("mission-run/").unwrap_or(run)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(internal)
}

fn decode(fields: Value) -> Result<Claim, FractalError> {
    serde_json::from_value(fields).map_err(internal)
}

/// One subject's map read: for a step run, its mission at the run's pinned revision, focused on
/// the step's node.
#[derive(Clone, Debug)]
pub struct SourceMapRead {
    pub subject: String,
    pub revision: String,
    pub step: Option<String>,
    pub node: Option<String>,
    pub map: Option<Value>,
}

impl Store {
    pub fn source_map_read(&self, subject: &str, revision: Option<&str>) -> Result<SourceMapRead, FractalError> {
        if !subject.starts_with("step-run/") {
            let (revision, map) = self.source_map(subject, revision)?;
            return Ok(SourceMapRead { subject: subject.to_owned(), revision, step: None, node: None, map });
        }
        if revision.is_some() {
            return Err(FractalError::new(
                "invalid-source-map-query",
                "a step run's revision is the one its run pinned; omit `revision`",
            ));
        }
        let target = step_target(&self.readers.get(), subject)?
            .ok_or_else(|| FractalError::new("unknown-subject", format!("{subject} is not a step run")))?;
        let mission = format!("mission/{}", target.mission);
        let (revision, map) = self.source_map(&mission, Some(&target.revision))?;
        Ok(SourceMapRead {
            node: Some(locate::step_path(&target.mission, &target.step_path)),
            subject: mission,
            revision,
            step: Some(subject.to_owned()),
            map,
        })
    }

    /// Every revision of `subject` that has a map, newest first.
    pub fn source_map_revisions(&self, subject: &str) -> Result<Revisions, FractalError> {
        let connection = self.readers.get();
        let current = source_maps::current_revision(&connection, subject)?;
        // Claim subjects are `<subject>@<revision>`; `A` follows `@`, so this range is exactly them.
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-he9d5580678ac owner=fractal_store::store::source_index TODO=trace-statement-and-value-representation */ &smallclaims::store::canonical_sql(
                "SELECT subject, body, accepted_at_unix_ms FROM claims
                 WHERE subject>=?1 AND subject<?2 AND kind=?3 ORDER BY CANONICAL_ASC(claims)",
            ))
            .map_err(internal)?;
        let rows = statement
            .query_map(params![format!("{subject}@"), format!("{subject}A"), KIND], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(internal)?;
        let mut seen = BTreeSet::new();
        let mut revisions = Vec::new();
        for row in rows {
            let (claim_subject, body, accepted) = row.map_err(internal)?;
            // First write wins: a later claim for the same revision is not its map.
            if !seen.insert(claim_subject) {
                continue;
            }
            let mut body: Value = serde_json::from_str(&body).map_err(internal)?;
            let claim = decode(body["fields"].take())?;
            revisions.push(RevisionEntry {
                revision: claim.revision,
                repository: claim.repository,
                r#ref: claim.r#ref,
                sha: claim.sha,
                sequence: claim.sequence,
                set_revision: claim.set_revision,
                recorded_at_unix_ms: accepted.parse().map_err(internal)?,
            });
        }
        revisions.sort_by_key(|entry| std::cmp::Reverse((entry.recorded_at_unix_ms, entry.sequence)));
        Ok(Revisions { subject: subject.to_owned(), current, revisions })
    }

    /// The maps of every live agent and mission revision that name `path`.
    pub fn source_where(&self, path: &str, repository: Option<&str>, line: Option<u32>) -> Result<Where, FractalError> {
        let connection = self.readers.get();
        let mut live: Vec<(String, String)> = Vec::new();
        let mut agents = connection
            .prepare("SELECT subject, revision FROM desired WHERE kind='agent'")
            .map_err(internal)?;
        let mut missions = connection
            .prepare("SELECT 'mission/'||mission_id, revision FROM mission_definitions WHERE state<>'retired'")
            .map_err(internal)?;
        for statement in [&mut agents, &mut missions] {
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(internal)?;
            for row in rows {
                live.push(row.map_err(internal)?);
            }
        }
        drop((agents, missions));
        let mut claims = Vec::new();
        for (subject, revision) in live {
            if let Some(fields) = read(&connection, &subject, &revision)? {
                claims.push(decode(fields)?);
            }
        }
        drop(connection);
        let mut failure = None;
        let found = where_used(path, repository, line, &claims, |hash| match self.source_blob(hash) {
            Ok(bytes) => bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            Err(error) => {
                failure.get_or_insert(error);
                None
            }
        });
        match failure {
            Some(error) => Err(error),
            None => Ok(found),
        }
    }

    /// `path:line (owner/repo@sha)` of a step run's step, or of gate `gate` on a step run or on a
    /// mission run, when the pinned revision has a map.
    pub fn source_declared_at(&self, subject: &str, gate: Option<&str>) -> Result<Option<String>, FractalError> {
        let connection = self.readers.get();
        let (mission, revision, kdl) = if subject.starts_with("step-run/") {
            let Some(target) = step_target(&connection, subject)? else {
                return Ok(None);
            };
            let kdl = match gate {
                Some(gate) => locate::gate_path(&target.mission, Some(&target.step_path), gate),
                None => locate::step_path(&target.mission, &target.step_path),
            };
            (target.mission, target.revision, kdl)
        } else if subject.starts_with("mission-run/") {
            let Some((mission, revision)) = run_target(&connection, subject)? else {
                return Ok(None);
            };
            let kdl = match gate {
                Some(gate) => locate::gate_path(&mission, None, gate),
                None => format!("mission[{mission}]"),
            };
            (mission, revision, kdl)
        } else {
            return Ok(None);
        };
        let Some(fields) = read(&connection, &format!("mission/{mission}"), &revision)? else {
            return Ok(None);
        };
        Ok(decode(fields)?.declared_at(&kdl))
    }

    /// The agent declaration body whose desired revision is `revision`, from its intent history.
    pub fn agent_declaration_at(&self, subject: &str, revision: &str) -> Result<Option<Value>, FractalError> {
        let connection = self.readers.get();
        let mut statement = connection
            .prepare("SELECT body FROM claims WHERE subject=?1 AND kind='intent.desired' ORDER BY store_index DESC")
            .map_err(internal)?;
        let rows = statement.query_map([subject], |row| row.get::<_, String>(0)).map_err(internal)?;
        for body in rows {
            let desired: DesiredSubject = serde_json::from_str(&body.map_err(internal)?).map_err(internal)?;
            if desired.kind == "agent" && desired_revision(&desired) == revision {
                return Ok(Some(desired.desired));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use fractal_schema::source_map::claim_subject;

    #[test]
    fn claim_subject_range_holds_only_this_subjects_revisions() {
        for (other, inside) in [("mission/a@r1", true), ("mission/a/b@r1", false), ("mission/aa@r1", false), ("mission/a", false)] {
            let within = ("mission/a@".."mission/aA").contains(&other);
            assert_eq!(within, inside, "{other}");
        }
        assert_eq!(claim_subject("mission/a", "r1"), "mission/a@r1");
    }
}
