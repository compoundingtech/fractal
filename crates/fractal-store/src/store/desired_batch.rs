//! Bounded desired declarations and comparison summaries at one reader cut.
use super::*;
use base64::Engine as _;
use super::desired_progress::{desired_projection_progress, DesiredProjectionProgress};

pub const DESIRED_BATCH_LIMIT: usize = 1000;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct DesiredBatchRequest {
    /// Empty selects every declaration, including stopped and retired declarations.
    #[serde(default)]
    pub subjects: Vec<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DesiredBatchItem {
    #[serde(flatten)]
    pub declaration: DesiredSubject,
    pub desired_token: Option<String>,
    /// Sparse maintained comparison fields, never a cold actual-state fold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DesiredBatchPage {
    pub items: Vec<DesiredBatchItem>,
    pub next_cursor: Option<String>,
    pub store_index: u64,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct DesiredCursor {
    origin: String,
    cut: u64,
    progress: DesiredProjectionProgress,
    subjects: Vec<String>,
    kind: Option<String>,
    limit: usize,
    after: String,
}

fn decode_cursor(encoded: &str) -> Result<DesiredCursor> {
    let invalid = || FractalError::new("invalid-desired-cursor", "invalid desired page cursor");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.strip_prefix("desired/v1/").ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid().into())
}

impl Store {
    pub fn desired_batch(&self, request: &DesiredBatchRequest) -> Result<DesiredBatchPage> {
        if request.subjects.len() > DESIRED_BATCH_LIMIT {
            return Err(FractalError::new("desired-batch-limit", "at most 1000 subject names may be requested")
                .with_detail("limit", DESIRED_BATCH_LIMIT)
                .with_detail("supplied", request.subjects.len()).into());
        }
        let limit = request.limit.unwrap_or(DESIRED_BATCH_LIMIT);
        if limit == 0 || limit > DESIRED_BATCH_LIMIT {
            return Err(FractalError::new("desired-page-limit", "limit must be between 1 and 1000")
                .with_detail("limit", DESIRED_BATCH_LIMIT).into());
        }
        let mut subjects = request.subjects.clone();
        subjects.sort();
        subjects.dedup();
        let cursor = request.cursor.as_deref().map(decode_cursor).transpose()?;
        // Projection and repair writes can advance without a claim append. Fence the
        // claim cut, graph frontier and desired row-write witness inside this snapshot.
        self.read_snapshot(|cut| {
            let connection = self.readers.get();
            let progress = desired_projection_progress(&connection)?;
            if let Some(cursor) = &cursor {
                if cursor.origin != self.origin() || cursor.subjects != subjects
                    || cursor.kind != request.kind || cursor.limit != limit {
                    return Err(FractalError::new("invalid-desired-cursor", "cursor does not match this desired selection").into());
                }
                if cursor.cut != cut || cursor.progress != progress {
                    return Err(FractalError::new("desired-cursor-expired", "the store changed since the desired page was read")
                        .with_detail("store_index", cut).into());
                }
            }
            let after = cursor.as_ref().map_or("", |cursor| cursor.after.as_str());
            smallclaims::touched::note_read(|| "kind:intent.desired".to_owned());
            // Reuse the reader pinned by read_snapshot for every projection read.
            let mut query = match (subjects.is_empty(), request.kind.is_some()) {
                (true, false) => connection.prepare_cached(
                    "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
                     FROM desired WHERE ?1='[]' AND ?2 IS NULL
                     AND subject>?3 ORDER BY subject LIMIT ?4",
                )?,
                (true, true) => connection.prepare_cached(
                    "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
                     FROM desired INDEXED BY desired_kind_subject_index
                     WHERE ?1='[]' AND kind=?2 AND subject>?3 ORDER BY subject LIMIT ?4",
                )?,
                (false, false) => connection.prepare_cached(
                    "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
                     FROM desired WHERE subject IN (SELECT value FROM json_each(?1))
                     AND ?2 IS NULL AND subject>?3 ORDER BY subject LIMIT ?4",
                )?,
                (false, true) => connection.prepare_cached(
                    "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
                     FROM desired INDEXED BY desired_kind_subject_index
                     WHERE subject IN (SELECT value FROM json_each(?1))
                     AND kind=?2 AND subject>?3 ORDER BY subject LIMIT ?4",
                )?,
            };
            let mut desired = query.query_map(
                params![serde_json::to_string(&subjects)?, request.kind, after, limit + 1], desired_from_row,
            )?.collect::<rusqlite::Result<Vec<_>>>()?;
            // Missions have a separate selected-definition projection. Read at most one
            // page from each indexed family, then merge; never materialize the full graph.
            if request.kind.as_deref().is_none_or(|kind| kind == "mission")
                && (after < "mission/" || after.starts_with("mission/")) {
                smallclaims::touched::note_read(|| "kind:mission.published".to_owned());
                let mission_names = subjects.iter().filter_map(|subject| subject.strip_prefix("mission/"))
                    .collect::<Vec<_>>();
                if subjects.is_empty() || !mission_names.is_empty() {
                    let mut query = if subjects.is_empty() {
                        connection.prepare_cached(
                            "SELECT 'mission/' || d.mission_id, 'mission', r.body, NULL, NULL, NULL, NULL
                             FROM mission_definitions d JOIN mission_revisions r
                             ON r.mission_id=d.mission_id AND r.revision=d.revision
                             WHERE ?1 AND ?2='[]' AND d.mission_id>?3 ORDER BY d.mission_id LIMIT ?4",
                        )?
                    } else {
                        connection.prepare_cached(
                            "SELECT 'mission/' || d.mission_id, 'mission', r.body, NULL, NULL, NULL, NULL
                             FROM mission_definitions d JOIN mission_revisions r
                             ON r.mission_id=d.mission_id AND r.revision=d.revision
                             WHERE NOT ?1 AND d.mission_id IN (SELECT value FROM json_each(?2))
                             AND d.mission_id>?3 ORDER BY d.mission_id LIMIT ?4",
                        )?
                    };
                    desired.extend(query.query_map(params![subjects.is_empty(), serde_json::to_string(&mission_names)?,
                        after.strip_prefix("mission/").unwrap_or(""), limit + 1], desired_from_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?);
                }
            }
            desired.sort_by(|left, right| left.subject.cmp(&right.subject));
            desired.dedup_by(|left, right| left.subject == right.subject);
            let has_more = desired.len() > limit;
            desired.truncate(limit);
            let next_cursor = if has_more {
                let cursor = DesiredCursor {
                    origin: self.origin().into(), cut, progress, subjects: subjects.clone(), kind: request.kind.clone(),
                    limit, after: desired.last().expect("nonempty bounded page").subject.clone(),
                };
                Some(format!("desired/v1/{}", base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(serde_json::to_vec(&cursor)?)))
            } else { None };
            let names = desired.iter().map(|item| item.subject.as_str()).collect::<Vec<_>>();
            let mut tokens = self.selected_desired_tokens(&names)?;
            let items = desired.into_iter().map(|declaration| {
                let actual = self.desired_actual_summary(&declaration)?;
                Ok(DesiredBatchItem {
                    desired_token: tokens.remove(&declaration.subject), declaration, actual,
                })
            }).collect::<Result<Vec<_>>>()?;
            Ok(DesiredBatchPage { items, next_cursor, store_index: cut })
        })
    }
}
