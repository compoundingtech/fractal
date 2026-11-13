//! The person's own availability and directive note in one pane: whether they are available,
//! which asks may interrupt them, whether their asks' deadlines are held, and the note agents
//! read. No collection stream carries availability, so the pane reads `availability.list` and
//! `notes.list` when it opens, when the terminal regains focus, after a reconnect and on `r`,
//! never on a timer. Its keys set or clear each axis through `availability.set`, and the pane
//! shows what Fractal answered.

use super::doc::{Doc, DocExt, Hit};
use super::{text, theme};
use chrono::{DateTime, NaiveDate, NaiveTime, SecondsFormat, TimeDelta, TimeZone, Utc};
use fractal_client::{
    AvailabilityDeadlineClock, AvailabilityDeadlineClockChange, AvailabilityInterruptions, AvailabilityInterruptionsChange,
    AvailabilityPresence, AvailabilitySet, AvailabilityStatus, AvailabilityStatusChange, AvailabilityStatusSource, Client,
    ClientError, DirectiveNote, ErrorCode, PersonAvailability,
};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;
use std::fmt::Display;

/// Labels are padded to this many cells, so the values line up.
const LABEL: usize = 16;
/// The keys a button presses that are not letters.
pub(crate) const ENTER: char = '\n';
pub(crate) const ESC: char = '\u{1b}';
/// Until when a value holds, by key; Enter keeps it until changed.
const PRESETS: [(char, &str, Preset); 4] = [
    ('1', "30 minutes", Preset::HalfHour),
    ('2', "1 hour", Preset::Hour),
    ('3', "2 hours", Preset::TwoHours),
    ('4', "tomorrow 09:00", Preset::TomorrowNine),
];

/// Under a word a newer Fractal sent, which this version does not know and shows as `unknown`.
const UNKNOWN: &str = "A word this version does not know; updating shows it.";

/// What Fractal last said: the person's effective availability and their current note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) availability: PersonAvailability,
    pub(crate) note: Option<DirectiveNote>,
}

/// What the person can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Status,
    Interruptions,
    DeadlineClock,
}

impl Axis {
    fn title(self) -> &'static str {
        match self {
            Axis::Status => "status",
            Axis::Interruptions => "interruptions",
            Axis::DeadlineClock => "deadline clock",
        }
    }

    /// Each value's key, label and word; `None` returns the axis to its default.
    fn choices(self) -> &'static [(char, &'static str, Option<Word>)] {
        match self {
            Axis::Status => &[
                ('1', "away", Some(Word::Status(AvailabilityStatus::Away))),
                ('2', "available", Some(Word::Status(AvailabilityStatus::Available))),
                ('0', "from presence", None),
            ],
            Axis::Interruptions => &[
                ('1', "urgent asks only", Some(Word::Interruptions(AvailabilityInterruptions::Urgent))),
                ('2', "none", Some(Word::Interruptions(AvailabilityInterruptions::None))),
                ('0', "all asks", None),
            ],
            Axis::DeadlineClock => &[
                ('1', "hold routine asks", Some(Word::DeadlineClock(AvailabilityDeadlineClock::HoldRoutine))),
                ('2', "hold every ask", Some(Word::DeadlineClock(AvailabilityDeadlineClock::HoldAll))),
                ('0', "run", None),
            ],
        }
    }
}

/// A value the person can choose, as the client names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Status(AvailabilityStatus),
    Interruptions(AvailabilityInterruptions),
    DeadlineClock(AvailabilityDeadlineClock),
}

impl Word {
    /// The word on the wire.
    fn as_str(self) -> &'static str {
        match self {
            Word::Status(word) => word.as_str(),
            Word::Interruptions(word) => word.as_str(),
            Word::DeadlineClock(word) => word.as_str(),
        }
    }
}

/// A preset end: from now, or tomorrow at 09:00 local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preset {
    HalfHour,
    Hour,
    TwoHours,
    TomorrowNine,
}

/// What the person is choosing: an axis's value, then until when it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Editor {
    Value(Axis),
    Until { axis: Axis, value: String, label: String },
}

/// One change, sent alone: each answer is the whole availability after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Availability(AvailabilitySet),
}

/// A request the live loop sends for the pane, answered with a [`Done`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    /// Read availability and the note; the generation fences an answer that a reconnect or a
    /// write has made old.
    Read(u64),
    Write(Write),
}

