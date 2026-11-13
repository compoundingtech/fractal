//! The person's own availability and directive note in one pane: whether they are available,
//! which asks may interrupt them, whether their asks' deadlines are held, and the note agents
//! read. No collection stream carries availability, so the pane reads `availability.list` and
//! `notes.list` when it opens, when the terminal regains focus, after a reconnect and on `r`,
//! never on a timer.

use super::doc::{Doc, DocExt, Hit};
use super::{text, theme};
use chrono::{DateTime, TimeZone};
use fractal_client::{
    AvailabilityDeadlineClock, AvailabilityInterruptions, AvailabilityPresence, AvailabilityStatus, AvailabilityStatusSource, Client,
    ClientError, DirectiveNote, ErrorCode, PersonAvailability,
};
use ratatui::text::{Line, Span};
use std::fmt::Display;

/// Labels are padded to this many cells, so the values line up.
const LABEL: usize = 16;

/// Under a word a newer Fractal sent, which this version does not know and shows as `unknown`.
const UNKNOWN: &str = "A word this version does not know; updating shows it.";

/// What Fractal last said: the person's effective availability and their current note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) availability: PersonAvailability,
    pub(crate) note: Option<DirectiveNote>,
}

/// A request the live loop sends for the pane, answered with a [`Done`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    /// Read availability and the note; the generation fences an answer from an older connection.
    Read(u64),
}

#[derive(Debug)]
pub(crate) enum Done {
    Read(u64, Result<Reading, String>),
}

#[derive(Default)]
pub(crate) struct Availability {
    /// Whether the pane was on screen on the last pass: opening it reads again.
    shown: bool,
    /// Something may have changed since the last read: focus came back, a reconnect, `r`.
    stale: bool,
    reading: bool,
    /// Bumped by a reconnect, so a read sent before it is never shown.
    generation: u64,
    last: Option<Reading>,
    /// Why the last read failed: said beside an older reading, or alone.
    failed: Option<String>,
}

impl Availability {
    /// The requests the pane needs now, given whether it is on screen.
    pub(crate) fn wanted(&mut self, shown: bool) -> Vec<Want> {
        self.stale |= shown && !self.shown;
        self.shown = shown;
        let mut wants = Vec::new();
        if shown && self.stale && !self.reading {
            self.stale = false;
            self.reading = true;
            wants.push(Want::Read(self.generation));
        }
        wants
    }

    /// Read again as soon as the pane is on screen (now, if it is).
    pub(crate) fn refresh(&mut self) {
        self.stale = true;
    }

    /// A new connection, perhaps to another member: a read sent before it is not shown.
    pub(crate) fn connected(&mut self) {
        self.generation += 1;
        self.reading = false;
        self.stale = true;
    }

    pub(crate) fn done(&mut self, done: Done) {
        match done {
            Done::Read(generation, outcome) => {
                if generation != self.generation {
                    return;
                }
                self.reading = false;
                match outcome {
                    Ok(reading) => {
                        self.last = Some(reading);
                        self.failed = None;
                    }
                    Err(why) => self.failed = Some(why),
                }
            }
        }
    }

