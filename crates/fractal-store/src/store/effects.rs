//! Durable native-message admission. Absence on a peer is never permission to send.
use super::*;
use fractal_kernel::model::EffectOwner;
pub use fractal_schema::effects::{EffectClass, EffectRecord};
use fractal_schema::effects::{KIND_CONFIRMED, KIND_IN_FLIGHT, KIND_INTENDED, KIND_UNRESOLVED};

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h4d33c1e58bfd owner=fractal_store::store::effects TODO=chunk-or-reference-variable-columns */
        "CREATE INDEX IF NOT EXISTS claims_effect_step_index
         ON claims(json_extract(body,'$.fields.bound_step_run'), subject)
         WHERE kind IN ('effect.intended','effect.in-flight','effect.confirmed','effect.unresolved');",
    )?;
    Ok(())
}

#[derive(Debug)]
pub enum EffectAdmission {
    Originate(EffectRecord),
    Recover(EffectRecord),
    /// A successful transaction value: parking must survive the refusal.
    Blocked(FractalError),
}

pub struct EffectTarget<'a> {
    pub provider: &'a str,
    pub class: EffectClass,
    pub target: &'a str,
}

pub struct EffectResolution<'a> {
    pub outcome: &'a str,
    pub provider_ref: Option<&'a str>,
    pub evidence: &'a [String],
    pub key: &'a str,
}

/// JSON tuples are unambiguous even when mission names or user keys contain separators.
pub fn effect_key(owner: &EffectOwner) -> Result<String, FractalError> {
    Ok(format!("native-effect/{}", serde_json::to_string(&(owner.mission.strip_prefix("mission/").unwrap_or(&owner.mission), &owner.key)).map_err(internal)?))
}

pub fn native_message_key(owner: &EffectOwner) -> Result<String, FractalError> {
    Ok(format!("native-effect-message/{}", operation_id_for_key(&effect_key(owner)?)))
}

fn intended_input(record: &EffectRecord, key: &str) -> Result<ClaimInput, FractalError> {
    Ok(ClaimInput {
        subject: record.effect_id.clone(), kind: KIND_INTENDED.into(), actor: Some(record.lease_owner.clone()),
        fields: serde_json::from_value(serde_json::to_value(record).map_err(internal)?).map_err(internal)?,
        evidence: record.evidence.clone(), expected_subject: None, idempotency_key: Some(key.into()),
    })
}

fn intended_tx(tx: &Connection, id: &str) -> Result<Option<EffectRecord>, FractalError> {
    let claim: Option<String> = tx.query_row(
        /* sqlite-large-value: allow-for-now lv-h2cc8692633a8 owner=fractal_store::store::effects TODO=trace-statement-and-value-representation */
        &canonical_sql("SELECT id FROM claims WHERE subject=?1 AND kind=?2 ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
        params![id, KIND_INTENDED], |row| row.get(0)).optional().map_err(internal)?;
    claim.map(|id| {
        let claim = claim_by_id_tx(tx, &id).map_err(internal)?.ok_or_else(|| internal("the effect claim is missing"))?;
        serde_json::from_value(claim.body["fields"].clone()).map_err(internal)
    }).transpose()
}

/// A resolution clears only episodes in its causal past, never concurrent observations.
pub(super) fn effect_is_unresolved_tx(tx: &Connection, subject: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        /* sqlite-large-value: allow-for-now lv-hb9a49c3cca1c owner=fractal_store::store::effects TODO=trace-statement-and-value-representation */
        "WITH RECURSIVE resolved(id) AS (
           SELECT p.value FROM claims r, json_each(r.predecessors) p
           WHERE r.subject=?1 AND r.kind='effect.confirmed'
             AND json_extract(r.body,'$.fields.resolution')='human'
           UNION
           SELECT p.value FROM resolved a JOIN claims c ON c.id=a.id, json_each(c.predecessors) p
           WHERE c.subject=?1
         )
         SELECT EXISTS(SELECT 1 FROM claims u WHERE u.subject=?1 AND u.kind='effect.unresolved'
           AND NOT EXISTS(SELECT 1 FROM resolved WHERE resolved.id=u.id))",
        [subject], |row| row.get(0))
}

