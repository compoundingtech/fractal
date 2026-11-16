//! Daily ask digest boundaries use local calendar dates, never fixed 24-hour subtraction.
use anyhow::{Context as _, Result};
use chrono::{Days, LocalResult, NaiveDate, TimeZone as _, Utc};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::config::{AsksConfig, DEFAULT_ASK_DIGEST_AT, DEFAULT_ASK_DIGEST_TIMEZONE};
use crate::store::Store;

const DIGEST_AT_KEY: &str = "person_ask_digest_at";
const DIGEST_TIMEZONE_KEY: &str = "person_ask_digest_timezone";

/// Today's local digest and the preceding local daily boundary. The window is half-open:
/// `window_start <= resolved_at < window_end`. `due` includes the exact scheduled instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AskDigestWindow {
    pub date: NaiveDate,
    pub window_start: u128,
    pub window_end: u128,
    pub due_at: u128,
    pub next_deadline: u128,
    pub due: bool,
}

/// Calculate a person's digest using an injected clock and IANA timezone. Before today's
/// scheduled time, this returns today's pending window, not yesterday's digest. A repeated
/// local minute uses its first instant; a missing minute uses the first real minute after it.
pub fn ask_digest_window(now: u128, timezone: &str, digest_at: &str) -> Result<AskDigestWindow> {
    let minute = AsksConfig::parse_digest_at_minute(digest_at)?;
    let zone: chrono_tz::Tz = timezone.parse().context("asks.digest_timezone must be an IANA timezone")?;
    let instant = Utc.timestamp_millis_opt(i64::try_from(now).context("invalid digest clock")?)
        .single().context("invalid digest clock")?;
    let date = instant.with_timezone(&zone).date_naive();
    let previous = date.checked_sub_days(Days::new(1)).context("digest date overflow")?;
    let due_at = local_boundary(zone, date, minute)?;
    let due = now >= due_at;
    let next_deadline = if due {
        let tomorrow = date.checked_add_days(Days::new(1)).context("digest date overflow")?;
        local_boundary(zone, tomorrow, minute)?
    } else {
        due_at
    };
    Ok(AskDigestWindow {
        date,
        window_start: local_boundary(zone, previous, minute)?,
        window_end: due_at,
        due_at,
        next_deadline,
        due,
    })
}

fn local_boundary(zone: chrono_tz::Tz, date: NaiveDate, minute: u16) -> Result<u128> {
    let requested = date.and_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)
        .context("invalid digest local time")?;
    // Match calendar schedules: gaps advance to the next valid minute, folds use the first.
    for offset in 0..=2880 {
        let local = requested.checked_add_signed(chrono::Duration::minutes(offset))
            .context("digest date overflow")?;
        let instant = match zone.from_local_datetime(&local) {
            LocalResult::Single(instant) => instant,
            LocalResult::Ambiguous(earlier, later) => earlier.min(later),
            LocalResult::None => continue,
        };
        return u128::try_from(instant.timestamp_millis()).context("digest boundary predates Unix epoch");
    }
    anyhow::bail!("no valid local digest instant within two days of {requested} in {zone}")
}

/// Read the installed local member policy as `(digest_at, fleet_timezone)`. Neither person
/// declarations nor availability currently carry a timezone; this explicit fleet setting is
/// their fallback, independent of the daemon host's operating-system timezone.
pub fn ask_digest_policy(connection: &Connection) -> Result<(String, String)> {
    let read = |key: &str, default: &str| -> rusqlite::Result<String> {
        Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [key], |row| row.get(0))
            .optional()?.unwrap_or_else(|| default.to_owned()))
    };
    Ok((read(DIGEST_AT_KEY, DEFAULT_ASK_DIGEST_AT)?, read(DIGEST_TIMEZONE_KEY, DEFAULT_ASK_DIGEST_TIMEZONE)?))
}

impl Store {
    /// Install both digest settings atomically after validating the wall time and timezone.
    /// Unchanged settings cause no write; invalid reloads preserve the last good policy.
    pub fn set_ask_digest_policy(&self, config: &AsksConfig) -> Result<bool> {
        config.digest_at_minute()?;
        config.digest_timezone.parse::<chrono_tz::Tz>()
            .context("asks.digest_timezone must be an IANA timezone")?;
        let current = ask_digest_policy(&self.readers.get())?;
        if current.0 == config.digest_at && current.1 == config.digest_timezone {
            return Ok(false);
        }
        self.connection.batched(|tx| {
            for (key, value) in [(DIGEST_AT_KEY, &config.digest_at), (DIGEST_TIMEZONE_KEY, &config.digest_timezone)] {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h05b10cf45ef8 owner=fractal_store::ask_digest_clock TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![key, value])?;
            }
            Ok::<_, rusqlite::Error>(())
        }).map_err(anyhow::Error::msg)??;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(at: &str) -> u128 {
        u128::try_from(chrono::DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis()).unwrap()
    }

