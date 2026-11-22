//! Runtime evidence is reduced per incarnation and observing origin. Presentation folds
//! merge unrelated observations and must never decide workspace ownership or runtime death.
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Liveness {
    #[default]
    Unknown,
    Running,
    Ended,
}

#[derive(Default)]
struct OriginEvidence {
    liveness: Liveness,
    status: Option<String>,
    claim: Option<String>,
    workspaces: std::collections::BTreeSet<String>,
    unresolved_workspace: bool,
}

#[derive(Default)]
struct IncarnationEvidence {
    origins: BTreeMap<String, OriginEvidence>,
    named_end: bool,
    ever_ran: bool,
}

#[derive(Default)]
struct AgentEvidence {
    incarnations: BTreeMap<String, IncarnationEvidence>,
    anonymous: BTreeMap<String, Liveness>,
    newest_incarnation: Option<String>,
    saw_observation: bool,
    missing_host: bool,
}

struct Observation {
    subject: String,
    claim: String,
    origin: String,
    incarnation: Option<String>,
    status: Option<String>,
    workspace_present: bool,
    workspace: Option<String>,
}


impl AgentEvidence {
    fn observe(&mut self, observation: Observation) {
        self.saw_observation = true;
        if observation.origin.is_empty() {
            self.missing_host = true;
            return;
        }
        if observation.status.as_deref().is_some_and(crate::store::is_incarnation_end_status) {
            if let Some(incarnation) = observation.incarnation {
                // A named end matches only this incarnation, on any observing host.
                let evidence = self.incarnations.entry(incarnation).or_default();
                evidence.named_end = true;
                for origin in evidence.origins.values_mut() {
                    origin.liveness = Liveness::Ended;
                }
            } else {
                // An unnamed end says only that this host's copies ended. It cannot
                // kill a newer incarnation observed by another host.
                for incarnation in self.incarnations.values_mut() {
                    if let Some(origin) = incarnation.origins.get_mut(&observation.origin) {
                        origin.liveness = Liveness::Ended;
                    }
                }
                self.anonymous.insert(observation.origin, Liveness::Ended);
            }
            return;
        }
        let Some(incarnation) = observation.incarnation else {
            // Unattributed runtime evidence cannot establish a specific incarnation.
            // Preserve the gap until an unnamed end on this host resolves its copies.
            self.anonymous.insert(observation.origin, Liveness::Unknown);
            return;
        };
        let entry = self.incarnations.entry(incarnation);
        if observation.status.as_deref() == Some("running")
            && !matches!(&entry, std::collections::btree_map::Entry::Occupied(known) if known.get().ever_ran)
        {
            self.newest_incarnation = Some(entry.key().clone());
        }
        let evidence = entry.or_default();
        let default_liveness = if evidence.named_end { Liveness::Ended } else { Liveness::Unknown };
        let origin = evidence.origins.entry(observation.origin).or_insert_with(|| OriginEvidence {
            liveness: default_liveness,
            ..Default::default()
        });
        evidence.named_end = false;
        if observation.status.as_deref() == Some("running") {
            origin.liveness = Liveness::Running;
            evidence.ever_ran = true;
        } else if origin.liveness == Liveness::Ended {
            origin.liveness = Liveness::Unknown;
        }
        origin.status = observation.status;
        origin.claim = Some(observation.claim);
        if observation.workspace_present {
            // Retain every witnessed path. Reusing an incarnation identifier after
            // an end cannot prove that its earlier workspace is no longer in use.
            match observation.workspace {
                Some(workspace) if Path::new(&workspace).is_absolute() => {
                    origin.workspaces.insert(workspace);
                    origin.unresolved_workspace = false;
                }
                _ => origin.unresolved_workspace = true,
            }
        }
    }
}

fn gap(
    subject: &crate::model::SubjectStatus,
    incarnation: Option<&str>,
    reason: IncompleteReason,
    known_origin: Option<&str>,
    host: Option<&str>,
) -> Option<Value> {
    let known_host = known_origin.map(client_host_id);
    if let (Some(host), Some(known)) = (host, known_host.as_deref())
        && host != known
    {
        return None;
    }
    Some(json!({
        "id": subject.subject,
        "layer": layer(subject),
        "incarnation_id": incarnation,
        "host": known_host,
        "reason": reason,
    }))
}

fn layer(subject: &crate::model::SubjectStatus) -> ActivityLayer {
    if subject.projection.layer == "current" { ActivityLayer::Current } else { ActivityLayer::Historical }
}

