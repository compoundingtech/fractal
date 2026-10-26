//! Account limits: a recent reading of each paying account's 5-hour and weekly limits, and
//! the policy that stops an account's seats as it nears its weekly limit.
//!
//! A harness reports its account's limits with every status refresh, so every seat on an account
//! repeats them, and an idle seat keeps its last reading. An account's limits are therefore the
//! highest weekly reading within an hour of the freshest source observation, in the latest reset
//! window, even after seats switch away, labelled with its original time and measuring seat.
//! This also protects readers from older producers that re-stamp cached limits. Readings never
//! cross accounts or providers: a harness that names no account reads as `PROVIDER/unknown`.
//! Seatless identity-less history is hidden once that driver has identified evidence; it is
//! never attributed to an identified account, and active seats keep their unknown evidence.


use super::*;

use serde::Deserialize;
use smallclaims::store::canonical;

pub const ACCOUNT_READING_WINDOW_MS: u64 = 3_600_000;

/// Choose an account's reading. `readings` pairs each with its claim's canonical key, which breaks
/// the ties nothing in the readings themselves decides.
fn select_account_reading(readings: Vec<(AccountLimit, Vec<u8>)>) -> AccountLimit {
    // A partial snapshot after a relaunch can know only the five-hour window. It is not a
    // weekly observation and must neither erase nor freshen the last weekly evidence.
    let has_weekly = readings
        .iter()
        .any(|(limit, _)| limit.weekly_percent.is_some());
    let readings = readings
        .into_iter()
        .filter(|(limit, _)| !has_weekly || limit.weekly_percent.is_some())
        .collect::<Vec<_>>();
    let latest = readings
        .iter()
        .map(|(limit, _)| limit.measured_at_unix_ms)
        .max()
        .unwrap();
    let since = latest.saturating_sub(ACCOUNT_READING_WINDOW_MS);
    // Reset advancement beats percentage: an old window must not resurrect exhaustion after
    // a reset, even when an older producer publishes that window again with a fresh stamp.
    let reset = readings
        .iter()
        .filter(|(limit, _)| limit.measured_at_unix_ms >= since)
        .filter_map(|(limit, _)| limit.weekly_resets_at_unix_ms)
        .max();
    readings
        .into_iter()
        .filter(|(limit, _)| {
            limit.measured_at_unix_ms >= since && limit.weekly_resets_at_unix_ms == reset
        })
        .max_by(|(left, left_key), (right, right_key)| {
            left.weekly_percent
                .partial_cmp(&right.weekly_percent)
                .unwrap()
                .then_with(|| {
                    // Providers that report only a five-hour window use that window's level.
                    if left.weekly_percent.is_none() && right.weekly_percent.is_none() {
                        left.five_hour_resets_at_unix_ms
                            .cmp(&right.five_hour_resets_at_unix_ms)
                            .then_with(|| {
                                left.five_hour_percent
                                    .partial_cmp(&right.five_hour_percent)
                                    .unwrap()
                            })
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| {
                    (left.measured_at_unix_ms, &left.measured_by)
                        .cmp(&(right.measured_at_unix_ms, &right.measured_by))
                })
                .then_with(|| left_key.cmp(right_key))
        })
        .map(|(limit, _)| limit)
        .expect("the latest reset has a recent reading")
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AccountLimit {
    /// The account label, or `DRIVER/unknown` when the harness named none.
    pub account: String,
    /// The declared account (`ada/claude`) the measuring seat was launched on, when it was bound
    /// to one. Bound seats derive the opaque label from this name; unbound seats use provider identity.
    #[serde(default)]
    pub account_ref: Option<String>,
    /// Whether the source named a provider identity or a declared account. This says nothing
    /// about the age or completeness of its quota evidence.
    #[serde(default)]
    pub identified: bool,
    /// The provider label this reading carried before a declaration claimed it as the account's
    /// identity. The limits policy still matches it, so naming an account never changes whether it
    /// is exempt. Never serialized.
    #[serde(skip)]
    pub identity: Option<String>,
    pub driver: String,
    pub plan: Option<String>,
    pub five_hour_percent: Option<f64>,
    pub five_hour_resets_at_unix_ms: Option<u64>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at_unix_ms: Option<u64>,
    pub measured_at_unix_ms: u64,
    /// The seat whose harness measured this reading, and that seat's host.
    pub measured_by: String,
    pub host: String,
    /// Every seat whose newest reading names this account.
    pub seats: Vec<String>,
}

pub fn reading(origin: &str, body: &Value) -> Option<(AccountLimit, String)> {
    let fields = body.get("fields")?;
    let driver = fields["driver"].as_str()?.to_owned();
    let identity = fields["account"]
        .as_str()
        .filter(|account| !account.is_empty());
    let account_ref = fields["account_ref"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    // Older bound producers can carry a generic label or none at all. Match the publishers'
    // stable declared label so one declared account cannot split as its provider label changes.
    let account = account_ref.as_ref().map_or_else(
        || identity.map_or_else(|| format!("{driver}/unknown"), str::to_owned),
        |name| fractal_drivers::account::account_label(&driver, &format!("declared:{name}")),
    );
    let identified = account_ref.is_some() || identity.is_some();
    let percent = |name: &str| fields[name].as_f64().filter(|value| value.is_finite());
    Some((
        AccountLimit {
            account: account.clone(),
            account_ref,
            identified,
            identity: None,
            driver,
            plan: fields["plan"].as_str().map(str::to_owned),
            five_hour_percent: percent("five_hour_percent"),
            five_hour_resets_at_unix_ms: fields["five_hour_resets_at_unix_ms"].as_u64(),
            weekly_percent: percent("weekly_percent"),
            weekly_resets_at_unix_ms: fields["weekly_resets_at_unix_ms"].as_u64(),
            measured_at_unix_ms: fields["measured_at_unix_ms"].as_u64()?,
            measured_by: String::new(),
            host: origin.to_owned(),
            seats: Vec::new(),
        },
        account,
    ))
}

// Account limits are a projection of the `harness.limits` claims, kept as two small tables that
// each claim updates as it arrives. Seats publish a limits claim every few seconds and the kind is
// never trimmed (101,000 claims on one real store), so folding all of them on every read, twice
// every two minutes, cost seconds of reading and parsing. A reading now costs the rows it touches.
//
// `account_limit_readings` holds, per account and per weekly or partial (five-hour only)
// reading, the readings within an hour of that account's newest: all the answer can still depend
// on, since the newest only moves forward and a weekly reading anywhere hides every partial one.
// `account_limit_seats` holds each seat's newest reading in canonical claim order and the account
// it names. Every tie is broken by the claim's canonical key, never by the order this node admitted
// claims in, so every node folds the same answer. A claim's store index only grows, so a cursor in
// `meta` says which claims are folded; it decides only what is folded next, never a winner.
// The tables keep version 1's columns, which this version fills but never decides by, so a
// version 1 build that opens this store can still fold into them after it empties them.
const LIMITS_PROJECTION_VERSION: &str = "2";
const LIMITS_CURSOR: &str = "account_limits_through_index";
const LIMITS_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h36d2fcc0cc4f owner=fractal_store::store::limits TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS account_limit_readings (
    driver TEXT NOT NULL,
    account TEXT NOT NULL,
    account_ref TEXT NOT NULL,
    weekly INTEGER NOT NULL CHECK(weekly IN (0,1)),
    measured_at_unix_ms INTEGER NOT NULL,
    subject TEXT NOT NULL,
    store_index INTEGER NOT NULL,
    origin TEXT NOT NULL,
    body TEXT NOT NULL,
    head_key BLOB NOT NULL DEFAULT x'',
    PRIMARY KEY (driver, account, account_ref, weekly, measured_at_unix_ms, subject, store_index)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS account_limit_seats (
    subject TEXT PRIMARY KEY,
    driver TEXT NOT NULL,
    account TEXT NOT NULL,
    account_ref TEXT NOT NULL,
    accepted_at_unix_ms INTEGER NOT NULL,
    store_index INTEGER NOT NULL,
    head_key BLOB NOT NULL DEFAULT x''
) WITHOUT ROWID;
"#;

pub fn create_limits_schema(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hf8150ea039e8 owner=fractal_store::store::limits TODO=trace-statement-and-value-representation */ LIMITS_SCHEMA)
        .context("creating the account limits projection")
}

/// Claims an append or a projection pass folds at most: the new one, with slack for a few that
/// arrived replicated and wait for projection.
const LIMITS_APPEND_PAGE: usize = 64;
/// Claims one catch-up page folds: about 25 ms of writer time, after which the writer serves what
/// queued meanwhile.
pub const LIMITS_CATCH_UP_PAGE: usize = 200;
pub const LIMITS_READY: &str = "account_limits_ready";

/// Check the projection's version when a store opens. Opening never folds history: a store that
/// has `harness.limits` claims but no projection is filled by [`Store::catch_up_account_limits`],
/// a page at a time, and reads say "not yet known" until it has finished.
pub fn open_limits(transaction: &Transaction<'_>) -> Result<()> {
    let version: Option<String> = transaction
        .query_row(
            "SELECT value FROM meta WHERE key='account_limits_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() != Some(LIMITS_PROJECTION_VERSION) {
        // An earlier version's tables lack the canonical keys: they hold nothing but what the
        // claims fold to, so drop them and fold the claims again.
        transaction.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hcb99c194170a owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE IF EXISTS account_limit_readings; DROP TABLE IF EXISTS account_limit_seats;",
        )?;
        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h423c3b87a417 owner=fractal_store::store::limits TODO=trace-statement-and-value-representation */ LIMITS_SCHEMA)?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h8a09ed0018a1 owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN (?1, ?2)",
            [LIMITS_CURSOR, LIMITS_READY],
        )?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hc9a89786c83a owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('account_limits_version',?1)",
            [LIMITS_PROJECTION_VERSION],
        )?;
    }
    // A store with nothing to catch up on is ready at once.
    let ready: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
        [LIMITS_READY],
        |row| row.get(0),
    )?;
    let has_history: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='harness.limits')",
        [],
        |row| row.get(0),
    )?;
    if !ready && !has_history {
        mark_limits_ready(transaction)?;
    }
    Ok(())
}