    /// The pane. `now` is the person's local clock, which every time is shown in.
    pub(crate) fn page<Tz: TimeZone>(&self, live: bool, width: usize, now: &DateTime<Tz>) -> Doc
    where
        Tz::Offset: Display,
    {
        let mut doc = Doc::new();
        let mut title = vec![Span::styled("Availability", theme::strong(theme::TEXT))];
        if let Some(reading) = &self.last {
            title.push(Span::styled(format!("  {}", reading.availability.person), theme::dim()));
        }
        doc.line(Line::from(title));
        doc.blank();
        let Some(reading) = &self.last else {
            let words = match (&self.failed, live) {
                (Some(why), _) => format!("Could not read your availability: {why}"),
                (None, true) => "Reading your availability…".into(),
                (None, false) => "Availability is read from a live Fractal; this demo has none.".into(),
            };
            doc.wrap(&[text::run(words, theme::soft())], width);
            if self.failed.is_some() {
                doc.blank();
                doc.buttons(&[("r", "try again", Hit::Availability('r'), theme::ACCENT)]);
            }
            return doc;
        };
        let at = |label: &str, time: &Option<String>| time.as_deref().map(|time| format!(" · {label} {}", when(time, now)));
        let found = &reading.availability;
        let (status, color) = match found.status {
            AvailabilityStatus::Available => ("available", theme::GREEN),
            AvailabilityStatus::Away => ("away", theme::YELLOW),
            AvailabilityStatus::Unknown => ("unknown", theme::TEXT),
        };
        let (source, explanation) = match found.status_source {
            AvailabilityStatusSource::Override => (" · set by you", None),
            AvailabilityStatusSource::Presence => (" · from presence", None),
            AvailabilityStatusSource::Unreported => {
                (" · no device reporting", Some("No device has reported for 24 hours; agents ask as usual."))
            }
            AvailabilityStatusSource::Unknown => (" · from an unknown source", None),
        };
        let mut details = source.to_owned();
        details.extend(at("until", &found.status_until));
        row(&mut doc, "Status", status, theme::strong(color), &details, width);
        let unknown = found.status == AvailabilityStatus::Unknown || found.status_source == AvailabilityStatusSource::Unknown;
        explain(&mut doc, if unknown { Some(UNKNOWN) } else { explanation }, width);
        if let Some(presence) = found.presence {
            let details = at("reported", &found.presence_observed_at).unwrap_or_default();
            row(&mut doc, "Presence", presence.as_str(), theme::text(), &details, width);
            explain(&mut doc, (presence == AvailabilityPresence::Unknown).then_some(UNKNOWN), width);
        }
        let (interruptions, explanation) = match found.interruptions {
            AvailabilityInterruptions::All => ("all asks", None),
            AvailabilityInterruptions::Urgent => ("urgent asks only", Some("Only an ask with priority critical interrupts you.")),
            AvailabilityInterruptions::None => ("none", Some("No ask interrupts you.")),
            AvailabilityInterruptions::Unknown => ("unknown", Some(UNKNOWN)),
        };
        let details = at("until", &found.interruptions_until).unwrap_or_default();
        row(&mut doc, "Interruptions", interruptions, theme::text(), &details, width);
        explain(&mut doc, explanation, width);
        let (clock, explanation) = match found.deadline_clock {
            AvailabilityDeadlineClock::Run => ("running", None),
            AvailabilityDeadlineClock::HoldRoutine => (
                "holding routine asks",
                Some("Routine asks' deadlines wait and urgent asks keep running; the time held is added to each deadline."),
            ),
            AvailabilityDeadlineClock::HoldAll => ("holding every ask", Some("Every ask's deadline waits; the time held is added to it.")),
            AvailabilityDeadlineClock::Unknown => ("unknown", Some(UNKNOWN)),
        };
        let mut details = at("since", &found.deadline_clock_since).unwrap_or_default();
        details.extend(at("until", &found.deadline_clock_until));
        row(&mut doc, "Deadline clock", clock, theme::text(), &details, width);
        explain(&mut doc, explanation, width);
        match &reading.note {
            Some(note) => {
                let mut details = at("until", &note.expires_at).unwrap_or_default();
                if note.author != note.person {
                    details.push_str(&format!(" · by {}", note.author));
                }
                row(&mut doc, "Note", &note.text, theme::text(), &details, width);
            }
            None => row(&mut doc, "Note", "none", theme::dim(), "", width),
        }
        if let Some(why) = &self.failed {
            doc.blank();
            doc.wrap(&[text::run(format!("Could not read it again: {why}"), theme::soft())], width);
        }
        doc.blank();
        doc.buttons(&[("r", "read again", Hit::Availability('r'), theme::ACCENT)]);
        doc
    }
}

