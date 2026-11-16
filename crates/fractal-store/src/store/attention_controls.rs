//! Each person's own controls over their attention rows, one row per (person, attention id):
//! when the person first saw it, and until when they put it off. Seen keeps the smallest
//! canonical claim key and snooze the largest, so append, replication and replay agree whatever
//! order the claims arrive in. Claims are the authority; this table is rebuilt from them.
use super::*;
use fractal_schema::attention_controls as schema;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h436a70aa582a owner=fractal_store::store::attention_controls TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS person_attention_controls_v2 (
    person TEXT NOT NULL, attention_id TEXT NOT NULL,
    seen_at_ms INTEGER, seen_key BLOB,
    snoozed_until_ms INTEGER, snoozed_indefinitely INTEGER NOT NULL DEFAULT 0, snooze_key BLOB,
    PRIMARY KEY(person,attention_id)
) WITHOUT ROWID;
"#;

/// One row's controls: when its person first saw it, and the newest snooze's end (none when the
/// newest snooze ended it). Times are Unix milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub seen_at_ms: Option<u128>,
    pub snoozed_until_ms: Option<u128>,
    pub snoozed_indefinitely: bool,
}

/// Projects one appended or replicated control claim. A claim that does not beat the stored
/// one leaves the row untouched.
pub fn project(transaction: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    if repaired(transaction, claim)? {
        return Ok(());
    }
    // A previous binary must replay these claims on downgrade instead of trusting its old table.
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h2bacb7039cd7 owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='person_attention_controls_v1'", [])?;
    let fields = schema_fields_for_body(&claim.kind, &claim.body)?;
    let attention_id = fields
        .get("attention_id")
        .and_then(Value::as_str)
        .context("validated attention control id")?;
    let key = canonical::sortable_key(&canonical::claim_key(transaction, &claim.id)?);
    let at = i64::try_from(claim.accepted_at_unix_ms).unwrap_or(i64::MAX);
    if claim.kind == schema::SEEN_KIND {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h4e413f1ac6ad owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO person_attention_controls_v2(person,attention_id,seen_at_ms,seen_key) VALUES(?1,?2,?3,?4)
             ON CONFLICT(person,attention_id) DO UPDATE SET seen_at_ms=excluded.seen_at_ms,seen_key=excluded.seen_key
             WHERE person_attention_controls_v2.seen_key IS NULL OR excluded.seen_key<person_attention_controls_v2.seen_key",
            params![claim.subject, attention_id, at, key],
        )?;
    } else {
        let until = fields.get("until_unix_ms").and_then(Value::as_u64).map(|until| until as i64);
        let indefinite = fields.get("indefinite").and_then(Value::as_bool).unwrap_or(false);
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h1d7863e514e3 owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO person_attention_controls_v2(person,attention_id,snoozed_until_ms,snoozed_indefinitely,snooze_key) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(person,attention_id) DO UPDATE SET snoozed_until_ms=excluded.snoozed_until_ms,snoozed_indefinitely=excluded.snoozed_indefinitely,snooze_key=excluded.snooze_key
             WHERE person_attention_controls_v2.snooze_key IS NULL OR excluded.snooze_key>person_attention_controls_v2.snooze_key",
            params![claim.subject, attention_id, until, indefinite, key],
        )?;
    }
    Ok(())
}

fn repaired(transaction: &Transaction<'_>, claim: &ClaimRecord) -> Result<bool> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1) OR EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')",
        [&claim.id], |row| row.get(0),
    )?)
}

/// Rebuild this projection namespace from all authoritative controls, including legacy claims
/// without an indefinite flag. The previous table is not authority and remains untouched.
pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    let ready: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='person_attention_controls_v2_ready')
           AND NOT EXISTS(SELECT 1 FROM meta WHERE key='person_attention_controls_v1')",
        [], |row| row.get(0),
    )?;
    if !ready {
        rebuild(transaction)?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h181ff947c149 owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('person_attention_controls_v2_ready','1')", [])?;
    }
    Ok(())
}

/// Replays every unrepaired control into an empty table. Each merge is order-free, so claim
/// order does not matter; a repair that retracts a winner also rebuilds through here.
pub fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h984819dfe56b owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='person_attention_controls_v1'", [])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h09c12b28e061 owner=fractal_store::store::attention_controls TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM person_attention_controls_v2", [])?;
    let mut statement = transaction.prepare(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE kind IN ('person.attention-seen','person.attention-snoozed')",
    )?;
    let claims = statement.query_map([], claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for claim in &claims {
        project(transaction, claim)?;
    }
    Ok(())
}

