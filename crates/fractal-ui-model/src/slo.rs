//! How urgent an objective or budget row is: the one rule the terminal UI colors by and st
//! serves to the phone and the web app as `SloRow.tone`.
//!
//! An objective that missed its target, and a budget with little left, need the person (red).
//! A met row is fine (green), a stale reading is doubtful (yellow), and a row with nothing
//! measured is quiet.

/// A budget with this little left (percent) reads as nearly spent.
pub const NEARLY_SPENT: f64 = 10.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Red,
    Green,
    Yellow,
    Quiet,
}

impl Tone {
    /// The word st serves.
    pub fn word(self) -> &'static str {
        match self {
            Tone::Red => "red",
            Tone::Green => "green",
            Tone::Yellow => "yellow",
            Tone::Quiet => "quiet",
        }
    }
}

/// The tone of a row from its kind (`objective` or `budget`), its status (`met`, `missed`,
/// `no-data` or `stale`) and what is left of its allowance or error budget.
pub fn tone(kind: &str, status: &str, remaining_percent: Option<f64>) -> Tone {
    if status == "missed" || kind == "budget" && remaining_percent.is_some_and(|left| left <= NEARLY_SPENT) {
        return Tone::Red;
    }
    match status {
        "met" => Tone::Green,
        "stale" => Tone::Yellow,
        _ => Tone::Quiet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missed_objective_and_a_nearly_spent_budget_are_red() {
        assert_eq!(tone("objective", "missed", Some(0.0)), Tone::Red);
        assert_eq!(tone("budget", "met", Some(10.0)), Tone::Red);
        assert_eq!(tone("budget", "stale", Some(3.0)), Tone::Red);
        // An objective with an error budget close to empty is not red until it misses.
        assert_eq!(tone("objective", "met", Some(2.0)), Tone::Green);
    }

    #[test]
    fn other_rows_are_green_yellow_or_quiet_by_status() {
        assert_eq!(tone("budget", "met", Some(10.5)), Tone::Green);
        assert_eq!(tone("budget", "stale", Some(60.0)), Tone::Yellow);
        assert_eq!(tone("budget", "no-data", None), Tone::Quiet);
        assert_eq!(tone("objective", "no-data", None), Tone::Quiet);
        assert_eq!(Tone::Yellow.word(), "yellow");
    }
}
