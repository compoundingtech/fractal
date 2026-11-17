//! Account quota is a projection of configured omp-usage observations.
//! Each provider window retains its own clock, coverage and provenance. A newer gap replaces
//! an older number; neither cached evidence nor another window can freshen that number.
use super::*;
use serde::Deserialize;
use smallclaims::store::canonical;

pub const ACCOUNT_READING_WINDOW_MS: u64 = 3_600_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LimitWindow {
    pub window: String,
    pub used_percent: Option<f64>,
    pub resets_at_unix_ms: Option<u64>,
    pub measured_at_unix_ms: u64,
    pub coverage: String,
    pub source: String,
    pub host: String,
}

impl LimitWindow {
    pub fn available(&self, now: u128, fresh_ms: u64) -> bool {
        self.source == "omp-usage" && self.coverage == "reported"
            && self.used_percent.is_some_and(|used| used.is_finite() && (0.0..=100.0).contains(&used))
            && self.resets_at_unix_ms.is_some_and(|reset| u128::from(reset) > now)
            && u128::from(self.measured_at_unix_ms) <= now.saturating_add(60_000)
            && now.saturating_sub(u128::from(self.measured_at_unix_ms)) <= u128::from(fresh_ms.min(ACCOUNT_READING_WINDOW_MS))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AccountLimit {
    pub account: String,
    #[serde(default)]
    pub account_ref: Option<String>,
    #[serde(default)]
    pub identified: bool,
    #[serde(skip)]
    pub identity: Option<String>,
    pub driver: String,
    pub plan: Option<String>,
    pub measured_at_unix_ms: u64,
    pub measured_by: String,
    pub host: String,
    pub seats: Vec<String>,
    pub windows: Vec<LimitWindow>,
}

impl AccountLimit {
    /// The weekly evidence used by admission and enforcement, never another window's clock.
    pub fn weekly_window(&self, now: u128, fresh_ms: u64) -> Option<&LimitWindow> {
        self.windows.iter().filter(|window| matches!(window.window.as_str(), "weekly" | "7d"))
            .filter(|window| window.available(now, fresh_ms))
            .max_by_key(|window| window.measured_at_unix_ms)
    }
}

/// Decode one normalized projection row. Only the omp-usage normalization writes this shape.
pub fn reading(origin: &str, body: &Value) -> Option<(AccountLimit, String)> {
    let fields = body.get("fields")?;
    let account = fields["account"].as_str()?.to_owned();
    Some((AccountLimit {
        account: account.clone(),
        account_ref: fields["account_ref"].as_str().map(str::to_owned),
        identified: true,
        identity: None,
        driver: fields["driver"].as_str()?.to_owned(),
        plan: None,
        measured_at_unix_ms: fields["measured_at_unix_ms"].as_u64()?,
        measured_by: String::new(),
        host: origin.into(),
        seats: Vec::new(),
        windows: serde_json::from_value(fields["observed_windows"].clone()).ok()?,
    }, account))
}

fn observation_readings(body: &Value) -> Vec<Value> {
    let Some(readings) = body.pointer("/fields/readings").and_then(Value::as_array) else { return Vec::new(); };
    readings.iter().filter_map(|fields| {
        let provider = fields["provider"].as_str()?;
        let driver = match provider { "anthropic" => "claude", "openai-codex" => "codex", other => other };
        let timestamp = |name: &str| fields[name].as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .and_then(|at| u64::try_from(at.timestamp_millis()).ok());
        let measured = timestamp("measured_at")?;
        Some(json!({"fields": {
            "driver":driver, "account":fields["account"], "account_ref":fields["account_ref"],
            "measured_at_unix_ms":measured,
            "observed_windows":[{"window":fields["window"], "used_percent":fields["used_percent"],
                "resets_at_unix_ms":timestamp("resets_at"), "measured_at_unix_ms":measured,
                "coverage":fields["coverage"], "source":"omp-usage", "host":fields["host"]}]
        }}))
    }).collect()
}

// Rebuildable from durable usage.limits.observed claims. A distinct namespace prevents old
// harness-derived rows or a previous binary's cursor from becoming quota evidence.
const LIMITS_CURSOR: &str = "observed_account_limits_v4_through_index";
pub const LIMITS_READY: &str = "observed_account_limits_v4_ready";
pub const LIMITS_CATCH_UP_PAGE: usize = 200;
const LIMITS_APPEND_PAGE: usize = 64;
const LIMITS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observed_account_limits_v4 (
    subject TEXT NOT NULL,
    driver TEXT NOT NULL,
    account TEXT NOT NULL,
    window TEXT NOT NULL,
    measured_at_unix_ms INTEGER NOT NULL,
    origin TEXT NOT NULL,
    body TEXT NOT NULL,
    head_key BLOB NOT NULL,
    PRIMARY KEY(subject, driver, account, window)
) WITHOUT ROWID;
"#;

pub fn create_limits_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(LIMITS_SCHEMA).context("creating observed account limits projection")
}

pub fn open_limits(transaction: &Transaction<'_>) -> Result<()> {
    create_limits_schema(transaction)?;
    let has_history: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='usage.limits.observed')", [], |row| row.get(0))?;
    if !has_history { mark_limits_ready(transaction)?; }
    Ok(())
}

fn mark_limits_ready(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute("INSERT OR REPLACE INTO meta(key,value) VALUES(?1,'4')", [LIMITS_READY])?;
    Ok(())
}