/// `label   value · details`, the value wrapped under itself.
fn row(doc: &mut Doc, label: &str, value: &str, style: ratatui::style::Style, details: &str, width: usize) {
    let runs = [text::run(text::sanitize(value), style), text::run(details.to_owned(), theme::dim())];
    doc.lines(text::wrap(
        &runs,
        width,
        &[text::run(format!("{label:<LABEL$}"), theme::dim())],
        &[text::run(" ".repeat(LABEL), theme::dim())],
        None,
    ));
}

/// What a value other than the default does, under its row.
fn explain(doc: &mut Doc, explanation: Option<&str>, width: usize) {
    if let Some(explanation) = explanation {
        row(doc, "", explanation, theme::dim(), "", width);
    }
}

/// An RFC 3339 time in the person's local time: `14:30` today, `tomorrow 09:00`,
/// `yesterday 18:00`, else `Nov 14 09:00`. A time Fractal wrote unreadably is shown as written.
pub(crate) fn when<Tz: TimeZone>(at: &str, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    let Ok(parsed) = DateTime::parse_from_rfc3339(at) else {
        return at.to_owned();
    };
    let local = parsed.with_timezone(&now.timezone());
    let clock = local.format("%H:%M");
    match (local.date_naive() - now.date_naive()).num_days() {
        0 => clock.to_string(),
        1 => format!("tomorrow {clock}"),
        -1 => format!("yesterday {clock}"),
        _ => local.format("%b %-d %H:%M").to_string(),
    }
}

/// A refusal as the person reads it: an older daemon, or a pairing without the scope, says
/// what to do about it.
pub(crate) fn refusal(error: &ClientError) -> String {
    if error.is_missing_route() {
        return "this Fractal does not know availability yet: its daemon needs an update".into();
    }
    if let ClientError::Api(ErrorCode::Forbidden, message, _) = error
        && message.contains("not granted")
        && let Some(scope) = message.split('`').nth(1)
    {
        return format!("this device's pairing does not grant {scope}: pair the device again to allow it");
    }
    error.human()
}

pub(crate) async fn perform(client: &Client, want: Want) -> Done {
    match want {
        Want::Read(generation) => Done::Read(generation, read(client).await),
    }
}

async fn read(client: &Client) -> Result<Reading, String> {
    let people = client.availability_list(None).await.map_err(|error| refusal(&error))?.value.people;
    let Some(availability) = people.into_iter().next() else {
        return Err("Fractal names no person for this session; availability is a person's own".into());
    };
    let note = client
        .notes_list()
        .await
        .map_err(|error| refusal(&error))?
        .value
        .notes
        .into_iter()
        .find(|note| note.person == availability.person);
    Ok(Reading { availability, note })
}