fn append_rows(rows: &mut Vec<Value>, subject: &crate::model::SubjectStatus, evidence: AgentEvidence, host: Option<&str>) {
    if !evidence.saw_observation {
        rows.extend(gap(subject, None, IncompleteReason::NoActual, None, host));
        return;
    }
    if evidence.missing_host {
        rows.extend(gap(subject, None, IncompleteReason::MissingHost, None, host));
    }
    if evidence.anonymous.values().any(|state| *state != Liveness::Ended) {
        rows.extend(gap(subject, None, IncompleteReason::MissingIncarnation, None, host));
    }
    for (incarnation, evidence_for_incarnation) in evidence.incarnations {
        let mut origins = evidence_for_incarnation.origins.into_iter()
            .filter(|(_, evidence)| evidence.liveness != Liveness::Ended);
        let Some((origin, observation)) = origins.next() else { continue; };
        let mut emit = |mut row: Value| {
            if let Some(newer) = evidence.newest_incarnation.as_deref()
                .filter(|newer| *newer != incarnation.as_str())
            {
                // Advisory only: another start is not matching end evidence.
                row["superseded_by"] = json!(newer);
            }
            rows.push(row);
        };
        if origins.next().is_some() {
            if let Some(row) = gap(subject, Some(&incarnation), IncompleteReason::IndeterminateAuthority, None, host) {
                emit(row);
            }
            continue;
        }
        if observation.liveness != Liveness::Running {
            if let Some(row) = gap(subject, Some(&incarnation), IncompleteReason::UnresolvedLiveness, None, host) {
                emit(row);
            }
            continue;
        }
        let observed_host = client_host_id(&origin);
        if host.is_some_and(|host| host != observed_host.as_str()) {
            continue;
        }
        let Some(claim) = observation.claim else {
            if let Some(row) = gap(subject, Some(&incarnation), IncompleteReason::NoActual, None, host) {
                emit(row);
            }
            continue;
        };
        if observation.workspaces.is_empty() || observation.unresolved_workspace {
            if let Some(row) = gap(subject, Some(&incarnation), IncompleteReason::MissingWorkspace, Some(&origin), host) {
                emit(row);
            }
        }
        let (status, status_raw) = ActivityStatus::from_actual(observation.status.as_deref());
        for workspace in observation.workspaces {
            let mut row = json!({
                "id": subject.subject,
                "layer": layer(subject),
                "incarnation_id": &incarnation,
                "host": observed_host,
                "workspace": workspace,
                "status": status,
                "actual_claim": claim,
                "actual_origin": origin,
            });
            if let Some(raw) = status_raw {
                row["status_raw"] = json!(raw);
            }
            emit(row);
        }
    }
}

pub(super) fn read(
    store: &Store,
    subjects: &[crate::model::SubjectStatus],
    cut: u64,
    host: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let connection = store.readers.get();
    let sql = smallclaims::store::canonical_sql(&format!(
        "SELECT claims.subject, claims.id, claims.origin,
                json_extract(claims.body, '$.fields.incarnation_id'),
                json_extract(claims.body, '$.fields.status'),
                json_type(claims.body, '$.fields.workspace') IS NOT NULL,
                CASE WHEN json_type(claims.body, '$.fields.workspace')='text'
                     THEN json_extract(claims.body, '$.fields.workspace') END
         FROM json_each(?1) subjects
         JOIN claims INDEXED BY claims_subject_kind_index ON claims.subject=subjects.value
         WHERE claims.store_index<=?2 AND claims.kind='runtime.observed'
           AND {}
         ORDER BY claims.subject, CANONICAL_ASC(claims)",
        crate::store::UNREPAIRED_CLAIM,
    ));
    let mut query = connection.prepare_cached(&sql)?;
    let mut inventory = Vec::new();
    for chunk in subjects.chunks(crate::store::card_fold::CHUNK) {
        smallclaims::read_budget::check()?;
        let names = chunk.iter().map(|subject| &subject.subject).collect::<Vec<_>>();
        let mut observations = query.query_map(rusqlite::params![serde_json::to_string(&names)?, cut], |row| {
            Ok(Observation {
                subject: row.get(0)?, claim: row.get(1)?, origin: row.get(2)?,
                incarnation: row.get::<_, Option<String>>(3)?.filter(|id| !id.is_empty()),
                status: row.get(4)?, workspace_present: row.get(5)?, workspace: row.get(6)?,
            })
        })?.peekable();
        for subject in chunk {
            let mut evidence = AgentEvidence::default();
            while observations.peek().is_some_and(|row| row.as_ref().map_or(true, |row| row.subject == subject.subject)) {
                smallclaims::read_budget::check()?;
                if let Some(observation) = observations.next() {
                    evidence.observe(observation?);
                }
            }
            append_rows(&mut inventory, subject, evidence, host);
        }
    }
    Ok(inventory)
}