    #[test]
    fn policy_changes_are_atomic_and_invalid_reload_keeps_the_last_policy() {
        let store = Store::open_memory("digest-clock").unwrap();
        assert_eq!(ask_digest_policy(&store.readers.get()).unwrap(), ("09:00".into(), "UTC".into()));
        assert!(!store.set_ask_digest_policy(&AsksConfig::default()).unwrap());
        let mut config = AsksConfig {
            digest_at: "08:30".into(),
            digest_timezone: "Europe/Berlin".into(),
            ..AsksConfig::default()
        };
        assert!(store.set_ask_digest_policy(&config).unwrap());
        assert!(!store.set_ask_digest_policy(&config).unwrap());
        let installed = ask_digest_policy(&store.readers.get()).unwrap();
        config.digest_timezone = "No/SuchZone".into();
        assert!(store.set_ask_digest_policy(&config).is_err());
        assert_eq!(ask_digest_policy(&store.readers.get()).unwrap(), installed);
        config.digest_timezone = "UTC".into();
        config.digest_at = "25:00".into();
        assert!(store.set_ask_digest_policy(&config).is_err());
        assert_eq!(ask_digest_policy(&store.readers.get()).unwrap(), installed);
    }

    #[test]
    fn daily_digest_is_due_at_exactly_nine_local() {
        let before = ask_digest_window(ms("2026-11-15T07:59:59.999Z"), "Europe/Berlin", "09:00").unwrap();
        assert!(!before.due);
        assert_eq!(before.date, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
        assert_eq!(before.next_deadline, ms("2026-11-15T08:00:00Z"));
        let due = ask_digest_window(before.next_deadline, "Europe/Berlin", "09:00").unwrap();
        assert!(due.due);
        assert_eq!(due.window_start, ms("2026-11-14T08:00:00Z"));
        assert_eq!(due.window_end, ms("2026-11-15T08:00:00Z"));
        assert_eq!(due.next_deadline, ms("2026-11-16T08:00:00Z"));
    }

    #[test]
    fn nine_local_survives_short_and_long_dst_days() {
        for (now, start, end, hours) in [
            ("2026-03-29T07:00:00Z", "2026-03-28T08:00:00Z", "2026-03-29T07:00:00Z", 23),
            ("2026-10-25T08:00:00Z", "2026-10-24T07:00:00Z", "2026-10-25T08:00:00Z", 25),
        ] {
            let window = ask_digest_window(ms(now), "Europe/Berlin", "09:00").unwrap();
            assert!(window.due);
            assert_eq!(window.window_start, ms(start));
            assert_eq!(window.window_end, ms(end));
            assert_eq!(window.window_end - window.window_start, hours * 3_600_000);
        }
    }

    #[test]
    fn gaps_advance_and_folds_choose_the_first_instant() {
        let gap = ask_digest_window(ms("2026-03-29T01:00:00Z"), "Europe/Berlin", "02:30").unwrap();
        assert!(gap.due);
        assert_eq!(gap.due_at, ms("2026-03-29T01:00:00Z"));
        let fold = ask_digest_window(ms("2026-10-25T00:30:00Z"), "Europe/Berlin", "02:30").unwrap();
        assert!(fold.due);
        assert_eq!(fold.due_at, ms("2026-10-25T00:30:00Z"));
        assert_eq!(fold.next_deadline, ms("2026-10-26T01:30:00Z"));
    }

    #[test]
    fn timezone_and_clock_are_explicit_and_validated() {
        let window = ask_digest_window(ms("2026-11-15T03:30:00Z"), "Asia/Kolkata", "09:00").unwrap();
        assert!(window.due);
        assert_eq!(window.due_at, ms("2026-11-15T03:30:00Z"));
        assert!(ask_digest_window(ms("2026-11-15T03:30:00Z"), "No/SuchZone", "09:00").is_err());
        assert!(ask_digest_window(u128::MAX, "UTC", "09:00").is_err());
    }
}