impl super::Ui {
    /// A key on the focused Availability pane: `r` reads it again.
    pub(crate) fn availability_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        if self.focused_pane() != Some(super::pane::Pane::Availability) || !key.modifiers.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Char(letter) => self.availability_press(letter),
            _ => false,
        }
    }

    /// A key pressed or its button clicked.
    pub(crate) fn availability_press(&mut self, letter: char) -> bool {
        match letter {
            'r' => self.availability.refresh(),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-11-12T13:20:00+01:00").unwrap()
    }

    fn reading() -> Reading {
        Reading {
            availability: serde_json::from_value(serde_json::json!({
                "person": "person/robin",
                "status": "away",
                "status_source": "override",
                "status_until": "2026-11-12T13:30:00Z",
                "presence": "idle",
                "presence_observed_at": "2026-11-12T12:15:00Z",
                "interruptions": "urgent",
                "interruptions_until": "2026-11-13T08:00:00Z",
                "deadline_clock": "hold-routine",
                "deadline_clock_since": "2026-11-12T12:05:00Z",
                "deadline_clock_until": null,
            }))
            .unwrap(),
            note: Some(DirectiveNote {
                person: "person/robin".into(),
                author: "person/robin".into(),
                time: "2026-11-12T12:00:00Z".into(),
                text: "Heads down on the release; ask only about production.".into(),
                expires_at: Some("2026-11-12T17:00:00Z".into()),
                revision: "1".into(),
            }),
        }
    }

    fn lines(doc: &Doc) -> Vec<String> {
        doc.lines.iter().map(|line| text::plain(line).trim_end().to_owned()).collect()
    }

    #[test]
    fn reads_on_open_focus_and_reconnect_never_while_hidden() {
        let mut pane = Availability::default();
        assert!(pane.wanted(false).is_empty(), "nothing is read while the pane is not on screen");
        assert_eq!(pane.wanted(true), vec![Want::Read(0)], "opening reads");
        assert!(pane.wanted(true).is_empty(), "one read at a time, and none without a reason");
        pane.done(Done::Read(0, Ok(reading())));
        assert!(pane.wanted(true).is_empty(), "staying open reads nothing more");
        pane.refresh();
        assert!(pane.wanted(false).is_empty(), "focus back with the pane hidden waits for it");
        assert_eq!(pane.wanted(true), vec![Want::Read(0)], "and reads once it shows");
        pane.connected();
        assert_eq!(pane.wanted(true), vec![Want::Read(1)], "a reconnect reads again");
        let mut moved = reading();
        moved.availability.status = AvailabilityStatus::Available;
        pane.done(Done::Read(0, Ok(moved)));
        assert_eq!(pane.last, Some(reading()), "an answer from before the reconnect is not shown");
        pane.done(Done::Read(1, Err("the connection to Fractal broke".into())));
        assert_eq!(pane.last, Some(reading()), "a failed read keeps the last reading");
        assert!(lines(&pane.page(true, 80, &now())).contains(&"Could not read it again: the connection to Fractal broke".to_owned()));
    }

    #[test]
    fn times_are_local_and_named_by_day() {
        assert_eq!(when("2026-11-12T13:30:00Z", &now()), "14:30");
        assert_eq!(when("2026-11-13T08:00:00Z", &now()), "tomorrow 09:00");
        assert_eq!(when("2026-11-11T17:00:00Z", &now()), "yesterday 18:00");
        assert_eq!(when("2026-11-20T08:00:00Z", &now()), "Nov 20 09:00");
        assert_eq!(when("soon", &now()), "soon");
    }

    #[test]
    fn page_shows_every_axis_with_its_source_and_times() {
        let mut pane = Availability::default();
        pane.wanted(true);
        pane.done(Done::Read(0, Ok(reading())));
        assert_eq!(
            lines(&pane.page(true, 80, &now())),
            [
                "Availability  person/robin",
                "",
                "Status          away · set by you · until 14:30",
                "Presence        idle · reported 13:15",
                "Interruptions   urgent asks only · until tomorrow 09:00",
                "                Only an ask with priority critical interrupts you.",
                "Deadline clock  holding routine asks · since 13:05",
                "                Routine asks' deadlines wait and urgent asks keep running; the",
                "                time held is added to each deadline.",
                "Note            Heads down on the release; ask only about production. · until",
                "                18:00",
                "",
                " r  read again",
            ]
        );
    }

    #[test]
    fn defaults_say_what_they_are_and_unread_panes_say_why() {
        let mut pane = Availability::default();
        assert_eq!(lines(&pane.page(false, 80, &now()))[2], "Availability is read from a live Fractal; this demo has none.");
        assert_eq!(lines(&pane.page(true, 80, &now()))[2], "Reading your availability…");
        pane.wanted(true);
        pane.done(Done::Read(0, Err("this device's pairing does not grant read.projections: pair the device again to allow it".into())));
        let shown = lines(&pane.page(true, 80, &now()));
        assert!(shown[2].starts_with("Could not read your availability: this device's pairing"), "{shown:?}");
        assert_eq!(shown.last().map(String::as_str), Some(" r  try again"));
        let mut quiet = reading();
        quiet.availability = serde_json::from_value(serde_json::json!({
            "person": "person/robin", "status": "available", "status_source": "presence", "status_until": null,
            "presence": null, "presence_observed_at": null, "interruptions": "all", "interruptions_until": null,
            "deadline_clock": "run", "deadline_clock_since": null, "deadline_clock_until": null,
        }))
        .unwrap();
        quiet.note = None;
        pane.done(Done::Read(0, Ok(quiet)));
        assert_eq!(
            lines(&pane.page(true, 80, &now()))[2..7],
            [
                "Status          available · from presence",
                "Interruptions   all asks",
                "Deadline clock  running",
                "Note            none",
                "",
            ]
        );
    }

    #[test]
    fn an_unreported_source_reads_available_and_unknown_words_stay_readable() {
        let mut pane = Availability::default();
        pane.wanted(true);
        let mut unreported = reading();
        unreported.availability = serde_json::from_value(serde_json::json!({
            "person": "person/robin", "status": "available", "status_source": "unreported", "status_until": null,
            "presence": "unreported", "presence_observed_at": null, "interruptions": "all", "interruptions_until": null,
            "deadline_clock": "run", "deadline_clock_since": null, "deadline_clock_until": null,
        }))
        .unwrap();
        unreported.note = None;
        pane.done(Done::Read(0, Ok(unreported)));
        assert_eq!(
            lines(&pane.page(true, 80, &now()))[2..7],
            [
                "Status          available · no device reporting",
                "                No device has reported for 24 hours; agents ask as usual.",
                "Presence        unreported",
                "Interruptions   all asks",
                "Deadline clock  running",
            ]
        );
        let mut newer = reading();
        newer.availability = serde_json::from_value(serde_json::json!({
            "person": "person/robin", "status": "busy", "status_source": "calendar", "status_until": "2026-11-12T13:30:00Z",
            "presence": "in-call", "presence_observed_at": "2026-11-12T12:15:00Z", "interruptions": "vip", "interruptions_until": null,
            "deadline_clock": "hold-weekend", "deadline_clock_since": null, "deadline_clock_until": null,
        }))
        .unwrap();
        newer.note = None;
        pane.done(Done::Read(0, Ok(newer)));
        assert_eq!(
            lines(&pane.page(true, 80, &now()))[2..11],
            [
                "Status          unknown · from an unknown source · until 14:30",
                "                A word this version does not know; updating shows it.",
                "Presence        unknown · reported 13:15",
                "                A word this version does not know; updating shows it.",
                "Interruptions   unknown",
                "                A word this version does not know; updating shows it.",
                "Deadline clock  unknown",
                "                A word this version does not know; updating shows it.",
                "Note            none",
            ]
        );
    }

    #[test]
    fn pane_renders_through_the_terminal_ui_and_r_reads_again() {
        use crate::ui::{Ui, demo, pane::Pane};
        use crossterm::event::{KeyCode, KeyEvent};
        use ratatui::{Terminal, backend::TestBackend};
        let mut ui = Ui::new(demo::test_world());
        ui.live = true;
        ui.open_in_glass(Pane::Availability, crate::ui::glass::Open::Tab);
        assert_eq!(ui.availability.wanted(true), vec![Want::Read(0)]);
        ui.availability.done(Done::Read(0, Ok(reading())));
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|frame| ui.render_pane(frame, &Pane::Availability)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen = (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n");
        println!("{screen}");
        for expected in ["Availability  person/robin", "Status          away · set by you · until", "Deadline clock  holding routine asks", "r  read again"] {
            assert!(screen.contains(expected), "{expected:?} in\n{screen}");
        }
        assert!(ui.availability.wanted(true).is_empty());
        ui.key(KeyEvent::from(KeyCode::Char('r')));
        assert_eq!(ui.availability.wanted(true), vec![Want::Read(0)], "r reads again");
    }
}