#[derive(Debug)]
pub(crate) enum Wrote {
    Availability(PersonAvailability),
}

#[derive(Debug)]
pub(crate) enum Done {
    Read(u64, Result<Reading, String>),
    Wrote(Result<Wrote, String>),
}

/// What a key did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pressed {
    Ignored,
    Done,
    Say(String),
}

#[derive(Default)]
pub(crate) struct Availability {
    /// Whether the pane was on screen on the last pass: opening it reads again.
    shown: bool,
    /// Something may have changed since the last read: focus came back, a reconnect, `r`.
    stale: bool,
    /// The generation of the read in flight.
    reading: Option<u64>,
    /// Bumped by a reconnect and by each write, so an older read is never shown.
    generation: u64,
    last: Option<Reading>,
    /// Why the last read failed: said beside an older reading, or alone.
    failed: Option<String>,
    editor: Option<Editor>,
    /// Changes not sent yet, one at a time in the order they were made.
    writes: VecDeque<Write>,
    writing: bool,
    /// Why Fractal did not take the last change.
    refused: Option<String>,
}

impl Availability {
    /// The requests the pane needs now, given whether it is on screen.
    pub(crate) fn wanted(&mut self, shown: bool) -> Vec<Want> {
        self.stale |= shown && !self.shown;
        self.shown = shown;
        let mut wants = Vec::new();
        if !self.writing
            && let Some(write) = self.writes.pop_front()
        {
            self.writing = true;
            self.generation += 1;
            wants.push(Want::Write(write));
        }
        if shown && self.stale && self.reading.is_none() && !self.writing {
            self.stale = false;
            self.reading = Some(self.generation);
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
        self.reading = None;
        self.stale = true;
    }

    pub(crate) fn done(&mut self, done: Done) {
        match done {
            Done::Read(generation, outcome) => {
                if self.reading == Some(generation) {
                    self.reading = None;
                }
                if generation != self.generation {
                    return;
                }
                match outcome {
                    Ok(reading) => {
                        self.last = Some(reading);
                        self.failed = None;
                    }
                    Err(why) => self.failed = Some(why),
                }
            }
            Done::Wrote(outcome) => {
                self.writing = false;
                match outcome {
                    Ok(Wrote::Availability(availability)) => {
                        self.refused = None;
                        if let Some(last) = self.last.as_mut() {
                            last.availability = availability;
                        }
                    }
                    // Whatever happened, the pane shows Fractal's word on it next.
                    Err(why) => {
                        self.refused = Some(why);
                        self.stale = true;
                    }
                }
            }
        }
    }

    pub(crate) fn editing(&self) -> bool {
        self.editor.is_some()
    }

    /// A key, or the button that presses it. `s`, `i` and `d` choose what to change, a digit
    /// its value or end, Enter keeps a value until changed, Esc puts the choice away.
    pub(crate) fn press<Tz: TimeZone>(&mut self, key: char, now: &DateTime<Tz>) -> Pressed {
        let Some(editor) = self.editor.clone() else {
            let axis = match key {
                'r' => {
                    self.refresh();
                    return Pressed::Done;
                }
                's' => Axis::Status,
                'i' => Axis::Interruptions,
                'd' => Axis::DeadlineClock,
                _ => return Pressed::Ignored,
            };
            if self.last.is_none() {
                return Pressed::Say("Your availability is not read yet: r reads it".into());
            }
            self.editor = Some(Editor::Value(axis));
            return Pressed::Done;
        };
        if key == ESC {
            self.editor = None;
            return Pressed::Done;
        }
        match editor {
            Editor::Value(axis) => match axis.choices().iter().find(|(choice, ..)| *choice == key) {
                Some((_, label, Some(value))) => {
                    self.editor = Some(Editor::Until { axis, value: value.as_str().into(), label: (*label).into() });
                }
                Some((_, _, None)) => self.queue(axis, None, None),
                None => return Pressed::Ignored,
            },
            Editor::Until { axis, value, .. } => {
                let until = match key {
                    ENTER => None,
                    key => match PRESETS.iter().find(|(preset, ..)| *preset == key) {
                        Some((_, _, preset)) => Some(preset_until(*preset, now)),
                        None => return Pressed::Ignored,
                    },
                };
                self.queue(axis, Some(value), until);
            }
        }
        Pressed::Done
    }

    /// Send one change: `value: None` returns the axis to its default.
    fn queue(&mut self, axis: Axis, value: Option<String>, until: Option<DateTime<Utc>>) {
        let until = until.map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true));
        let write = match axis {
            Axis::Status => Write::Availability(AvailabilitySet {
                status: Some(AvailabilityStatusChange { value, until }),
                ..AvailabilitySet::default()
            }),
            Axis::Interruptions => Write::Availability(AvailabilitySet {
                interruptions: Some(AvailabilityInterruptionsChange {
                    value: value.unwrap_or_else(|| AvailabilityInterruptions::All.as_str().into()),
                    until,
                }),
                ..AvailabilitySet::default()
            }),
            Axis::DeadlineClock => Write::Availability(AvailabilitySet {
                deadline_clock: Some(AvailabilityDeadlineClockChange {
                    value: value.unwrap_or_else(|| AvailabilityDeadlineClock::Run.as_str().into()),
                    until,
                }),
                ..AvailabilitySet::default()
            }),
        };
        self.editor = None;
        self.refused = None;
        self.writes.push_back(write);
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
        self.editor_section(&mut doc, width);
        if self.writing || !self.writes.is_empty() {
            doc.blank();
            doc.line(Line::from(Span::styled("Saving…", theme::dim())));
        }
        if let Some(why) = &self.refused {
            doc.blank();
            doc.wrap(&[text::run(format!("Not changed: {why}"), theme::fg(theme::RED))], width);
        }
        doc
    }

    /// The keys for what can be changed now.
    fn editor_section(&self, doc: &mut Doc, width: usize) {
        let accent = |key: &'static str, label: &'static str, press: char| (key, label, Hit::Availability(press), theme::ACCENT);
        let cancel = ("Esc", "cancel", Hit::Availability(ESC), theme::OVERLAY1);
        match &self.editor {
            None => doc.buttons_fit(
                &[
                    accent("s", "status", 's'),
                    accent("i", "interruptions", 'i'),
                    accent("d", "deadline clock", 'd'),
                    accent("r", "read again", 'r'),
                ],
                width,
            ),
            Some(Editor::Value(axis)) => {
                doc.section(&format!("Set {}", axis.title()), None, width);
                let keys = axis.choices().iter().map(|(key, _, _)| key.to_string()).collect::<Vec<_>>();
                let mut buttons = axis
                    .choices()
                    .iter()
                    .zip(&keys)
                    .map(|((key, label, _), shown)| (shown.as_str(), *label, Hit::Availability(*key), theme::ACCENT))
                    .collect::<Vec<_>>();
                buttons.push(cancel);
                doc.buttons_fit(&buttons, width);
            }
            Some(Editor::Until { label, .. }) => {
                doc.section(&format!("{label} until"), None, width);
                let mut buttons = PRESETS
                    .iter()
                    .zip(["1", "2", "3", "4"])
                    .map(|((key, label, _), shown)| (shown, *label, Hit::Availability(*key), theme::ACCENT))
                    .collect::<Vec<_>>();
                buttons.push(accent("Enter", "until you change it", ENTER));
                buttons.push(cancel);
                doc.buttons_fit(&buttons, width);
            }
        }
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

/// The end a preset names: from `now`, or literally tomorrow at 09:00 in `now`'s zone.
pub(crate) fn preset_until<Tz: TimeZone>(preset: Preset, now: &DateTime<Tz>) -> DateTime<Utc> {
    let utc = now.with_timezone(&Utc);
    match preset {
        Preset::HalfHour => utc + TimeDelta::minutes(30),
        Preset::Hour => utc + TimeDelta::hours(1),
        Preset::TwoHours => utc + TimeDelta::hours(2),
        Preset::TomorrowNine => at_local(now, tomorrow(now), NaiveTime::from_hms_opt(9, 0, 0).expect("09:00 is a time")),
    }
}

fn tomorrow<Tz: TimeZone>(now: &DateTime<Tz>) -> NaiveDate {
    now.date_naive().succ_opt().expect("a day after today")
}

/// A wall-clock time on a day in `now`'s zone; in a gap a clock change skipped, the first
/// quarter hour after it.
fn at_local<Tz: TimeZone>(now: &DateTime<Tz>, day: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let zone = now.timezone();
    let mut local = day.and_time(time);
    loop {
        if let Some(at) = zone.from_local_datetime(&local).earliest() {
            return at.with_timezone(&Utc);
        }
        local += TimeDelta::minutes(15);
    }
}

/// A refusal as the person reads it: an older daemon, a pairing without the scope, or an
/// agent's session says what to do about it.
pub(crate) fn refusal(error: &ClientError) -> String {
    if error.is_missing_route() {
        return "this Fractal does not know availability yet: its daemon needs an update".into();
    }
    if let ClientError::Api(ErrorCode::Forbidden, message, _) = error {
        if message.contains("agents are refused") {
            return "only a person changes their own availability and note, and this session is an agent's".into();
        }
        if message.contains("not granted")
            && let Some(scope) = message.split('`').nth(1)
        {
            return format!("this device's pairing does not grant {scope}: pair the device again to allow it");
        }
    }
    error.human()
}

pub(crate) async fn perform(client: &Client, want: Want) -> Done {
    match want {
        Want::Read(generation) => Done::Read(generation, read(client).await),
        Want::Write(Write::Availability(request)) => Done::Wrote(
            client.availability_set(&request).await.map(|reply| Wrote::Availability(reply.value)).map_err(|error| refusal(&error)),
        ),
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
    /// A key on the focused Availability pane.
    pub(crate) fn availability_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        if self.focused_pane() != Some(super::pane::Pane::Availability) {
            return false;
        }
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Esc if plain && self.availability.editing() => self.availability_press(ESC),
            KeyCode::Enter if plain && self.availability.editing() => self.availability_press(ENTER),
            KeyCode::Char(letter) if plain => self.availability_press(letter),
            _ => false,
        }
    }

    /// A key pressed or its button clicked.
    pub(crate) fn availability_press(&mut self, key: char) -> bool {
        match self.availability.press(key, &chrono::Local::now()) {
            Pressed::Ignored => false,
            Pressed::Done => true,
            Pressed::Say(message) => {
                self.flash(message);
                true
            }
        }
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
                " s  status    i  interruptions    d  deadline clock    r  read again",
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

    fn loaded() -> Availability {
        let mut pane = Availability::default();
        pane.wanted(true);
        pane.done(Done::Read(0, Ok(reading())));
        pane
    }

    fn at(time: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
    }

    /// The changes the live loop would send now.
    fn sent(pane: &mut Availability) -> Vec<Write> {
        pane.wanted(true)
            .into_iter()
            .filter_map(|want| match want {
                Want::Write(write) => Some(write),
                Want::Read(_) => None,
            })
            .collect()
    }

    fn keys(pane: &mut Availability, keys: &str) {
        for key in keys.chars() {
            assert_eq!(pane.press(key, &now()), Pressed::Done, "{key:?}");
        }
    }

    fn set(change: AvailabilitySet) -> Write {
        Write::Availability(change)
    }

    #[test]
    fn presets_end_from_now_or_literally_tomorrow_at_nine() {
        assert_eq!(preset_until(Preset::HalfHour, &now()), at("2026-11-12T12:50:00Z"));
        assert_eq!(preset_until(Preset::Hour, &now()), at("2026-11-12T13:20:00Z"));
        assert_eq!(preset_until(Preset::TwoHours, &now()), at("2026-11-12T14:20:00Z"));
        assert_eq!(preset_until(Preset::TomorrowNine, &now()), at("2026-11-13T08:00:00Z"));
        let early = DateTime::parse_from_rfc3339("2026-11-12T06:00:00+01:00").unwrap();
        assert_eq!(preset_until(Preset::TomorrowNine, &early), at("2026-11-13T08:00:00Z"), "tomorrow, even before nine today");
        let late = DateTime::parse_from_rfc3339("2026-11-12T23:59:00-08:00").unwrap();
        assert_eq!(preset_until(Preset::TomorrowNine, &late), at("2026-11-13T17:00:00Z"), "the local day decides");
    }

    #[test]
    fn keys_choose_a_value_then_its_end_and_changes_go_one_at_a_time() {
        assert_eq!(Availability::default().press('s', &now()), Pressed::Say("Your availability is not read yet: r reads it".into()));
        let mut pane = loaded();
        keys(&mut pane, "s12");
        keys(&mut pane, "i0");
        keys(&mut pane, "d2\n");
        let away = AvailabilityStatusChange {
            value: Some(AvailabilityStatus::Away.as_str().into()),
            until: Some("2026-11-12T13:20:00.000Z".into()),
        };
        assert_eq!(sent(&mut pane), [set(AvailabilitySet { status: Some(away), ..AvailabilitySet::default() })], "away for an hour");
        assert!(sent(&mut pane).is_empty(), "the next change waits for the answer");
        assert!(lines(&pane.page(true, 80, &now())).contains(&"Saving…".to_owned()));
        let mut answer = reading().availability;
        answer.status_until = Some("2026-11-12T13:20:00.000Z".into());
        pane.done(Done::Wrote(Ok(Wrote::Availability(answer.clone()))));
        assert_eq!(pane.last.as_ref().map(|last| &last.availability), Some(&answer), "the answer is shown");
        let all = AvailabilityInterruptionsChange { value: AvailabilityInterruptions::All.as_str().into(), until: None };
        assert_eq!(sent(&mut pane), [set(AvailabilitySet { interruptions: Some(all), ..AvailabilitySet::default() })], "0 clears");
        pane.done(Done::Wrote(Err("this device's pairing does not grant write.availability: pair the device again to allow it".into())));
        let shown = lines(&pane.page(true, 80, &now()));
        assert!(shown.iter().any(|line| line.starts_with("Not changed: this device's pairing does not grant write.availability")), "{shown:?}");
        let held = AvailabilityDeadlineClockChange { value: AvailabilityDeadlineClock::HoldAll.as_str().into(), until: None };
        assert_eq!(sent(&mut pane), [set(AvailabilitySet { deadline_clock: Some(held), ..AvailabilitySet::default() })], "Enter: until changed");
        pane.done(Done::Wrote(Ok(Wrote::Availability(answer))));
        assert_eq!(pane.wanted(true), [Want::Read(3)], "after a refusal Fractal's word is read again, newer than every write");
        pane.done(Done::Read(2, Ok(reading())));
        assert!(pane.reading.is_some(), "an answer older than a write is not taken");
        keys(&mut pane, "s\u{1b}");
        assert!(!pane.editing() && sent(&mut pane).is_empty(), "Esc sends nothing");
    }

    #[test]
    fn the_editor_shows_its_choices() {
        let mut pane = loaded();
        let tail = |pane: &Availability| lines(&pane.page(true, 80, &now()))[12..].to_vec();
        keys(&mut pane, "i");
        assert_eq!(
            tail(&pane),
            [
                "Set interruptions ──────────────────────────────────────────────────────────────",
                " 1  urgent asks only    2  none    0  all asks    Esc  cancel",
            ]
        );
        keys(&mut pane, "1");
        assert_eq!(
            tail(&pane),
            [
                "urgent asks only until ─────────────────────────────────────────────────────────",
                " 1  30 minutes    2  1 hour    3  2 hours    4  tomorrow 09:00",
                " Enter  until you change it    Esc  cancel",
            ]
        );
    }

    #[test]
    fn terminal_keys_and_clicks_reach_the_pane() {
        use crate::ui::{Ui, demo, pane::Pane};
        use crossterm::event::{KeyCode, KeyEvent};
        let mut ui = Ui::new(demo::test_world());
        ui.live = true;
        ui.open_in_glass(Pane::Availability, crate::ui::glass::Open::Tab);
        ui.availability.wanted(true);
        ui.availability.done(Done::Read(0, Ok(reading())));
        ui.key(KeyEvent::from(KeyCode::Char('d')));
        ui.click(Hit::Availability('1'));
        ui.key(KeyEvent::from(KeyCode::Char('4')));
        let [Write::Availability(change)] = &sent(&mut ui.availability)[..] else { panic!("one change") };
        let clock = change.deadline_clock.as_ref().expect("the deadline clock");
        assert_eq!(clock.value, AvailabilityDeadlineClock::HoldRoutine.as_str());
        let until = DateTime::parse_from_rfc3339(clock.until.as_deref().expect("an end")).unwrap();
        assert_eq!(until.with_timezone(&chrono::Local).format("%H:%M").to_string(), "09:00", "tomorrow 09:00 local");
        ui.key(KeyEvent::from(KeyCode::Char('s')));
        ui.key(KeyEvent::from(KeyCode::Esc));
        assert!(!ui.availability.editing(), "Esc puts the choice away");
    }
}