pub fn flush_limits(transaction: &Transaction<'_>) -> Result<usize> {
    flush_limits_page(transaction, LIMITS_APPEND_PAGE)
}

pub fn flush_limits_page(transaction: &Transaction<'_>, limit: usize) -> Result<usize> {
    let through = transaction.query_row("SELECT value FROM meta WHERE key=?1", [LIMITS_CURSOR],
        |row| row.get::<_, String>(0)).optional()?.and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
    let claims = transaction.prepare_cached(
        "SELECT id,subject,origin,body,store_index FROM claims WHERE kind='usage.limits.observed' AND store_index>?1 ORDER BY store_index LIMIT ?2")?
        .query_map(params![through, limit as i64], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, u64>(4)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut newest = through;
    for (id, subject, origin, body, index) in &claims {
        newest = newest.max(*index);
        let head_key = canonical::sortable_key(&canonical::claim_key(transaction, id)?);
        for value in observation_readings(&serde_json::from_str(body)?) {
            let Some((reading, _)) = reading(origin, &value) else { continue; };
            let window = &reading.windows[0];
            transaction.execute(
                "INSERT INTO observed_account_limits_v4(subject,driver,account,window,measured_at_unix_ms,origin,body,head_key)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(subject,driver,account,window) DO UPDATE SET measured_at_unix_ms=excluded.measured_at_unix_ms,
                    origin=excluded.origin,body=excluded.body,head_key=excluded.head_key
                 WHERE (excluded.measured_at_unix_ms,excluded.head_key) > (measured_at_unix_ms,head_key)",
                params![subject, reading.driver, reading.account, window.window, reading.measured_at_unix_ms as i64,
                    origin, serde_json::to_string(&value)?, head_key])?;
        }
    }
    if newest > through {
        transaction.execute("INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![LIMITS_CURSOR,newest.to_string()])?;
    }
    if claims.len() < limit { mark_limits_ready(transaction)?; }
    Ok(claims.len())
}

pub fn refold_limits_tx(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute("DELETE FROM observed_account_limits_v4", [])?;
    transaction.execute("DELETE FROM meta WHERE key IN (?1,?2)", [LIMITS_CURSOR, LIMITS_READY])?;
    while flush_limits_page(transaction, LIMITS_CATCH_UP_PAGE)? == LIMITS_CATCH_UP_PAGE {}
    Ok(())
}

/// Unfiltered deterministic evidence for checkpoint equivalence, not enforcement.
pub fn account_limits_at(connection: &Connection) -> Result<Vec<AccountLimit>> {
    account_limits_with(connection, None)
}

pub fn account_limits_for_writers(connection: &Connection, writers: &BTreeSet<String>) -> Result<Vec<AccountLimit>> {
    account_limits_with(connection, Some(writers))
}

fn account_limits_with(connection: &Connection, writers: Option<&BTreeSet<String>>) -> Result<Vec<AccountLimit>> {
    let mut accounts = BTreeMap::<(String,String), (AccountLimit, BTreeMap<String,(LimitWindow,Vec<u8>)>)>::new();
    let mut statement = connection.prepare_cached("SELECT subject,origin,body,head_key FROM observed_account_limits_v4")?;
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
        row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?)))? {
        let (subject, origin, body, head_key) = row?;
        if writers.is_some_and(|writers| !writers.contains(&subject)) { continue; }
        let Some((mut reading, _)) = reading(&origin, &serde_json::from_str(&body)?) else { continue; };
        reading.measured_by = subject;
        reading.host.clone_from(&reading.windows[0].host);
        let key = (reading.driver.clone(),reading.account.clone());
        let windows = std::mem::take(&mut reading.windows);
        let entry = match accounts.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert((reading, BTreeMap::new())),
            std::collections::btree_map::Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                if (reading.measured_at_unix_ms, &reading.measured_by) > (entry.0.measured_at_unix_ms, &entry.0.measured_by) {
                    entry.0 = reading;
                }
                entry
            }
        };
        for window in windows {
            if entry.1.get(&window.window).is_none_or(|(current,key)|
                (window.measured_at_unix_ms,&head_key) > (current.measured_at_unix_ms,key)) {
                entry.1.insert(window.window.clone(),(window,head_key.clone()));
            }
        }
    }
    Ok(accounts.into_values().map(|(mut account,windows)| {
        account.windows = windows.into_values().map(|(window,_)| window).collect();
        account
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> LimitWindow {
        LimitWindow { window:"weekly".into(),used_percent:Some(99.0),resets_at_unix_ms:Some(10_000_000),
            measured_at_unix_ms:1_000_000,coverage:"reported".into(),source:"omp-usage".into(),host:"alder".into() }
    }

    #[test]
    fn evidence_expires_after_one_hour_even_with_a_longer_configured_horizon() {
        let reading = window();
        assert!(reading.available(4_600_000,9_000_000));
        assert!(!reading.available(4_600_001,9_000_000));
        assert!(!reading.available(10_000_000,9_000_000));
    }

    #[test]
    fn gaps_missing_resets_and_future_evidence_are_unavailable() {
        let mut reading = window();
        reading.coverage = "unreported".into();
        reading.used_percent = None;
        assert!(!reading.available(1_000_000,ACCOUNT_READING_WINDOW_MS));
        reading = window();
        reading.resets_at_unix_ms = None;
        assert!(!reading.available(1_000_000,ACCOUNT_READING_WINDOW_MS));
        reading = window();
        assert!(!reading.available(939_999,ACCOUNT_READING_WINDOW_MS));
    }
}
