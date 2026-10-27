//! Indexed launch evidence usable inside the publication writer transaction.
use super::*;
use crate::model::MemberSpec;
use crate::publication::{Interruption, interruption};

pub type Guard = dyn Fn(&DesiredSubject, &Check) -> Result<(), St3Error> + Sync;

#[derive(Default)]
pub struct CommitChecks<'a> {
    pub owned: Option<&'a owned_sets::Options>,
    pub guard: Option<&'a Guard>,
}

pub struct Check {
    pub incarnation: Option<String>,
    pub launched: Option<MemberSpec>,
    pub interruption: Interruption,
    pub unknown_reason: Option<&'static str>,
}

pub fn launched_member(
    connection: &Connection,
    subject: &str,
    origin: &str,
    incarnation: &str,
    at: u64,
    local_origin: &str,
) -> Result<Option<MemberSpec>> {
    if !publication_facts::ready(connection)? {
        return Ok(None);
    }
    let query = format!(
        "SELECT claims.store_index, claims.body FROM local_publication_claims AS facts INDEXED BY local_publication_claims_lookup
         JOIN claims ON claims.id=facts.claim_id
         WHERE facts.subject=?1 AND facts.origin=?2 AND facts.incarnation=?3
           AND facts.kind='runtime.action.succeeded'
           AND claims.store_index<=?4 AND {UNREPAIRED_CLAIM} ORDER BY facts.sort_key DESC LIMIT 1"
    );
    let shared: Option<(u64, String)> = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hdd1697922ff7 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ &query)?
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h176268083051 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ params![subject, origin, incarnation, at.min(i64::MAX as u64)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // Start successes may be retained locally. Those records sort after their graph cut;
    // the newest local start for this incarnation supersedes an earlier graph start.
    let local: Option<(u64, String)> = if origin == local_origin {
        connection
            .prepare_cached(
                "SELECT local_observations.after_store_index, local_observations.body
                 FROM local_publication_starts AS facts
                 JOIN local_observations ON local_observations.id=facts.observation_id
                 WHERE facts.subject=?1 AND facts.incarnation=?2
                   AND local_observations.after_store_index<=?3
                 ORDER BY facts.observation_id DESC LIMIT 1",
            )?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-he71c6d4b5146 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ params![subject, incarnation, at.min(i64::MAX as u64)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
    } else {
        None
    };
    let body = shared
        .into_iter()
        .chain(local)
        .max_by_key(|(index, _)| *index)
        .map(|(_, body)| body);
    let Some(body) = body else {
        return Ok(None);
    };
    let body: Value = serde_json::from_str(&body)?;
    let Some(token) = body
        .pointer("/fields/desired_token")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let body: Option<String> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h98979ff8bdcc owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ &format!(
        "SELECT body FROM claims WHERE id=?1 AND subject=?2 AND kind='intent.desired' AND {UNREPAIRED_CLAIM}"
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-h70afe4d4f91d owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ params![token,subject], |row| row.get(0)).optional()?;
    Ok(body
        .and_then(|body| serde_json::from_str::<DesiredSubject>(&body).ok())
        .and_then(|desired| desired.member))
}

pub fn check(
    connection: &Connection,
    proposed: &DesiredSubject,
    at: u64,
    local_origin: &str,
) -> Result<Check> {
    smallclaims::touched::note_read(|| proposed.subject.clone());
    if !publication_facts::ready(connection)? {
        return Ok(Check {
            incarnation: None,
            launched: None,
            interruption: Interruption::Unknown,
            unknown_reason: Some("publication-evidence-incomplete"),
        });
    }
    // Seek the next origin prefix instead of DISTINCT's walk over every historical row.
    // One runtime per origin prevents a stopped peer from hiding a running incarnation.
    let mut origins = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h7ee321bcff24 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ "WITH RECURSIVE origins(origin) AS (
            SELECT MIN(origin) FROM local_publication_claims
                INDEXED BY local_publication_runtime_lookup
                WHERE subject=?1 AND kind='runtime.observed'
            UNION ALL
            SELECT (SELECT MIN(origin) FROM local_publication_claims
                INDEXED BY local_publication_runtime_lookup
                WHERE subject=?1 AND kind='runtime.observed' AND origin>origins.origin)
            FROM origins WHERE origin IS NOT NULL
         ) SELECT origin FROM origins WHERE origin IS NOT NULL",
    )?;
    let origins = origins
        .query_map([&proposed.subject], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let query = format!(
        "SELECT claims.body FROM local_publication_claims AS facts
         INDEXED BY local_publication_runtime_lookup JOIN claims ON claims.id=facts.claim_id
         WHERE facts.subject=?1 AND facts.origin=?2 AND facts.kind='runtime.observed'
           AND claims.store_index<=?3 AND {UNREPAIRED_CLAIM} ORDER BY facts.sort_key DESC LIMIT 1"
    );
    let mut unchanged = Check {
        incarnation: None,
        launched: None,
        interruption: Interruption::None,
        unknown_reason: None,
    };
    for origin in origins {
        let body: Option<String> = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hb4a17852ca35 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ &query)?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-hffd9a012f0b4 owner=fractal_store::store::publication TODO=trace-statement-and-value-representation */ params![proposed.subject, origin, at.min(i64::MAX as u64)],
                |row| row.get(0),
            )
            .optional()?;
        let Some(body) = body else {
            continue;
        };
        let body: Value = serde_json::from_str(&body)?;
        let fields = body.get("fields").unwrap_or(&body);
        if fields["status"] != "running" {
            continue;
        }
        let incarnation = fields["incarnation_id"].as_str().map(str::to_owned);
        let launched = incarnation
            .as_deref()
            .map(|incarnation| {
                launched_member(
                    connection,
                    &proposed.subject,
                    &origin,
                    incarnation,
                    at,
                    local_origin,
                )
            })
            .transpose()?
            .flatten();
        let interruption = interruption(proposed, true, incarnation.as_deref(), launched.as_ref());
        let check = Check {
            incarnation,
            launched,
            unknown_reason: (interruption == Interruption::Unknown)
                .then_some("publication-launch-unknown"),
            interruption,
        };
        if check.interruption != Interruption::None {
            return Ok(check);
        }
        unchanged = check;
    }
    Ok(unchanged)
}

impl Store {
    pub fn publication_action(
        &self,
        proposed: &DesiredSubject,
    ) -> Result<Option<PlannedAction>> {
        let connection = self.readers.get();
        let check = check(
            &connection,
            proposed,
            current_index(&connection)?,
            &self.origin,
        )?;
        let (action, reason) = match check.interruption {
            Interruption::None => return Ok(None),
            Interruption::Stop => (
                "stop",
                "the publication would stop a running incarnation".into(),
            ),
            Interruption::Restart(changes) => (
                "restart",
                format!(
                    "the launched {} changed; a running seat restarts on its last session",
                    changes.join(" and ")
                ),
            ),
            Interruption::Unknown => (
                "observe-or-start",
                check
                    .unknown_reason
                    .unwrap_or("publication-launch-unknown")
                    .into(),
            ),
        };
        Ok(Some(PlannedAction {
            subject: proposed.subject.clone(),
            action: action.into(),
            reason,
        }))
    }
}
