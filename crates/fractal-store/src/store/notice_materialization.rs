//! Current notice readiness only; source/control payloads never cross this boundary.
use super::*;
use fractal_schema::notice;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum NoticeMaterializationState {
    Ready { snapshot_claim: String, binding_claim: String },
    Rebuilding { reason: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum NoticeCollectionState {
    Ready,
    Rebuilding { reason: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoticeMaterialization {
    pub id: String,
    pub source_id: String,
    pub state: NoticeMaterializationState,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoticeMaterializationCollection {
    pub state: NoticeCollectionState,
    pub notices: Vec<NoticeMaterialization>,
}

impl Store {
    pub fn notice_materializations(&self, person: &str) -> Result<NoticeMaterializationCollection, FractalError> {
        fractal_schema::registry().validate_subject(person).map_err(custom::typed)?;
        if !person.starts_with("person/") {
            return Err(FractalError::new("invalid-notice-recipient", "notice recipient must be a person"));
        }
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction().map_err(internal)?;
        let mut unavailable_reason = None;
        let binding = match notice_authority::binding(&tx) {
            Ok((_, binding)) => Some(binding),
            Err(error) if error.code == "notices-unavailable" => {
                unavailable_reason = Some(error.message);
                None
            }
            Err(error) => return Err(error),
        };
        for manifest in notice::manifests() {
            let registration = custom::registration(&tx, &manifest.subject_prefix).map_err(internal)?;
            if !registration.is_some_and(|(hash, registered, state)| {
                state == "ready" && registered.subject_prefix == manifest.subject_prefix
                    && notice::validate_registration(&registered, &hash).is_ok()
            }) {
                unavailable_reason = Some("reviewed notice registrations are missing or conflicting".into());
            }
        }
        // Quarantine headers are enough to fence readiness. Never decode opaque replica bytes.
        let mut headers = tx.prepare(
            "SELECT subject_hint,kind_hint FROM replica_records WHERE state IN ('unknown','invalid') AND (
             subject_hint GLOB 'custom/attention/*' OR subject_hint IN ('custom','custom/','custom/attention')
             OR subject_hint GLOB 'custom/st3-kinds/attention/notice-*/v1' OR kind_hint='custom.attention' OR kind_hint GLOB 'custom.attention.*'
             OR kind_hint='attention.notice' OR kind_hint GLOB 'attention.notice-*')",
        ).map_err(internal)?;
        let mut rows = headers.query([]).map_err(internal)?;
        while let Some(row) = rows.next().map_err(internal)? {
            let subject: Option<String> = row.get(0).map_err(internal)?;
            let kind: Option<String> = row.get(1).map_err(internal)?;
            if subject.as_deref().is_some_and(|subject| notice::is_reserved_subject(subject)
                || notice::manifests().iter().any(|manifest| manifest.subject() == subject))
                || kind.as_deref().is_some_and(notice::is_reserved_kind) {
                unavailable_reason = Some("reserved notice prerequisites are quarantined".into());
            }
        }
        // The kind index bounds candidate traversal; only addressed current-pin creations qualify.
        let sources = tx.prepare(/* sqlite-large-value: allow-for-now lv-h6c6fb8f237c8 owner=fractal_store::store::notice_materialization TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT DISTINCT subject FROM claims WHERE kind='custom.attention.notice-created'
             AND subject>=?1 AND subject<?2 AND json_extract(body,'$.fields.recipient')=?3
             AND json_extract(body,'$.fields._registration')=?4
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
             AND claims.id=(SELECT created.id FROM claims created
                 WHERE created.subject=claims.subject AND created.kind='custom.attention.notice-created'
                 AND json_extract(created.body,'$.fields._registration')=?4
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=created.id AND state='repaired')
                 ORDER BY CANONICAL_ASC(created) LIMIT 1)
             ORDER BY subject"),
        ).map_err(internal)?.query_map(
            params![notice::SOURCE_PREFIX, "custom/attention/source0", person, notice::HASHES[0]],
            |row| row.get::<_, String>(0),
        ).map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        let mut result = NoticeMaterializationCollection { state: NoticeCollectionState::Ready, notices: Vec::new() };
        if let Some(reason) = &unavailable_reason {
            result.state = NoticeCollectionState::Unavailable { reason: reason.clone() };
        }
        for source_id in sources {
            // Independently establish the exact creation identity and recipient, not cached fields.
            let (count, creation, recipient): (i64, Option<String>, Option<String>) = tx.query_row(
                "SELECT count(*),min(id),min(json_extract(body,'$.fields.recipient')) FROM claims
                 WHERE subject=?1 AND kind='custom.attention.notice-created'
                 AND json_extract(body,'$.fields._registration')=?2
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')",
                params![source_id, notice::HASHES[0]], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).map_err(internal)?;
            let id = notice::id_for_source(&source_id).map_err(custom::typed)?;
            let projection = custom::source(&tx, &source_id).map_err(internal)?;
            let healthy = count == 1 && recipient.as_deref() == Some(person)
                && projection.as_ref().is_some_and(|row| {
                    row["state"] == "ready" && row["registration"] == notice::HASHES[0]
                        && row["registered_kind"] == notice::manifests()[0].kind
                        && row["provenance"]["creation"]["claim_id"].as_str() == creation.as_deref()
                });
            let state = if let Some(reason) = &unavailable_reason {
                NoticeMaterializationState::Unavailable { reason: reason.clone() }
            } else if !healthy {
                NoticeMaterializationState::Unavailable { reason: "source creation or custom projection is missing, invalid, or conflicting".into() }
            } else if let Some(binding) = &binding {
                // Only admitted claims from ready current-pin snapshot subjects participate.
                let snapshot = tx.query_row(/* sqlite-large-value: allow-for-now lv-h3551f62fadfd owner=fractal_store::store::notice_materialization TODO=trace-statement-and-value-representation */ &canonical_sql(
                    "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                     JOIN custom_sources s ON s.subject=claims.subject
                     WHERE claims.kind IN ('custom.attention.snapshot-created','custom.attention.snapshot-updated')
                     AND claims.subject>=?1 AND claims.subject<?2 AND s.kind='attention.notice-snapshot'
                     AND s.state='ready' AND s.registration=?3
                     AND json_extract(claims.body,'$.fields._registration')=?3
                     AND json_extract(claims.body,'$.fields.source_id')=?4
                     AND json_extract(claims.body,'$.fields.creation_claim')=?5
                     AND json_extract(claims.body,'$.fields.runtime_binding')=?6
                     AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
                     ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                    params![notice::SNAPSHOT_PREFIX, "custom/attention/snapshot0", notice::HASHES[1], source_id, creation, binding],
                    |row| row.get::<_, String>(0),
                ).optional().map_err(internal)?;
                match snapshot {
                    Some(snapshot_claim) => NoticeMaterializationState::Ready { snapshot_claim, binding_claim: binding.clone() },
                    None => NoticeMaterializationState::Rebuilding { reason: "no ready snapshot for the current runtime binding".into() },
                }
            } else {
                NoticeMaterializationState::Unavailable { reason: "missing notice-runtime designation".into() }
            };
            match &state {
                NoticeMaterializationState::Unavailable { reason } => result.state = NoticeCollectionState::Unavailable { reason: reason.clone() },
                NoticeMaterializationState::Rebuilding { reason } if !matches!(result.state, NoticeCollectionState::Unavailable { .. }) => {
                    result.state = NoticeCollectionState::Rebuilding { reason: reason.clone() };
                }
                _ => {}
            }
            result.notices.push(NoticeMaterialization { id, source_id, state });
        }
        Ok(result)
    }
}
