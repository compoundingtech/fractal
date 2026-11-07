//! Cheap desired declarations and comparison summaries at one reader snapshot.
use super::*;

pub const DESIRED_BATCH_LIMIT: usize = 1000;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct DesiredBatchRequest {
    /// Empty selects every declaration, including stopped and retired declarations.
    #[serde(default)]
    pub subjects: Vec<String>,
    pub kind: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DesiredBatchItem {
    #[serde(flatten)]
    pub declaration: DesiredSubject,
    pub desired_token: Option<String>,
    /// Sparse comparison fields from maintained projections, never a cold actual-state fold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

impl Store {
    pub fn desired_batch(&self, request: &DesiredBatchRequest) -> Result<Vec<DesiredBatchItem>> {
        if request.subjects.len() > DESIRED_BATCH_LIMIT {
            return Err(FractalError::new("desired-batch-limit", "at most 1000 subject names may be requested")
                .with_detail("limit", DESIRED_BATCH_LIMIT)
                .with_detail("supplied", request.subjects.len()).into());
        }
        // Pin one pooled reader for declarations, selected tokens and all summary sources.
        self.read_snapshot_unindexed(|| {
            let mut desired = if request.subjects.is_empty() {
                self.desired_subjects()?
            } else {
                for subject in &request.subjects {
                    smallclaims::touched::note_read(|| subject.clone());
                }
                let connection = self.readers.get();
                let mut statement = connection.prepare_cached(
                    "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
                     FROM desired WHERE subject IN (SELECT value FROM json_each(?1)) ORDER BY subject",
                )?;
                statement.query_map([serde_json::to_string(&request.subjects)?], desired_from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            // Missions have their own transactionally maintained declaration projection.
            // They are not rows in `desired`, but must be included in an all-kind read.
            let connection = self.readers.get();
            let mut query = connection.prepare_cached(
                "SELECT 'mission/' || d.mission_id, r.body FROM mission_definitions d
                 JOIN mission_revisions r ON r.mission_id=d.mission_id AND r.revision=d.revision
                 WHERE (?1='[]' OR 'mission/' || d.mission_id IN (SELECT value FROM json_each(?1)))
                 ORDER BY d.mission_id",
            )?;
            let missions = query.query_map([serde_json::to_string(&request.subjects)?], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
            for (subject, body) in missions {
                desired.push(DesiredSubject {
                    accountable: None, subject, kind: "mission".into(),
                    desired: serde_json::from_str(&body)?, member: None,
                    owner_run: None, owner_generation: None, owner_step: None,
                });
            }
            desired.retain(|item| request.kind.as_ref().is_none_or(|kind| &item.kind == kind));
            desired.sort_by(|left, right| left.subject.cmp(&right.subject));
            desired.dedup_by(|left, right| left.subject == right.subject);
            let names = desired.iter()
                .map(|item| item.subject.as_str()).collect::<Vec<_>>();
            let mut tokens = self.selected_desired_tokens(&names)?;
            desired.into_iter().map(|declaration| {
                let actual = self.desired_actual_summary(&declaration)?;
                Ok(DesiredBatchItem {
                    desired_token: tokens.remove(&declaration.subject), declaration, actual,
                })
            }).collect()
        })
    }
}
