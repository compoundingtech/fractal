//! A nominal calendar tick for declaration display; no occurrence/run history or authority.
use super::*;

/// First nominal declared calendar tick at or after `as_of`, including an exact tick.
/// Keep the scheduler's existing DST gap/fold calculation; ignore execution and catch-up.
pub fn next_declared_calendar_at(calendar: &CalendarSchedule, as_of: i64) -> Result<i64> {
    let before = as_of.checked_sub(1).context("calendar display clock underflow")?;
    calendar_occurrence(calendar, None, before, "skip", None).map(|(_, at)| at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis()
    }
    fn daily(timezone: &str, minute: u16) -> CalendarSchedule {
        CalendarSchedule { at_minute: minute, weekday: None, timezone: timezone.into() }
    }
    #[test]
    fn declaration_clock_keeps_exact_tick_and_then_selects_the_next_day() {
        let calendar = daily("UTC", 8 * 60);
        let at = utc("2027-03-28T08:00:00Z");
        assert_eq!(next_declared_calendar_at(&calendar, at - 1).unwrap(), at);
        assert_eq!(next_declared_calendar_at(&calendar, at).unwrap(), at);
        assert_eq!(next_declared_calendar_at(&calendar, at + 1).unwrap(), utc("2027-03-29T08:00:00Z"));
    }
    #[test]
    fn declaration_clock_uses_first_real_minute_of_gap_and_first_instant_of_fold() {
        let berlin = daily("Europe/Berlin", 2 * 60 + 30);
        assert_eq!(next_declared_calendar_at(&berlin, utc("2027-03-28T00:30:00Z")).unwrap(), utc("2027-03-28T01:00:00Z"));
        assert_eq!(next_declared_calendar_at(&berlin, utc("2027-03-28T01:00:00Z")).unwrap(), utc("2027-03-28T01:00:00Z"));
        assert_eq!(next_declared_calendar_at(&berlin, utc("2026-10-25T00:30:00Z")).unwrap(), utc("2026-10-25T00:30:00Z"));
        assert_eq!(next_declared_calendar_at(&berlin, utc("2026-10-25T00:30:00.001Z")).unwrap(), utc("2026-10-26T01:30:00Z"));
    }
    #[test]
    fn declaration_clock_keeps_weekday_and_refuses_unknown_zone() {
        let mut calendar = daily("UTC", 9 * 60);
        calendar.weekday = Some(1);
        assert_eq!(next_declared_calendar_at(&calendar, utc("2027-03-23T12:00:00Z")).unwrap(), utc("2027-03-29T09:00:00Z"));
        calendar.timezone = "Unknown/Zone".into();
        assert!(next_declared_calendar_at(&calendar, 0).is_err());
    }
}