fn mark_limits_ready(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h7b845cb0a9d6 owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1, ?2)",
        [LIMITS_READY, LIMITS_PROJECTION_VERSION],
    )?;
    Ok(())
}

/// Fold the next `limit` `harness.limits` claims after the cursor into the tables, and return how
/// many. A page that finds fewer than `limit` has reached the end: the projection is ready.
pub fn flush_limits(transaction: &Transaction<'_>) -> Result<usize> {
    flush_limits_page(transaction, LIMITS_APPEND_PAGE)
}

pub fn flush_limits_page(transaction: &Transaction<'_>, limit: usize) -> Result<usize> {
    let through: u64 = transaction
        .query_row("SELECT value FROM meta WHERE key=?1", [LIMITS_CURSOR], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let claims = transaction
        .prepare_cached(
            "SELECT id, subject, origin, body, accepted_at_unix_ms, store_index FROM claims
             WHERE kind='harness.limits' AND store_index>?1 ORDER BY store_index LIMIT ?2",
        )?
        .query_map(params![through, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, u64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut newest_index = through;
    let mut touched = BTreeSet::<(String, String, String, bool)>::new();
    for (id, subject, origin, body, accepted_at, store_index) in &claims {
        newest_index = newest_index.max(*store_index);
        let value: Value = serde_json::from_str(body)?;
        let Some((limit, _)) = reading(origin, &value) else {
            continue;
        };
        let head_key = canonical::sortable_key(&canonical::claim_key(transaction, id)?);
        let account_ref = limit.account_ref.clone().unwrap_or_default();
        let weekly = limit.weekly_percent.is_some();
        transaction
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hf1ac321eba1a owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO account_limit_readings
                 (driver, account, account_ref, weekly, measured_at_unix_ms, subject, store_index, origin, body, head_key)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?
            .execute(/* sqlite-large-value: allow-for-now lv-hc848209c8c7f owner=fractal_store::store::limits TODO=trace-statement-and-value-representation */ params![
                limit.driver,
                limit.account,
                account_ref,
                weekly,
                limit.measured_at_unix_ms as i64,
                subject,
                *store_index as i64,
                origin,
                body,
                head_key
            ])?;
        // The seat's newest reading in canonical claim order.
        transaction
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hab71d076506e owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO account_limit_seats
                 (subject, driver, account, account_ref, accepted_at_unix_ms, store_index, head_key)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(subject) DO UPDATE SET driver=excluded.driver,
                    account=excluded.account, account_ref=excluded.account_ref,
                    accepted_at_unix_ms=excluded.accepted_at_unix_ms,
                    store_index=excluded.store_index, head_key=excluded.head_key
                 WHERE excluded.head_key > head_key",
            )?
            .execute(/* sqlite-large-value: allow-for-now lv-h0bdd1001c4f9 owner=fractal_store::store::limits TODO=trace-statement-and-value-representation */ params![
                subject,
                limit.driver,
                limit.account,
                account_ref,
                accepted_at.parse::<u64>().unwrap_or(0) as i64,
                *store_index as i64,
                head_key
            ])?;
        touched.insert((limit.driver, limit.account, account_ref, weekly));
    }
    // Readings more than an hour older than their group's newest can no longer decide anything.
    for (driver, account, account_ref, weekly) in touched {
        transaction
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-heddd3ac44bac owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM account_limit_readings
                 WHERE driver=?1 AND account=?2 AND account_ref=?3 AND weekly=?4
                   AND measured_at_unix_ms < (
                       SELECT MAX(measured_at_unix_ms) FROM account_limit_readings
                       WHERE driver=?1 AND account=?2 AND account_ref=?3 AND weekly=?4) - ?5",
            )?
            .execute(/* sqlite-large-value: allow-for-now lv-heeda99e2cde0 owner=fractal_store::store::limits TODO=trace-statement-and-value-representation */ params![
                driver,
                account,
                account_ref,
                weekly,
                ACCOUNT_READING_WINDOW_MS as i64
            ])?;
    }
    if newest_index > through {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h3764891d819e owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
            params![LIMITS_CURSOR, newest_index.to_string()],
        )?;
    }
    if claims.len() < limit {
        let ready: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
            [LIMITS_READY],
            |row| row.get(0),
        )?;
        if !ready {
            mark_limits_ready(transaction)?;
        }
    }
    Ok(claims.len())
}