fn latest_tx(tx: &Connection, id: &str) -> Result<Option<(String, EffectRecord)>, FractalError> {
    let unresolved = effect_is_unresolved_tx(tx, id).map_err(internal)?;
    let row: Option<(String, String)> = tx.query_row(
        /* sqlite-large-value: allow-for-now lv-h01c4365780a3 owner=fractal_store::store::effects TODO=trace-statement-and-value-representation */
        &canonical_sql("SELECT kind,id FROM claims WHERE subject=?1 AND kind IN (?2,?3,?4,?5)
         AND (?6 OR kind!=?5)
         ORDER BY CASE WHEN kind=?4 AND json_extract(body,'$.fields.resolution')='human' THEN 5
                  WHEN kind=?5 THEN 6 WHEN kind=?4 THEN 3 WHEN kind=?3 THEN 2 ELSE 1 END DESC,
                  CANONICAL_DESC(claims) LIMIT 1"),
        params![id, KIND_INTENDED, KIND_IN_FLIGHT, KIND_CONFIRMED, KIND_UNRESOLVED, unresolved],
        |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(internal)?;
    row.map(|(kind, id)| {
        let claim = claim_by_id_tx(tx, &id).map_err(internal)?.ok_or_else(|| internal("the effect claim is missing"))?;
        Ok((kind, serde_json::from_value(claim.body["fields"].clone()).map_err(internal)?))
    }).transpose()
}

pub(super) fn step_has_unresolved_effect_tx(connection: &Connection, step: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare_cached(
        "SELECT DISTINCT subject FROM claims
         WHERE json_extract(body,'$.fields.bound_step_run')=?1
           AND kind IN ('effect.intended','effect.in-flight','effect.confirmed','effect.unresolved')")?;
    for subject in statement.query_map([step], |row| row.get::<_, String>(0))? {
        if effect_is_unresolved_tx(connection, &subject?)? { return Ok(true); }
    }
    Ok(false)
}

pub(super) fn effect_views_for_step_tx(connection: &Connection, step: &str) -> rusqlite::Result<Vec<fractal_kernel::model::EffectRunView>> {
    let mut statement = connection.prepare_cached(
        "SELECT DISTINCT subject FROM claims WHERE json_extract(body,'$.fields.bound_step_run')=?1
         AND kind IN ('effect.intended','effect.in-flight','effect.confirmed','effect.unresolved')")?;
    let subjects = statement.query_map([step], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    subjects.into_iter().filter_map(|subject| match latest_tx(connection, &subject) {
        Ok(Some((kind, record))) => Some(Ok(fractal_kernel::model::EffectRunView {
            effect_id: subject, state: record.outcome.clone().unwrap_or_else(||
                kind.strip_prefix("effect.").unwrap_or(&kind).to_owned()), provider_ref: record.provider_ref,
        })),
        Ok(None) => None,
        Err(error) => Some(Err(rusqlite::Error::ToSqlConversionFailure(Box::new(error)))),
    }).collect()
}

fn append_effect_tx(tx: &Transaction<'_>, origin: &str, kind: &str, record: &EffectRecord) -> Result<(), FractalError> {
    // Bind every observed episode. A concurrent unresolved receipt remains fenced on merge.
    let mut statement = tx.prepare_cached("SELECT id FROM claims WHERE subject=?1").map_err(internal)?;
    let predecessors = statement.query_map([&record.effect_id], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
    append_claim_tx(tx, origin, &record.effect_id, kind,
        record.recovering_actor.as_deref().or(Some(&record.lease_owner)),
        &json!({"fields":record,"evidence":record.evidence}), &predecessors, None).map_err(internal)?;
    Ok(())
}

fn park_tx(tx: &Transaction<'_>, origin: &str, record: &EffectRecord, step: &str, reason: &str, recovering_actor: &str) -> Result<(), FractalError> {
    if effect_is_unresolved_tx(tx, &record.effect_id).map_err(internal)? {
        if step != record.bound_step_run { park_step_tx(tx, origin, step, record)?; }
        return Ok(());
    }
    let mut unresolved = record.clone();
    unresolved.resolution = None;
    unresolved.outcome = None;
    unresolved.error = Some(reason.into());
    unresolved.recovering_actor = Some(recovering_actor.into());
    append_effect_tx(tx, origin, KIND_UNRESOLVED, &unresolved)?;
    let run: Option<(String, String)> = tx.query_row(
        "SELECT m.id,CASE WHEN m.phase='terminal' THEN 'normal' ELSE m.phase END
         FROM step_runs s JOIN mission_runs m ON m.id=s.run_id
         WHERE s.subject=?1 AND s.generation_id=m.current_generation_id",
        [&record.bound_step_run], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(internal)?;
    if let Some((run, phase)) = run {
        let subject = format!("mission-run/{run}");
        let predecessors = latest_claim_id_tx(tx, &subject).map_err(internal)?.into_iter().collect::<Vec<_>>();
        let claim = append_claim_tx(tx, origin, &subject, "mission-run.state", Some("daemon/runtime"),
            &json!({"fields":{"status":"blocked","phase":phase,"reason":"effect-unresolved"},
                "evidence":[record.effect_id]}), &predecessors, None).map_err(internal)?;
        project_mission_run_update(tx, &claim)?;
    }
    park_step_tx(tx, origin, &record.bound_step_run, record)?;
    if step != record.bound_step_run { park_step_tx(tx, origin, step, record)?; }
    Ok(())
}

fn park_step_tx(tx: &Transaction<'_>, origin: &str, step: &str, record: &EffectRecord) -> Result<(), FractalError> {
    let predecessors = latest_claim_id_tx(tx, step).map_err(internal)?.into_iter().collect::<Vec<_>>();
    let claim = append_claim_tx(tx, origin, step, "step-run.state", Some("daemon/runtime"),
        &json!({"fields":{"status":"blocked","reason":"effect-unresolved","attempt":record.bound_attempt},
            "evidence":[record.effect_id]}), &predecessors, None).map_err(internal)?;
    project_mission_run_update(tx, &claim)
}

fn lease_tx(tx: &Connection, owner: &EffectOwner, sender: &str) -> Result<(), FractalError> {
    let valid: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM step_runs s JOIN mission_runs m ON m.id=s.run_id
         WHERE s.subject=?1 AND m.mission_id=?2 AND s.attempt=?3 AND s.lease_owner=?4
         AND s.lease_incarnation=?5 AND CAST(s.lease_expires_at_unix_ms AS INTEGER)>CAST(?6 AS INTEGER)
         AND s.status IN ('claimed','working','submitted'))",
        params![owner.step, owner.mission.strip_prefix("mission/").unwrap_or(&owner.mission), owner.attempt, sender, owner.incarnation, now_ms().to_string()],
        |row| row.get(0)).map_err(internal)?;
    if !valid || !work_incarnation_live(tx, sender, &owner.incarnation).map_err(internal)? {
        return Err(FractalError::new("stale-work-lease", "the effect requires the exact current live work lease"));
    }
    Ok(())
}

fn may_originate_tx(tx: &Connection, origin: &str, step: &str, caller: &str,
    attempt: u32, incarnation: &str) -> Result<bool, FractalError> {
    // Step rows originate in mission-run.created, not in a dedicated step-created claim.
    // That admission receipt is durable (fractal-schema/retention.toml).
    let anchor: Option<String> = tx.query_row(
        "SELECT CASE WHEN COUNT(DISTINCT c.origin)=1 THEN MIN(c.origin) END
         FROM step_runs s JOIN claims c ON c.subject='mission-run/'||s.run_id
         AND c.kind='mission-run.created' WHERE s.subject=?1",
        [step], |row| row.get(0)).map_err(internal)?;
    if anchor.as_deref() != Some(origin) { return Ok(false); }
    let claimed: Option<String> = tx.query_row(
        "SELECT CASE WHEN COUNT(DISTINCT origin)=1 THEN MIN(origin) END
         FROM claims WHERE subject=?1 AND kind='work.claimed' AND actor=?2
         AND json_extract(body,'$.fields.attempt')=?3
         AND json_extract(body,'$.fields.claim_incarnation')=?4",
        params![step, caller, attempt, incarnation], |row| row.get(0)).map_err(internal)?;
    Ok(claimed.as_deref() == Some(origin))
}

fn operation_conflicted_tx(tx: &Connection, operation: &str) -> Result<bool, FractalError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_operation_index
         WHERE json_extract(body,'$._operation.id')=?1
         AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims p WHERE p.id=claims.id)
         GROUP BY json_extract(body,'$._operation.id') HAVING COUNT(DISTINCT origin)>1)
         OR EXISTS(SELECT 1 FROM operations WHERE id=?1 AND state='conflict')",
        [operation], |row| row.get(0),
    ).map_err(internal)
}

impl Store {
    /// Effective wire state, including causal human resolution, not arrival order.
    pub fn effect_state(&self, subject: &str) -> Result<Option<(String, EffectRecord)>, FractalError> {
        Ok(latest_tx(&self.readers.get(), subject)?.map(|(kind, record)| {
            let state = record.outcome.clone().unwrap_or_else(||
                kind.strip_prefix("effect.").unwrap_or(&kind).to_owned());
            (state, record)
        }))
    }

    pub fn effect_by_key(&self, key: &str) -> Result<Option<(String, EffectRecord)>, FractalError> {
        self.effect_state(&format!("effect/{}", operation_id_for_key(key)))
    }

    pub fn effects_for_step(&self, step: &str) -> Result<Vec<(String, EffectRecord)>, FractalError> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT DISTINCT subject FROM claims WHERE json_extract(body,'$.fields.bound_step_run')=?1
             AND kind IN ('effect.intended','effect.in-flight','effect.confirmed','effect.unresolved')")
            .map_err(internal)?;
        let subjects = statement.query_map([step], |row| row.get::<_, String>(0)).map_err(internal)?
            .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        subjects.into_iter().filter_map(|subject| match latest_tx(&connection, &subject) {
            Ok(Some((kind, record))) => Some(Ok((record.outcome.clone().unwrap_or_else(||
                kind.strip_prefix("effect.").unwrap_or(&kind).to_owned()), record))),
            Ok(None) => None, Err(error) => Some(Err(error)),
        }).collect()
    }

    pub fn unresolved_effects(&self) -> Result<Vec<EffectRecord>, FractalError> {
        let subjects = {
            let connection = self.readers.get();
            let mut statement = connection.prepare_cached(
                "SELECT DISTINCT subject FROM claims WHERE kind='effect.unresolved'").map_err(internal)?;
            statement.query_map([], |row| row.get::<_, String>(0)).map_err(internal)?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
        };
        let mut records = Vec::new();
        for subject in subjects {
            if let Some((state, record)) = self.effect_state(&subject)? && state == "unresolved" { records.push(record); }
        }
        Ok(records)
    }

    /// The caller supplies server-established person authority, never the request actor alone.
    pub fn resolve_effect_authorized(&self, subject: &str, actor: &str, authorized_person: &str,
        resolution: EffectResolution<'_>) -> Result<EffectRecord, FractalError> {
        let EffectResolution { outcome, provider_ref, evidence, key } = resolution;
        if actor != authorized_person || !actor.starts_with("person/") || actor.len() == 7 || actor.matches('/').count() != 1 {
            return Err(FractalError::new("forbidden", "only an authenticated person can resolve an effect"));
        }
        if !matches!(outcome, "confirmed" | "abandoned") || key.trim().is_empty()
            || evidence.is_empty() || evidence.iter().any(|value| value.trim().is_empty())
            || provider_ref.is_some_and(|value| value.trim().is_empty())
            || (outcome == "confirmed" && provider_ref.is_none())
        {
            return Err(FractalError::new("invalid-effect-resolution", "resolution requires an outcome, key, evidence and a reference when confirmed"));
        }
        let record = self.effect_state(subject)?.ok_or_else(||
            FractalError::new("not-found", "the effect does not exist"))?.1;
        self.with_run_fence(&record.bound_step_run, |tx| {
            let mut request = json!({"actor":actor,"outcome":outcome,"evidence":evidence});
            if let Some(reference) = provider_ref { request["provider_ref"] = json!(reference); }
            let mut statement = tx.prepare_cached(
                "SELECT body FROM claims WHERE subject=?1 AND kind='effect.confirmed'
                 AND json_extract(body,'$._resolution.key')=?2").map_err(internal)?;
            let rows = statement.query_map(params![subject,key], |row| row.get::<_, String>(0)).map_err(internal)?;
            let mut existing = None;
            for body in rows {
                let body: Value = serde_json::from_str(&body.map_err(internal)?).map_err(internal)?;
                if body["_resolution"]["request"] != request {
                    return Err(FractalError::new("idempotency-conflict", "the resolution key already identifies different terms"));
                }
                if existing.is_none() {
                    existing = Some(serde_json::from_value(body["fields"].clone()).map_err(internal)?);
                }
            }
            if let Some(existing) = existing { return Ok(existing); }
            let Some((kind, _)) = latest_tx(tx, subject)? else { return Err(internal("effect disappeared")); };
            if kind != KIND_UNRESOLVED {
                return Err(FractalError::new("effect-not-unresolved", "only an unresolved effect can be human resolved"));
            }
            // Start with the immutable intent, not a replacement worker's observations.
            let mut resolved = intended_tx(tx, subject)?.unwrap_or_else(|| record.clone());
            resolved.resolution = Some("human".into());
            resolved.outcome = (outcome == "abandoned").then(|| "abandoned".into());
            resolved.recovering_actor = Some(actor.into());
            resolved.provider_ref = provider_ref.map(str::to_owned);
            resolved.error = None;
            resolved.evidence = evidence.to_vec();
            let mut statement = tx.prepare_cached("SELECT id FROM claims WHERE subject=?1").map_err(internal)?;
            let predecessors = statement.query_map([subject], |row| row.get::<_, String>(0)).map_err(internal)?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
            append_claim_tx(tx, self.origin(), subject, KIND_CONFIRMED, Some(actor),
                &json!({"fields":resolved,"evidence":evidence,"_resolution":{"key":key,"request":request}}),
                &predecessors, None).map_err(internal)?;
            if !step_has_unresolved_effect_tx(tx, &resolved.bound_step_run).map_err(internal)? {
                let previous = latest_claim_id_tx(tx, &resolved.bound_step_run).map_err(internal)?;
                let blocked: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM step_runs WHERE subject=?1 AND status='blocked'
                       AND blocked_reason='effect-unresolved')", [&resolved.bound_step_run], |row| row.get(0)).map_err(internal)?;
                if blocked {
                    let claim = append_claim_tx(tx, self.origin(), &resolved.bound_step_run, "step-run.state",
                        Some("daemon/runtime"), &json!({"fields":{"status":"ready","reason":"effect-resolved","attempt":resolved.bound_attempt},
                            "evidence":[resolved.effect_id]}), &previous.into_iter().collect::<Vec<_>>(), None).map_err(internal)?;
                    project_mission_run_update(tx, &claim)?;
                }
            }
            Ok(resolved)
        })
    }

    /// Recovery is observation only. Native effects never gain new send authority.
    pub fn reconcile_effects(&self) -> Result<bool, FractalError> {
        let subjects = {
            let connection = self.readers.get();
            let mut statement = connection.prepare_cached(
                "SELECT DISTINCT subject FROM claims WHERE kind IN ('effect.intended','effect.in-flight')").map_err(internal)?;
            statement.query_map([], |row| row.get::<_, String>(0)).map_err(internal)?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
        };
        let mut changed = false;
        for subject in subjects {
            let Some((state, record)) = self.effect_state(&subject)? else { continue };
            if state == "unresolved" || record.resolution.as_deref() == Some("human") { continue; }
            let conflict = operation_conflicted_tx(&self.readers.get(), &record.operation_id)?;
            if conflict {
                self.with_run_fence(&record.bound_step_run, |tx|
                    park_tx(tx, self.origin(), &record, &record.bound_step_run, "idempotency-conflict", "daemon/runtime"))?;
                changed = true;
                continue;
            }
            if state == "confirmed" { continue; }
            let live = {
                let connection = self.readers.get();
                connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM step_runs WHERE subject=?1 AND attempt=?2
                     AND lease_owner=?3 AND lease_incarnation=?4
                     AND CAST(lease_expires_at_unix_ms AS INTEGER)>CAST(?5 AS INTEGER)
                     AND status IN ('claimed','working','submitted'))",
                    params![record.bound_step_run,record.bound_attempt,record.lease_owner,record.lease_incarnation,now_ms().to_string()],
                    |row| row.get::<_, bool>(0)).map_err(internal)?
                    && work_incarnation_live(&connection, &record.lease_owner, &record.lease_incarnation).map_err(internal)?
            };
            if live { continue; }
            if record.provider == "native-message" {
                let key = format!("native-effect-message/{}", record.operation_id);
                if operation_conflicted_tx(&self.readers.get(), &operation_id_for_key(&key))? {
                    self.unresolved_message_effect(&record, "idempotency-conflict", "daemon/runtime")?;
                    changed = true;
                    continue;
                }
                match self.operation_claim(&key).map_err(internal)? {
                    Some(message) => {
                        if let Err(error) = self.confirm_message_effect(&record, &message, "daemon/runtime") {
                            if error.code != "effect-evidence-mismatch" { return Err(error); }
                            self.unresolved_message_effect(&record, error.code, "daemon/runtime")?;
                        }
                    }
                    None => { self.unresolved_message_effect(&record, "delivery-unknown", "daemon/runtime")?; }
                }
            } else {
                self.unresolved_message_effect(&record, "provider-readback-unavailable", "daemon/runtime")?;
            }
            changed = true;
        }
        Ok(changed)
    }

    pub fn step_has_unresolved_effect(&self, step: &str) -> Result<bool, FractalError> {
        step_has_unresolved_effect_tx(&self.readers.get(), step).map_err(internal)
    }

    pub fn mission_run_has_unresolved_effect(&self, run: &str) -> Result<bool, FractalError> {
        // Cleanup uses a lightweight run view without steps. Query the current generation
        // directly instead of treating an empty view as evidence that no effect is fenced.
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT s.subject FROM mission_runs m JOIN step_runs s ON s.run_id=m.id
             AND s.generation_id=m.current_generation_id WHERE m.id=?1").map_err(internal)?;
        for step in statement.query_map([run.strip_prefix("mission-run/").unwrap_or(run)],
            |row| row.get::<_, String>(0)).map_err(internal)? {
            if step_has_unresolved_effect_tx(&connection, &step.map_err(internal)?).map_err(internal)? { return Ok(true); }
        }
        Ok(false)
    }

    pub fn admit_message_effect(&self, owner: &EffectOwner, caller: &str, payload_sender: &str,
        payload_digest: &str, target: &str) -> Result<EffectAdmission, FractalError> {
        self.admit_effect(owner, caller, payload_sender, payload_digest, EffectTarget {
            provider: "native-message", class: EffectClass::NativeCoordinated, target,
        })
    }

    /// All adapters pass through daemon-owned capability validation before admission.
    pub fn admit_effect(&self, owner: &EffectOwner, caller: &str, payload_sender: &str,
        payload_digest: &str, target: EffectTarget<'_>) -> Result<EffectAdmission, FractalError> {
        let EffectTarget { provider, class, target } = target;
        fractal_schema::effects::validate_provider(provider, class, target)
            .map_err(|error| FractalError::new(error.code, error.message))?;
        let key = effect_key(owner)?;
        let operation_id = operation_id_for_key(&key);
        let id = format!("effect/{operation_id}");
        self.with_run_fence(&owner.step, |tx| {
            lease_tx(tx, owner, caller)?;
            let original = match intended_tx(tx, &id)? {
                Some(record) => Some(record),
                None => latest_tx(tx, &id)?.map(|(_, record)| record),
            };
            if let Some(original) = original {
                let resolved = latest_tx(tx, &id)?.filter(|(kind, record)|
                    kind == KIND_CONFIRMED && record.resolution.as_deref() == Some("human"));
                if let Some((_, resolved)) = resolved {
                    if original.payload_digest != payload_digest || original.target != target
                        || original.provider != provider || original.class != class
                        || original.lease_owner != payload_sender {
                        return Ok(EffectAdmission::Blocked(FractalError::new("idempotency-mismatch",
                            "the resolved effect retains its original immutable intent")));
                    }
                    return Ok(EffectAdmission::Recover(resolved));
                }
                if operation_conflicted_tx(tx, &operation_id)? {
                    park_tx(tx, self.origin(), &original, &owner.step, "idempotency-conflict", caller)?;
                    return Ok(EffectAdmission::Blocked(FractalError::new("idempotency-conflict",
                        "multiple writers admitted the same effect operation")));
                }
                // The immutable operation digest uses the original binding, not its replacement.
                let input = intended_input(&original, &key)?;
                if let Err(error) = replicated_operation_outcome_tx(tx, &input) {
                    park_tx(tx, self.origin(), &original, &owner.step, error.code, caller)?;
                    return Ok(EffectAdmission::Blocked(error));
                }
                if original.bound_step_run != owner.step {
                    return Err(FractalError::new("effect-work-binding-mismatch",
                        "recovery requires the current live attempt of the effect's original step-run"));
                }
                if original.payload_digest != payload_digest || original.target != target
                    || original.provider != provider || original.class != class
                    || original.lease_owner != payload_sender
                {
                    park_tx(tx, self.origin(), &original, &owner.step, "idempotency-mismatch", caller)?;
                    return Ok(EffectAdmission::Blocked(FractalError::new("idempotency-mismatch",
                        "the effect key already identifies a different message")));
                }
                let (kind, record) = latest_tx(tx, &id)?.ok_or_else(|| internal("the effect disappeared"))?;
                if kind == KIND_UNRESOLVED {
                    return Ok(EffectAdmission::Blocked(FractalError::new("effect-unresolved",
                        "the effect requires resolution before work can continue")));
                }
                return Ok(EffectAdmission::Recover(record));
            }
            if caller != payload_sender {
                return Err(FractalError::new("effect-caller-refused",
                    "first origination requires the verified worker to be the message sender"));
            }
            let record = EffectRecord {
                effect_id: id, operation_id, provider: provider.into(), target: target.into(),
                payload_digest: payload_digest.into(), class,
                bound_step_run: owner.step.clone(), bound_attempt: owner.attempt,
                lease_owner: caller.into(), lease_incarnation: owner.incarnation.clone(),
                provider_ref: None, result_digest: None, error: None, recovering_actor: None,
                resolution: None, outcome: None, evidence: Vec::new(),
            };
            if !may_originate_tx(tx, self.origin(), &owner.step, caller, owner.attempt, &owner.incarnation)? {
                park_tx(tx, self.origin(), &record, &owner.step,
                    "the creation anchor or current work claim is unavailable, ambiguous, or belongs to another host", caller)?;
                return Ok(EffectAdmission::Blocked(FractalError::new("effect-unresolved",
                    "only the creation-anchor host with a local live work claim may originate; nothing was sent")));
            }
            let input = intended_input(&record, &key)?;
            // Do not interpret a conflicted/checkpointed operation as an absent effect.
            match replicated_operation_outcome_tx(tx, &input) {
                Ok(Some(claim)) => return Ok(EffectAdmission::Recover(
                    serde_json::from_value(claim.body["fields"].clone()).map_err(internal)?)),
                Ok(None) => {}
                Err(error) => {
                    park_tx(tx, self.origin(), &record, &owner.step, error.code, caller)?;
                    return Ok(EffectAdmission::Blocked(error));
                }
            }
            let (operation, digest) = claim_operation(&input)?
                .ok_or_else(|| internal("the effect operation is missing"))?;
            let mut body = json!({"fields":record,"evidence":[]});
            smallclaims::store::idempotency::attach(&mut body, &key).map_err(internal)?;
            body["_operation"] = json!({"id":operation,"request_digest":digest});
            let claim = append_claim_tx(tx, self.origin(), &record.effect_id, KIND_INTENDED,
                Some(caller), &body, &[], None).map_err(internal)?;
            register_operation_tx(tx, &claim).map_err(internal)?;
            Ok(EffectAdmission::Originate(record))
        })
    }

    /// A second admission always recovers; only the first intended writer can enter send.
    pub fn effect_in_flight(&self, record: &EffectRecord) -> Result<(), FractalError> {
        self.with_run_fence(&record.bound_step_run, |tx| {
            let Some((kind, current)) = latest_tx(tx, &record.effect_id)? else {
                return Err(FractalError::new("effect-missing", "the intended effect is missing"));
            };
            if kind != KIND_INTENDED {
                return Err(FractalError::new("effect-not-intended", "the effect has already entered execution or resolution"));
            }
            if operation_conflicted_tx(tx, &record.operation_id)?
            {
                park_tx(tx, self.origin(), record, &record.bound_step_run,
                    "idempotency-conflict", &record.lease_owner)?;
                return Ok(Err(FractalError::new("idempotency-conflict",
                    "the effect operation has conflicting requests")));
            }
            let original = intended_tx(tx, &record.effect_id)?.ok_or_else(|| internal("the intended effect is missing"))?;
            if &original != record {
                return Err(FractalError::new("idempotency-mismatch", "the admitted effect changed"));
            }
            // Recheck the originating lease at the final durable boundary before send.
            let mission: String = tx.query_row("SELECT m.mission_id FROM step_runs s JOIN mission_runs m ON m.id=s.run_id WHERE s.subject=?1",
                [&record.bound_step_run], |row| row.get(0)).map_err(internal)?;
            lease_tx(tx, &EffectOwner { mission, key: String::new(), step: record.bound_step_run.clone(),
                attempt: record.bound_attempt, incarnation: record.lease_incarnation.clone() }, &record.lease_owner)?;
            if !may_originate_tx(tx, self.origin(), &record.bound_step_run, &record.lease_owner,
                record.bound_attempt, &record.lease_incarnation)?
            {
                return Err(FractalError::new("effect-origin-unavailable",
                    "only the creation-anchor host with a local work claim may send"));
            }
            append_effect_tx(tx, self.origin(), KIND_IN_FLIGHT, &current)?;
            Ok(Ok(()))
        })?
    }

    pub fn confirm_message_effect(&self, record: &EffectRecord, message: &ClaimRecord, recovering_actor: &str) -> Result<(), FractalError> {
        self.with_run_fence(&record.bound_step_run, |tx| {
            let Some((kind, mut current)) = latest_tx(tx, &record.effect_id)? else {
                return Err(FractalError::new("effect-missing", "the intended effect is missing"));
            };
            if kind == KIND_UNRESOLVED {
                return Err(FractalError::new("effect-unresolved", "an unresolved effect cannot be implicitly confirmed"));
            }
            if current.resolution.as_deref() == Some("human") { return Ok(()); }
            let message_operation = operation_id_for_key(&format!("native-effect-message/{}", current.operation_id));
            if message.kind != "message.sent" || message.body["fields"]["to"] != current.target
                || message.body["fields"]["from"] != current.lease_owner
                || smallclaims::store::operation_parts(&message.body)
                    .is_none_or(|(operation, _)| operation != message_operation)
            {
                return Err(FractalError::new("effect-evidence-mismatch", "the evidence is not the effect's native message"));
            }
            if kind == KIND_CONFIRMED {
                if current.provider_ref.as_deref() != Some(&message.subject) {
                    return Err(FractalError::new("effect-evidence-mismatch", "the effect already confirms another message"));
                }
                if current.recovering_actor.as_deref() == Some(recovering_actor) { return Ok(()); }
            } else {
                current.provider_ref = Some(message.subject.clone());
                current.evidence.push(message.id.clone());
            }
            current.recovering_actor = Some(recovering_actor.into());
            append_effect_tx(tx, self.origin(), KIND_CONFIRMED, &current)
        })
    }

    pub fn unresolved_message_effect(&self, record: &EffectRecord, reason: &str, recovering_actor: &str) -> Result<(), FractalError> {
        self.with_run_fence(&record.bound_step_run, |tx| {
            let Some((kind, current)) = latest_tx(tx, &record.effect_id)? else {
                return Err(FractalError::new("effect-missing", "the intended effect is missing"));
            };
            if kind == KIND_CONFIRMED || kind == KIND_UNRESOLVED { return Ok(()); }
            park_tx(tx, self.origin(), &current, &record.bound_step_run, reason, recovering_actor)
        })
    }

    pub fn resolve_effect_by_key(&self, owner: &EffectOwner) -> Result<Option<EffectRecord>, FractalError> {
        let id = format!("effect/{}", operation_id_for_key(&effect_key(owner)?));
        let connection = self.readers.get();
        if let Some(original) = intended_tx(&connection, &id)? {
            let input = intended_input(&original, &effect_key(owner)?)?;
            // This public reader must not hide replicated operation conflicts.
            let operation = claim_operation(&input)?;
            if let Some((operation, digest)) = operation
                && let Some((stored, _, state)) = operation_tx(&connection, &operation).map_err(internal)?
                && (state == "conflict" || stored != digest)
            {
                return Err(FractalError::new("idempotency-conflict", "the effect operation has conflicting requests"));
            }
        }
        Ok(latest_tx(&connection, &id)?.map(|(_, record)| record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> EffectOwner {
        EffectOwner { mission: "mission/example".into(), key: "announce".into(), step: "step-run/example/build".into(),
            attempt: 1, incarnation: "incarnation-example".into() }
    }

    #[test]
    fn identity_is_unambiguous_and_independent_of_replacement_binding() {
        let first = owner();
        let mut replacement = first.clone();
        replacement.step = "step-run/example/replacement".into();
        replacement.attempt = 2;
        replacement.incarnation = "incarnation-replacement".into();
        assert_eq!(effect_key(&first).unwrap(), effect_key(&replacement).unwrap());
        assert_eq!(native_message_key(&first).unwrap(), native_message_key(&replacement).unwrap());
        assert_ne!(effect_key(&first).unwrap(), native_message_key(&first).unwrap());
        let mut other = first.clone();
        other.mission.push('/');
        other.key.insert(0, '/');
        assert_ne!(effect_key(&first).unwrap(), effect_key(&other).unwrap());
    }

    #[test]
    fn stale_lease_cannot_create_an_effect() {
        let store = Store::open_memory("example").unwrap();
        let owner = owner();
        let result = store.admit_message_effect(&owner, "agent/example/worker", "agent/example/worker", "digest-example", "agent/example/recipient");
        assert!(matches!(result, Err(error) if error.code == "stale-work-lease"));
        assert!(store.resolve_effect_by_key(&owner).unwrap().is_none());
    }

    #[test]
    fn late_phase_observations_never_clear_an_unresolved_fence() {
        let store = Store::open_memory("example").unwrap();
        let owner = owner();
        let operation_id = operation_id_for_key(&effect_key(&owner).unwrap());
        let record = EffectRecord {
            effect_id: format!("effect/{operation_id}"), operation_id,
            provider: "native-message".into(), target: "agent/example/recipient".into(),
            payload_digest: "digest-example".into(), class: EffectClass::NativeCoordinated,
            bound_step_run: owner.step.clone(), bound_attempt: owner.attempt,
            lease_owner: "agent/example/worker".into(), lease_incarnation: owner.incarnation.clone(),
            provider_ref: Some("message/example".into()), result_digest: None,
            error: Some("delivery remains unresolved".into()), recovering_actor: Some("agent/example/recovery".into()),
            resolution: None, outcome: None,
            evidence: vec!["claim/example-message".into()],
        };
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for kind in [KIND_UNRESOLVED, KIND_CONFIRMED, KIND_IN_FLIGHT, KIND_INTENDED] {
            append_effect_tx(&tx, store.origin(), kind, &record).unwrap();
        }
        assert_eq!(latest_tx(&tx, &record.effect_id).unwrap().unwrap().0, KIND_UNRESOLVED);
        tx.commit().unwrap();
        assert!(store.operation_claim(&native_message_key(&owner).unwrap()).unwrap().is_none());
    }
}
