//! Host-local refresh intake fencing. Only writer transactions may change this boundary.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Seal {
    pub operation: String,
    pub incumbent: String,
    pub desired_token: String,
    pub activity_epoch: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Intake {
    epoch: u64,
    pending: Option<Seal>,
    sealed: bool,
    grants: BTreeSet<(String, String)>,
}

fn load(tx: &Connection, seat: &str) -> Result<Intake> {
    let value: Option<String> = tx
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [format!("refresh-intake/{seat}")],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map(|v| v.unwrap_or_default())
        .map_err(Into::into)
}

fn save(tx: &Connection, seat: &str, state: &Intake) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-hf3700210f0d7 owner=fractal_store::store::refresh_intake TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("refresh-intake/{seat}"), serde_json::to_string(state)?])?;
    Ok(())
}

fn current(tx: &Connection, seat: &str, seal: &Seal) -> Result<bool> {
    Ok(work_incarnation_live(tx, seat, &seal.incumbent)?
        && latest_actual(tx, seat)?.is_some_and(|value| {
            value["incarnation_id"] == seal.incumbent && value["status"] == "running"
        })
        && current_desired_row(tx, seat)?.is_some_and(|row| row.claim_id == seal.desired_token))
}

/// Called inside the work action's writer transaction, never from a preceding reader.
pub fn admit_claim_tx(tx: &Connection, seat: &str) -> Result<(), FractalError> {
    let mut state = load(tx, seat).map_err(internal)?;
    if state.sealed {
        return Err(FractalError::new(
            "refresh-intake-sealed",
            "refresh holds new work and mutation intake",
        ));
    }
    state.epoch = state
        .epoch
        .checked_add(1)
        .ok_or_else(|| internal("refresh epoch exhausted"))?;
    save(tx, seat, &state).map_err(internal)
}

/// Acquire the seal inside an existing writer transaction.
pub fn seal_tx(tx: &Connection, seat: &str, expected: &Seal) -> Result<bool> {
    let mut state = load(tx, seat)?;
    let held: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM step_runs WHERE lease_owner=?1)",
        [seat],
        |row| row.get(0),
    )?;
    if state.pending.as_ref() != Some(expected)
        || state.epoch != expected.activity_epoch
        || !state.grants.is_empty()
        || held
        || !current(tx, seat, expected)?
    {
        return Ok(false);
    }
    state.sealed = true;
    save(tx, seat, &state)?;
    Ok(true)
}

/// Admit a mutation inside its existing writer transaction.
pub fn admit_mutation_tx(
    tx: &Connection,
    seat: &str,
    incarnation: &str,
    grant: &str,
) -> Result<bool> {
    let mut state = load(tx, seat)?;
    if state.sealed || !work_incarnation_live(tx, seat, incarnation)? {
        return Ok(false);
    }
    if latest_actual(tx, seat)?
        .is_none_or(|value| value["incarnation_id"] != incarnation || value["status"] != "running")
    {
        return Ok(false);
    }
    if state.grants.insert((incarnation.into(), grant.into())) {
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("refresh epoch exhausted"))?;
        save(tx, seat, &state)?;
    }
    Ok(true)
}

impl Store {
    pub fn refresh_activity_epoch(&self, seat: &str) -> Result<u64> {
        Ok(load(&self.readers.get(), seat)?.epoch)
    }

    /// Register the exact operation being prepared, without closing intake or resetting age.
    pub fn prepare_refresh_intake(&self, seat: &str, expected: &Seal) -> Result<bool> {
        self.connection
            .batched(|tx| -> Result<bool> {
                let mut state = load(tx, seat)?;
                if state.sealed
                    || state.epoch != expected.activity_epoch
                    || !current(tx, seat, expected)?
                {
                    return Ok(false);
                }
                state.pending = Some(expected.clone());
                save(tx, seat, &state)?;
                Ok(true)
            })
            .map_err(anyhow::Error::msg)?
    }

    pub fn seal_refresh_intake(&self, seat: &str, expected: &Seal) -> Result<bool> {
        self.connection
            .batched(|tx| seal_tx(tx, seat, expected))
            .map_err(anyhow::Error::msg)?
    }

    /// Supersession, convergence and failure release only this operation's boundary.
    pub fn unseal_refresh_intake(&self, seat: &str, operation: &str) -> Result<bool> {
        self.connection
            .batched(|tx| -> Result<bool> {
                let mut state = load(tx, seat)?;
                if state
                    .pending
                    .as_ref()
                    .is_none_or(|seal| seal.operation != operation)
                {
                    return Ok(false);
                }
                state.pending = None;
                state.sealed = false;
                save(tx, seat, &state)?;
                Ok(true)
            })
            .map_err(anyhow::Error::msg)?
    }

    /// A grant is explicitly completed, never inferred closed from lease expiry.
    pub fn admit_refresh_mutation(
        &self,
        seat: &str,
        incarnation: &str,
        grant: &str,
    ) -> Result<bool> {
        self.connection
            .batched(|tx| admit_mutation_tx(tx, seat, incarnation, grant))
            .map_err(anyhow::Error::msg)?
    }

    pub fn complete_refresh_mutation(
        &self,
        seat: &str,
        incarnation: &str,
        grant: &str,
    ) -> Result<()> {
        self.connection
            .batched(|tx| -> Result<()> {
                let mut state = load(tx, seat)?;
                state.grants.remove(&(incarnation.into(), grant.into()));
                save(tx, seat, &state)
            })
            .map_err(anyhow::Error::msg)?
    }
}