/// Empty the projection and fold every `harness.limits` claim into it again, in one
/// transaction: what a checkpoint proof's copy reads, as a node that never trimmed would.
pub fn refold_limits_tx(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-he4e04bd23262 owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM account_limit_readings", [])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h96bc76b659d9 owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM account_limit_seats", [])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h4b99368d5627 owner=fractal_store::store::limits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [LIMITS_CURSOR])?;
    while flush_limits_page(transaction, LIMITS_CATCH_UP_PAGE)? == LIMITS_CATCH_UP_PAGE {}
    Ok(())
}

pub fn account_limits_at(connection: &Connection) -> Result<Vec<AccountLimit>> {
    type Key = (String, String, Option<String>);
    let mut statement = connection.prepare_cached(
        "SELECT driver, account, account_ref, weekly, origin, subject, body, head_key
         FROM account_limit_readings",
    )?;
    let mut rows = Vec::new();
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Vec<u8>>(7)?,
        ))
    })? {
        rows.push(row?);
    }
    // A seat that reported under a provider identity and later under a declared account is one
    // login that changed identity (a re-login), not two accounts: the identity's readings are the
    // declared account's. Only a seat's own reports link them, so nothing else is ever merged.
    let mut named = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for (driver, _, account_ref, _, _, subject, _, _) in &rows {
        if !account_ref.is_empty() {
            named
                .entry((driver.clone(), subject.clone()))
                .or_default()
                .insert(account_ref.clone());
        }
    }
    let mut alias = BTreeMap::<(String, String), String>::new();
    for (driver, account, account_ref, _, _, subject, _, _) in &rows {
        if account_ref.is_empty()
            && let Some(declared) = named
                .get(&(driver.clone(), subject.clone()))
                .and_then(|names| names.first())
        {
            let entry = alias.entry((driver.clone(), account.clone()));
            entry
                .and_modify(|current| {
                    if declared < current {
                        *current = declared.clone();
                    }
                })
                .or_insert_with(|| declared.clone());
        }
    }
    // A declaration may claim a provider label as the account's own: every reading under it, from
    // any seat, is that account's.
    for declared in super::accounts::declared_accounts_at(connection)? {
        for identity in &declared.identities {
            if let Some((driver, _)) = identity.split_once('/') {
                alias
                    .entry((driver.to_owned(), identity.clone()))
                    .or_insert_with(|| declared.name.clone());
            }
        }
    }
    let resolve = |driver: String, account: String, account_ref: String| -> Key {
        match alias.get(&(driver.clone(), account.clone())) {
            Some(declared) if account_ref.is_empty() => (
                driver.clone(),
                fractal_drivers::account::account_label(&driver, &format!("declared:{declared}")),
                Some(declared.clone()),
            ),
            _ => (driver, account, (!account_ref.is_empty()).then_some(account_ref)),
        }
    };
    let mut weekly = BTreeMap::<Key, Vec<(AccountLimit, Vec<u8>)>>::new();
    let mut partial = BTreeMap::<Key, Vec<(AccountLimit, Vec<u8>)>>::new();
    for (driver, account, account_ref, is_weekly, origin, subject, body, head_key) in rows {
        let body: Value = serde_json::from_str(&body)?;
        let Some((mut limit, _)) = reading(&origin, &body) else {
            continue;
        };
        limit.measured_by = subject;
        let aliased = account_ref.is_empty().then(|| account.clone());
        let key = resolve(driver, account, account_ref);
        if let Some(declared) = &key.2 {
            limit.identity = aliased.filter(|raw| *raw != key.1);
            limit.account = key.1.clone();
            limit.account_ref = Some(declared.clone());
            limit.identified = true;
        }
        if is_weekly {
            weekly.entry(key).or_default().push((limit, head_key));
        } else {
            partial.entry(key).or_default().push((limit, head_key));
        }
    }
    let mut accounts = BTreeMap::<Key, AccountLimit>::new();
    // A partial snapshot after a relaunch can know only the five-hour window. It is not a weekly
    // observation and must neither erase nor freshen the last weekly evidence.
    for (key, readings) in partial {
        if !weekly.contains_key(&key) {
            accounts.insert(key, select_account_reading(readings));
        }
    }
    for (key, readings) in weekly {
        accounts.insert(key, select_account_reading(readings));
    }
    let mut seats = connection.prepare_cached(
        "SELECT subject, driver, account, account_ref FROM account_limit_seats ORDER BY subject",
    )?;
    for row in seats.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })? {
        let (seat, driver, account, account_ref) = row?;
        let key = resolve(driver, account, account_ref);
        accounts
            .get_mut(&key)
            .expect("the seat reported an account")
            .seats
            .push(seat);
    }
    let identified_drivers = accounts
        .values()
        .filter(|limit| limit.identified)
        .map(|limit| limit.driver.clone())
        .collect::<BTreeSet<_>>();
    let mut active = connection.prepare_cached("SELECT subject FROM desired WHERE kind='agent'")?;
    let active_seats = active
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    accounts.retain(|_, limit| {
        limit.identified
            || !identified_drivers.contains(&limit.driver)
            || limit.seats.iter().any(|seat| active_seats.contains(seat))
    });
    Ok(accounts.into_values().collect())
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::limits_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::limits_tests_shared_seats::{HOUR, read, read_account, read_unnamed};
use super::*;

    #[test]
    fn the_catch_up_page_seeks_the_kind_index_past_the_cursor() {
        let store = Store::open_memory("alder").unwrap();
        let connection = store.readers.get();
        let mut statement = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id, subject, origin, body, accepted_at_unix_ms, store_index
                 FROM claims WHERE kind='harness.limits' AND store_index>?1
                 ORDER BY store_index LIMIT ?2",
            )
            .unwrap();
        let plan = statement
            .query_map([0, 200], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            plan,
            ["SEARCH claims USING INDEX claims_kind_index (kind=? AND store_index>?)"]
        );
    }

}