/// The controls of each `(person, attention id)` asked for that has any, read in the caller's
/// snapshot.
pub fn controls(connection: &Connection, rows: &[(&str, &str)]) -> Result<HashMap<(String, String), Controls>> {
    if rows.is_empty() {
        return Ok(HashMap::new());
    }
    let mut statement = connection.prepare_cached(
        "SELECT c.person,c.attention_id,c.seen_at_ms,c.snoozed_until_ms,c.snoozed_indefinitely FROM json_each(?1) wanted
         JOIN person_attention_controls_v2 c
           ON c.person=json_extract(wanted.value,'$[0]') AND c.attention_id=json_extract(wanted.value,'$[1]')",
    )?;
    let found = statement
        .query_map([serde_json::to_string(rows)?], |row| {
            let time = |index| row.get::<_, Option<i64>>(index).map(|at| at.and_then(|at| u128::try_from(at).ok()));
            Ok(((row.get(0)?, row.get(1)?), Controls { seen_at_ms: time(2)?, snoozed_until_ms: time(3)?, snoozed_indefinitely: row.get(4)? }))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(found)
}

/// Sets each client attention row's `seen_at` and `snoozed_until` from its person's own controls
/// (null when none), read on `connection`. Controls are an overlay on the row, not part of its
/// own state, so every reader of attention rows applies them the same way when it serves them.
pub fn overlay(connection: &Connection, rows: &mut [Value]) -> Result<()> {
    let keys = rows
        .iter()
        .filter_map(|row| Some((row["person_id"].as_str()?, row["id"].as_str()?)))
        .collect::<Vec<_>>();
    let found = controls(connection, &keys)?;
    let time = |at: Option<u128>| at.map_or(Value::Null, |at| json!(crate::client_values::client_timestamp(at)));
    for row in rows {
        let control = match (row["person_id"].as_str(), row["id"].as_str()) {
            (Some(person), Some(id)) => found.get(&(person.to_owned(), id.to_owned())).copied().unwrap_or_default(),
            _ => Controls::default(),
        };
        row["seen_at"] = time(control.seen_at_ms);
        row["snoozed_until"] = time(control.snoozed_until_ms);
        // Absence means false and preserves the legacy read shape for timed/cleared rows.
        if control.snoozed_indefinitely {
            row["snoozed_indefinitely"] = json!(true);
        } else {
            row.as_object_mut().expect("attention overlay sets object fields").remove("snoozed_indefinitely");
        }
    }
    Ok(())
}

/// A control the person asked for: seen, or a snooze until a time or ended (`None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Seen,
    Snooze(Option<u128>),
    SnoozeIndefinitely,
}

impl Store {
    /// The controls of each `(person, attention id)` asked for that has any.
    pub fn attention_controls(&self, rows: &[(&str, &str)]) -> Result<HashMap<(String, String), Controls>> {
        controls(&self.readers.get(), rows)
    }

    /// Overlays each row's person's own controls; see [`overlay`].
    pub fn overlay_attention_controls(&self, rows: &mut [Value]) -> Result<()> {
        overlay(&self.readers.get(), rows)
    }

    /// Record `person`'s control over `attention_id`, from `source` (the pairing grant or
    /// `host/MEMBER` the daemon names). Marking a row seen again completes without a claim; a
    /// snooze always appends, so the newest one wins on every member. Returns the claim, if
    /// one was appended, and the row's controls after it.
    pub fn record_attention_control(
        &self,
        person: &str,
        attention_id: &str,
        control: Control,
        source: &str,
        idempotency_key: String,
    ) -> Result<(Option<ClaimRecord>, Controls), FractalError> {
        let stored = || {
            controls(&self.readers.get(), &[(person, attention_id)])
                .map(|found| found.into_values().next().unwrap_or_default())
                .map_err(internal)
        };
        let (kind, mut fields) = match control {
            Control::Seen if stored()?.seen_at_ms.is_some() => return Ok((None, stored()?)),
            Control::Seen => (schema::SEEN_KIND, BTreeMap::new()),
            Control::Snooze(until) => (schema::SNOOZED_KIND, BTreeMap::from([("until_unix_ms".to_owned(), json!(until))])),
            Control::SnoozeIndefinitely => (schema::SNOOZED_KIND, BTreeMap::from([
                ("until_unix_ms".to_owned(), Value::Null), ("indefinite".to_owned(), json!(true)),
            ])),
        };
        fields.insert("attention_id".into(), json!(attention_id));
        fields.insert("source".into(), json!(source));
        let claim = self.append_claim(&ClaimInput {
            subject: person.into(),
            kind: kind.into(),
            actor: Some(person.into()),
            fields,
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(idempotency_key),
        })?;
        Ok((Some(claim), stored()?))
    }
}

#[cfg(test)]
#[path = "attention_controls_tests.rs"]
mod tests;
