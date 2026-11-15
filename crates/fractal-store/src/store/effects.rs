//! Durable native-message admission. Absence on a peer is never permission to send.
use super::*;
use fractal_kernel::model::EffectOwner;
pub use fractal_schema::effects::{EffectClass, EffectRecord};
use fractal_schema::effects::{KIND_CONFIRMED, KIND_IN_FLIGHT, KIND_INTENDED, KIND_UNRESOLVED};

#[derive(Debug)]
pub enum EffectAdmission {
    Originate(EffectRecord),
    Recover(EffectRecord),
    /// A successful transaction value: parking must survive the refusal.
    Blocked(FractalError),
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

fn latest_tx(tx: &Connection, id: &str) -> Result<Option<(String, EffectRecord)>, FractalError> {
    // A late observation cannot erase an unresolved fence or regress committed evidence.
    let row: Option<(String, String)> = tx.query_row(
        /* sqlite-large-value: allow-for-now lv-h115b9b22ffe0 owner=fractal_store::store::effects TODO=trace-statement-and-value-representation */
        &canonical_sql("SELECT kind,id FROM claims WHERE subject=?1 AND kind IN (?2,?3,?4,?5)
         ORDER BY CASE kind WHEN ?5 THEN 4 WHEN ?4 THEN 3 WHEN ?3 THEN 2 ELSE 1 END DESC,
                  CANONICAL_DESC(claims) LIMIT 1"),
        params![id, KIND_INTENDED, KIND_IN_FLIGHT, KIND_CONFIRMED, KIND_UNRESOLVED],
        |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(internal)?;
    row.map(|(kind, id)| {
        let claim = claim_by_id_tx(tx, &id).map_err(internal)?.ok_or_else(|| internal("the effect claim is missing"))?;
        Ok((kind, serde_json::from_value(claim.body["fields"].clone()).map_err(internal)?))
    }).transpose()
}

/// Unresolved is a monotonic phase, so no later work or effect observation can clear it.
pub(super) fn step_has_unresolved_effect_tx(connection: &Connection, step: &str) -> rusqlite::Result<bool> {
    connection.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='effect.unresolved'
         AND json_extract(body,'$.fields.bound_step_run')=?1)",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-h51c18f03a1dc owner=fractal_store::store::effects TODO=trace-statement-and-value-representation */ [step], |row| row.get(0))
}

fn append_effect_tx(tx: &Transaction<'_>, origin: &str, kind: &str, record: &EffectRecord) -> Result<(), FractalError> {
    let predecessors = latest_claim_id_tx(tx, &record.effect_id).map_err(internal)?.into_iter().collect::<Vec<_>>();
    append_claim_tx(tx, origin, &record.effect_id, kind,
        record.recovering_actor.as_deref().or(Some(&record.lease_owner)),
        &json!({"fields":record,"evidence":record.evidence}), &predecessors, None).map_err(internal)?;
    Ok(())
}

fn park_tx(tx: &Transaction<'_>, origin: &str, record: &EffectRecord, step: &str, reason: &str, recovering_actor: &str) -> Result<(), FractalError> {
    let mut unresolved = record.clone();
    unresolved.error = Some(reason.into());
    unresolved.recovering_actor = Some(recovering_actor.into());
    append_effect_tx(tx, origin, KIND_UNRESOLVED, &unresolved)?;
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

impl Store {
    pub fn step_has_unresolved_effect(&self, step: &str) -> Result<bool, FractalError> {
        step_has_unresolved_effect_tx(&self.readers.get(), step).map_err(internal)
    }

    pub fn mission_run_has_unresolved_effect(&self, run: &str) -> Result<bool, FractalError> {
        // Cleanup uses a lightweight run view without steps. Query the current generation
        // directly instead of treating an empty view as evidence that no effect is fenced.
        self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM mission_runs m
             JOIN step_runs s ON s.run_id=m.id AND s.generation_id=m.current_generation_id
             JOIN claims c ON c.kind='effect.unresolved'
                 AND json_extract(c.body,'$.fields.bound_step_run')=s.subject
             WHERE m.id=?1)",
            [run.strip_prefix("mission-run/").unwrap_or(run)], |row| row.get(0),
        ).map_err(internal)
    }

    pub fn admit_message_effect(&self, owner: &EffectOwner, caller: &str, payload_sender: &str,
        payload_digest: &str, target: &str) -> Result<EffectAdmission, FractalError> {
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
                if original.bound_step_run != owner.step {
                    return Err(FractalError::new("effect-work-binding-mismatch",
                        "recovery requires the current live attempt of the effect's original step-run"));
                }
                // The immutable operation digest uses the original binding, not its replacement.
                let input = intended_input(&original, &key)?;
                if let Err(error) = replicated_operation_outcome_tx(tx, &input) {
                    park_tx(tx, self.origin(), &original, &owner.step, error.code, caller)?;
                    return Ok(EffectAdmission::Blocked(error));
                }
                if original.payload_digest != payload_digest || original.target != target
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
                effect_id: id, operation_id, provider: "native-message".into(), target: target.into(),
                payload_digest: payload_digest.into(), class: EffectClass::NativeCoordinated,
                bound_step_run: owner.step.clone(), bound_attempt: owner.attempt,
                lease_owner: caller.into(), lease_incarnation: owner.incarnation.clone(),
                provider_ref: None, result_digest: None, error: None, recovering_actor: None, evidence: Vec::new(),
            };
            if !may_originate_tx(tx, self.origin(), &owner.step, caller, owner.attempt, &owner.incarnation)? {
                park_tx(tx, self.origin(), &record, &owner.step,
                    "the creation anchor or current work claim is unavailable, ambiguous, or belongs to another host", caller)?;
                return Ok(EffectAdmission::Blocked(FractalError::new("effect-unresolved",
                    "only the creation-anchor host with a local live work claim may originate; nothing was sent")));
            }
            let input = intended_input(&record, &key)?;
            // Do not interpret a conflicted/checkpointed operation as an absent effect.
            if let Some(claim) = replicated_operation_outcome_tx(tx, &input)? {
                return Ok(EffectAdmission::Recover(serde_json::from_value(claim.body["fields"].clone()).map_err(internal)?));
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
            let original = intended_tx(tx, &record.effect_id)?.ok_or_else(|| internal("the intended effect is missing"))?;
            if &original != record {
                return Err(FractalError::new("idempotency-mismatch", "the admitted effect changed"));
            }
            if let Some((_, _, state)) = operation_tx(tx, &record.operation_id).map_err(internal)?
                && state == "conflict"
            {
                return Err(FractalError::new("idempotency-conflict", "the effect operation has conflicting requests"));
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
            append_effect_tx(tx, self.origin(), KIND_IN_FLIGHT, &current)
        })
    }

    pub fn confirm_message_effect(&self, record: &EffectRecord, message: &ClaimRecord, recovering_actor: &str) -> Result<(), FractalError> {
        self.with_run_fence(&record.bound_step_run, |tx| {
            let Some((kind, mut current)) = latest_tx(tx, &record.effect_id)? else {
                return Err(FractalError::new("effect-missing", "the intended effect is missing"));
            };
            if kind == KIND_UNRESOLVED {
                return Err(FractalError::new("effect-unresolved", "an unresolved effect cannot be implicitly confirmed"));
            }
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
