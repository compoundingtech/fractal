//! What each tab lists and what its detail pane says.
//!
//! A screen produces sidebar items and a detail `Doc`. It never draws cells itself; the
//! shell owns layout, scrolling, selection and clicks.

use super::doc::{Doc, DocExt, Hit, meter};
use super::text::{self, run};
use super::theme;
use super::view::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// What Home says when nothing needs the person; setup's first-run test waits for it.
pub const HOME_IDLE_TEXT: &str = "No alerts right now.";
/// What Home and the alerts list say instead of "none" while the alerts they would show may be
/// missing or old: st is catching up with a peer, or stopped serving the list or its live stream.
pub const HOME_UNKNOWN_TEXT: &str = "No alerts known — this view is out of date.";

fn human_mission_label(mission: &Mission) -> String {
    let mut label = mission.label();
    if let Some(state) = label.strip_suffix(" · st does not say") {
        let end = state.len();
        label.replace_range(end.., " · Fractal does not say");
    }
    label
}

/// Whether the alerts shown (or their absence) can be out of date, and why.
fn alerts_out_of_date(world: &World) -> Option<String> {
    let stale: Vec<&str> = world
        .stale
        .iter()
        .map(String::as_str)
        .filter(|why| why.starts_with("Attention") || why.starts_with("live stream"))
        .collect();
    if world.behind.is_empty() && stale.is_empty() {
        return None;
    }
    let mut why = Vec::new();
    if !world.behind.is_empty() {
        why.push(format!("catching up: {}", world.behind.join("; ")));
    }
    why.extend(stale.iter().map(|why| (*why).to_owned()));
    Some(why.join("; "))
}

pub enum Item {
    Header {
        title: String,
        count: usize,
        color: Color,
    },
    Row {
        index: usize,
        first: Vec<Span<'static>>,
        right: Vec<Span<'static>>,
        second: Vec<Span<'static>>,
        /// Lines beneath the row that belong to it, such as an agent's subagents. They select
        /// and click as the row: nothing in them is selectable on its own.
        children: Vec<Line<'static>>,
    },
    Note(Line<'static>),
    /// A folder line in a tree: no spacing around it.
    Folder(Line<'static>),
}

/// Sidebar rows. `index` in each row is the position in the tab's selectable order.
pub struct Listing {
    pub items: Vec<Item>,
    /// Selectable ids in display order.
    pub ids: Vec<String>,
    pub state: ListState,
    pub legend: Vec<Line<'static>>,
}

pub enum ListState {
    Loading(&'static str),
    Empty(&'static str),
    Failed(String),
    Ready,
}

fn state_of<T>(load: &Load<Vec<T>>, loading: &'static str, empty: &'static str) -> ListState {
    match load {
        Load::Loading => ListState::Loading(loading),
        Load::Failed(error) => ListState::Failed(error.clone()),
        Load::Ready(items) if items.is_empty() => ListState::Empty(empty),
        Load::Ready(_) => ListState::Ready,
    }
}

// ------------------------------------------------------------------- glyphs

pub fn agent_glyph(state: AgentState, spinner: &'static str) -> (&'static str, Color) {
    match state {
        AgentState::NeedsYou => ("◆", theme::PERSON),
        AgentState::NeedsLogin => ("⚿", theme::PERSON),
        AgentState::Fault => ("✕", theme::FAULT),
        AgentState::Working => (spinner, theme::WORKING),
        AgentState::Compacting => ("◐", theme::SAPPHIRE),
        AgentState::Idle => ("●", theme::IDLE),
        AgentState::Starting => ("◌", theme::WAITING),
        AgentState::Down => ("⊘", theme::FAULT),
        AgentState::Stopped | AgentState::Retired => ("○", theme::QUIET),
        AgentState::Unknown => ("?", theme::QUIET),
    }
}

/// The word an agent row shows: st's state word, or, from an st too old to serve one, the state
/// st reports, marked so.
pub fn agent_shown_word(agent: &Agent) -> String {
    match &agent.reported {
        Some(state) => format!("{state} · older Fractal"),
        None => agent_word(agent.state).to_owned(),
    }
}

pub fn agent_word(state: AgentState) -> &'static str {
    match state {
        AgentState::NeedsYou => "needs you",
        AgentState::NeedsLogin => "needs login",
        AgentState::Fault => "broken",
        AgentState::Working => "working",
        AgentState::Compacting => "compacting",
        AgentState::Idle => "idle",
        AgentState::Starting => "starting",
        AgentState::Down => "down",
        AgentState::Stopped => "stopped",
        AgentState::Retired => "retired",
        AgentState::Unknown => "not managed",
    }
}

pub fn word_style(word: Word, spinner: &'static str) -> (&'static str, Color) {
    match word {
        Word::Decision => ("◆", theme::PERSON),
        Word::Stalled => ("▲", theme::FAULT),
        Word::Unstaffed => ("◇", theme::WAITING),
        Word::Unclaimed => ("◇", theme::OVERLAY1),
        Word::Queued => ("◌", theme::OVERLAY1),
        Word::Working => (spinner, theme::WORKING),
        Word::Watching => ("◉", theme::SAPPHIRE),
        Word::Held => ("◐", theme::SAPPHIRE),
        Word::Idle => ("●", theme::IDLE),
        Word::Done => ("✓", theme::DONE),
        Word::Failed => ("✕", theme::FAULT),
        Word::Cancelled => ("⊘", theme::QUIET),
        Word::NotStarted => ("○", theme::QUIET),
        Word::Unknown => ("?", theme::QUIET),
    }
}

fn step_style(state: StepState, spinner: &'static str) -> (&'static str, Color, &'static str) {
    fractal_mission_ui::step_style(
        state,
        spinner,
        &fractal_mission_ui::Palette::default(),
        &fractal_mission_ui::Glyphs::default(),
    )
}

pub fn attention_style(kind: &AttentionKind) -> (&'static str, Color) {
    match kind {
        AttentionKind::Review { .. }
        | AttentionKind::Feedback { .. }
        | AttentionKind::Launch { .. }
        | AttentionKind::Revision { .. }
        | AttentionKind::Request { .. }
        | AttentionKind::Prompt { .. } => ("◆", theme::PERSON),
        AttentionKind::Login { .. } => ("⚿", theme::PERSON),
        AttentionKind::Fault { .. } => ("✕", theme::FAULT),
        AttentionKind::Message { .. } => ("✉", theme::SAPPHIRE),
        AttentionKind::Update { .. } => ("✦", theme::GREEN),
        AttentionKind::Unplaced { .. } => ("?", theme::OVERLAY1),
    }
}

/// The answer an unplaced card offers: the one st lists for the row (a custom reply, else a
/// request's), or none.
pub(super) fn unplaced_answer(actions: &[String]) -> Option<&'static str> {
    ["custom.reply", "work.done"]
        .into_iter()
        .find(|answer| actions.iter().any(|action| action == answer))
}

fn harness_color(harness: Harness) -> Color {
    match harness {
        Harness::Claude => theme::PEACH,
        Harness::Codex => theme::BLUE,
        Harness::Omp => theme::TEAL,
        Harness::Pi => theme::LAVENDER,
        Harness::Unknown => theme::OVERLAY0,
    }
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn legend(entries: &[(&str, Color, &str)]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    for (index, (glyph, color, word)) in entries.iter().enumerate() {
        if index > 0 && index % 3 == 0 {
            lines.push(Line::from(std::mem::take(&mut spans)));
        }
        spans.push(span(format!("{glyph} "), theme::fg(*color)));
        spans.push(span(format!("{word:<10}"), theme::dim()));
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

// --------------------------------------------------------------------- home

pub fn home_list(world: &World, closed: &std::collections::HashSet<String>) -> Listing {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    let mut attention = world
        .attention
        .items()
        .iter()
        .filter(|item| !item.snoozed(world.at))
        .collect::<Vec<_>>();
    // What st closed while it was shown stays, in a section of its own below what needs you, so
    // the two are never mistaken for each other (Nathan, 2026-10-07: "2 need you", 4 listed).
    attention.sort_by_key(|item| (closed.contains(&item.id), item.tier));
    let mut current = None;
    for item in attention {
        let is_closed = closed.contains(&item.id);
        if current != Some((is_closed, item.tier)) {
            current = Some((is_closed, item.tier));
            let count = world
                .attention
                .items()
                .iter()
                .filter(|other| {
                    !other.snoozed(world.at)
                        && closed.contains(&other.id) == is_closed
                        && (is_closed || other.tier == item.tier)
                })
                .count();
            items.push(Item::Header {
                title: if is_closed {
                    "Recently closed: x clears each".into()
                } else {
                    item.tier.title().into()
                },
                count,
                color: if !is_closed && item.tier == Tier::Stopped {
                    theme::PERSON
                } else {
                    theme::OVERLAY1
                },
            });
        }
        let (glyph, color) = attention_style(&item.kind);
        let mut second = vec![span("   ", theme::dim())];
        if let Some(waiting) = &item.waiting {
            second.push(span(format!("{waiting} · "), theme::dim()));
        }
        second.push(span(format!("waited {}", item.age), theme::dim()));
        items.push(Item::Row {
            index: ids.len(),
            first: vec![
                span(format!(" {glyph} "), theme::fg(color)),
                span(format!("{:<9}", item.kind.word()), theme::fg(color)),
                span(item.title.clone(), theme::bold()),
            ],
            right: vec![],
            second,
            children: Vec::new(),
        });
        ids.push(item.id.clone());
    }
    let later = world
        .attention
        .items()
        .iter()
        .filter(|item| item.snoozed(world.at))
        .count();
    if later > 0 {
        items.push(Item::Note(Line::from(span(
            format!(" {later} put off until later"),
            theme::dim(),
        ))));
    }
    Listing {
        items,
        ids,
        state: state_of(
            &world.attention,
            "Checking for alerts…",
            if alerts_out_of_date(world).is_some() {
                "No alerts known — out of date."
            } else {
                "No alerts."
            },
        ),
        legend: legend(&[
            ("◆", theme::PERSON, "decide"),
            ("✕", theme::FAULT, "fault"),
            ("✉", theme::SAPPHIRE, "message"),
        ]),
    }
}

pub struct Drafts<'a> {
    pub text: Option<&'a str>,
    /// Where typing goes in `text`, as a byte offset.
    pub cursor: usize,
    pub editing: bool,
    pub confirm: Option<char>,
    /// The highlighted row on a structured request: a named answer, or `answers.len()` for the
    /// custom answer entry. Enter acts on it.
    pub answering: Option<usize>,
    /// The named answer (its id) the open note belongs to. Independent of the highlight: moving
    /// the highlight keeps the note bound to the answer it was written for.
    pub note_answer: Option<String>,
    /// Every named answer (by id) that has a saved note on this request.
    pub noted_answers: Vec<String>,
    /// Why the last answer to this request was not sent; stays until the next attempt.
    pub answer_error: Option<&'a str>,
    /// "Chat about this": who it goes to, the draft, and the thread so far.
    pub chat: Option<Chat<'a>>,
    /// A prompt's form: its mark, whether it has the keys, and a question's choices.
    pub form: super::alerts::FormState<'a>,
}

pub struct Chat<'a> {
    pub to: String,
    pub text: &'a str,
    pub cursor: usize,
    pub editing: bool,
    pub thread: Vec<Line<'static>>,
}

/// A clickable reference to a graph subject; clicking it opens a popover.
pub fn link(doc: &mut Doc, prefix: &str, label: &str, subject: &str) {
    doc.targets.push(super::doc::Target {
        line: doc.lines.len(),
        column: text::width(prefix) as u16,
        width: text::width(label) as u16,
        hit: Hit::Peek(subject.to_owned()),
    });
    doc.line(Line::from(vec![
        span(prefix.to_owned(), theme::dim()),
        span(
            label.to_owned(),
            theme::fg(theme::BLUE).add_modifier(Modifier::UNDERLINED),
        ),
    ]));
}

fn text_box(doc: &mut Doc, title: &str, drafts: &Drafts<'_>, placeholder: &str, width: usize) {
    let mut inner = Doc::new();
    let body = drafts.text.unwrap_or("");
    if body.is_empty() && !drafts.editing {
        inner.line(Line::from(span(placeholder.to_owned(), theme::dim())));
    } else {
        let at = drafts.editing.then_some(drafts.cursor);
        for runs in super::edit::lines(body, at, theme::text()) {
            inner.lines(text::wrap(&runs, width.saturating_sub(4), &[], &[], None));
        }
        // What is wanted stays in view under the cursor until something is typed.
        if body.is_empty() && !placeholder.is_empty() {
            inner.lines(text::wrap(
                &[text::run(placeholder.to_owned(), theme::dim())],
                width.saturating_sub(4),
                &[],
                &[],
                None,
            ));
        }
    }
    let start = doc.lines.len();
    doc.card(
        title,
        if drafts.editing {
            theme::ACCENT
        } else {
            theme::OVERLAY1
        },
        false,
        inner,
        width,
    );
    let end = doc.lines.len();
    for line in start..end {
        doc.targets.push(super::doc::Target {
            line,
            column: 0,
            width: width as u16,
            hit: Hit::Composer,
        });
    }
}

/// A request whose text is (or ends in) a JSON report: the words before it, then the fields that
/// say what happened, then the rest folded to a few. `None` when there is no JSON object.
/// A structured request (#1010) as a person reads it: the asker's recommendation first, then
/// the question, its summary, the reasons, the links, and each named answer with what it does.
/// `a` (or a click) chooses an answer, ↑↓ another, Enter sends it (cos, 2026-10-02).
fn structured_request(
    card: &mut Doc,
    from: &str,
    request: &fractal_client::StructuredRequest,
    images: &[fractal_conversation_ui::MailImage],
    drafts: &Drafts<'_>,
    inner: usize,
) {
    let label_of = |id: &str| {
        request
            .answers
            .iter()
            .find(|answer| answer.id == id)
            .map_or_else(|| id.to_owned(), |answer| answer.label.clone())
    };
    if let Some(recommendation) = &request.recommendation {
        card.lines(text::wrap(
            &[
                text::run("recommends  ", theme::dim()),
                text::run(
                    label_of(&recommendation.answer),
                    theme::strong(theme::GREEN),
                ),
                text::run(format!("  {}", recommendation.reason), theme::soft()),
            ],
            inner,
            &[],
            &[text::run("             ", theme::dim())],
            None,
        ));
        card.blank();
    }
    card.lines(text::markdown(
        &spaced(&request.question),
        inner,
        theme::text(),
    ));
    if let Some(summary) = request
        .summary
        .as_deref()
        .filter(|summary| !summary.is_empty())
    {
        card.blank();
        card.lines(text::markdown(&spaced(summary), inner, theme::soft()));
    }
    if !request.reasons.is_empty() {
        card.blank();
        for reason in &request.reasons {
            card.lines(text::wrap(
                &text::inline(reason, theme::text()),
                inner,
                &[text::run(" • ", theme::fg(theme::LAVENDER))],
                &[text::run("   ", theme::dim())],
                None,
            ));
        }
    }
    if !request.subjects.is_empty() {
        card.blank();
        for subject in &request.subjects {
            let target = subject
                .url
                .as_deref()
                .or(subject.reference.as_deref())
                .unwrap_or_default();
            card.lines(text::wrap(
                &[
                    text::run(subject.label.clone(), theme::text()),
                    text::run(format!("  {target}"), theme::dim()),
                ],
                inner,
                &[text::run("   ", theme::dim())],
                &[text::run("   ", theme::dim())],
                None,
            ));
        }
    }
    // The images it carries: a line each that opens it; where Fractal TUI keeps image state, the line
    // also loads it inline (`Content::expand_requests`).
    if !images.is_empty() {
        card.blank();
        for image in images {
            card.targets.push(super::doc::Target {
                line: card.lines.len(),
                column: 0,
                width: inner as u16,
                hit: Hit::Pane(fractal_conversation_ui::PaneIntent::Image(image.clone())),
            });
            card.line(Line::from(vec![
                span(
                    format!(
                        "   ▣ {} · {} · {} KB  ",
                        super::content::image_name(image),
                        image.media_type,
                        image.size.div_ceil(1024)
                    ),
                    theme::text(),
                ),
                span("open", theme::fg(theme::LAVENDER)),
            ]));
        }
    }
    card.blank();
    let recommended = request
        .recommendation
        .as_ref()
        .map(|wanted| wanted.answer.as_str());
    let named = request.answers.len();
    // Beside named answers a free answer is a row of its own; alone, words are the answer.
    let custom_row = request.custom && named > 0;
    let has_text = drafts.text.is_some_and(|text| !text.is_empty());
    let editor_open = drafts.editing || has_text;
    // The answer the open note belongs to, whatever row is highlighted now.
    let bound = drafts
        .note_answer
        .as_deref()
        .and_then(|id| request.answers.iter().find(|answer| answer.id == id));
    // With nothing highlighted, the row the editor writes for stays lifted.
    let lifted = drafts.answering.or_else(|| {
        if !editor_open {
            return None;
        }
        match drafts.note_answer.as_deref() {
            Some(id) => request.answers.iter().position(|answer| answer.id == id),
            None => custom_row.then_some(named),
        }
    });
    if named > 0 {
        card.section("choose one", None, inner);
    }
    for (index, answer) in request.answers.iter().enumerate() {
        let mut tags: Vec<(&str, Color)> = Vec::new();
        if recommended == Some(answer.id.as_str()) {
            tags.push(("recommended", theme::GREEN));
        }
        if answer.outcome.as_deref() == Some("request_changes") {
            tags.push(("needs your words", theme::YELLOW));
        }
        // Every answer that has a saved note says so, highlighted or not.
        let noted = drafts.noted_answers.contains(&answer.id)
            || (has_text && drafts.note_answer.as_deref() == Some(answer.id.as_str()));
        if noted {
            tags.push(("✎ note", theme::LAVENDER));
        }
        answer_row(
            card,
            Hit::Answer(index),
            &answer.label,
            &tags,
            &answer.consequence,
            lifted == Some(index),
            inner,
        );
    }
    if custom_row {
        let mut tags: Vec<(&str, Color)> = Vec::new();
        if has_text && drafts.note_answer.is_none() {
            tags.push(("✎ draft", theme::LAVENDER));
        }
        answer_row(
            card,
            Hit::Answer(named),
            "Custom answer",
            &tags,
            "write your own answer instead of these",
            lifted == Some(named),
            inner,
        );
    }
    card.blank();
    // A refused answer says why on the card until the next try, not in a passing flash.
    if let Some(error) = drafts.answer_error {
        card.lines(text::wrap(
            &[
                text::run("✕ Not sent  ", theme::strong(theme::RED)),
                text::run(error.to_owned(), theme::fg(theme::RED)),
            ],
            inner,
            &[],
            &[text::run("            ", theme::dim())],
            None,
        ));
        card.blank();
    }
    let highlighted = drafts.answering;
    let choosing = highlighted.is_some() && !drafts.editing;
    if editor_open {
        match bound {
            Some(answer) => {
                let outcome = outcome_word(answer);
                let required = answer.outcome.as_deref() == Some("request_changes");
                card.section("your note", None, inner);
                text_box(
                    card,
                    &format!("Note for “{}” · {outcome}", answer.label),
                    drafts,
                    if required {
                        "what should change? Enter sends it with this answer · Esc cancels"
                    } else {
                        "add context for this answer · Enter sends it · Esc cancels"
                    },
                    inner,
                );
                let others = noted_labels(request, &drafts.noted_answers, Some(&answer.id));
                let line = if choosing {
                    format!("This note stays with “{}” · {outcome}.", answer.label)
                } else {
                    match others {
                        Some(others) => format!(
                            "Enter sends “{}” · {outcome} with this note, plus your notes on {others}.",
                            answer.label
                        ),
                        None => format!("Enter sends “{}” · {outcome} with this note.", answer.label),
                    }
                };
                card.wrap(&[text::run(line, theme::dim())], inner);
                if !choosing {
                    let send = format!("Send “{}” · {outcome}", answer.label);
                    card.buttons_fit(
                        &[
                            ("enter", send.as_str(), Hit::Enter, theme::ACCENT),
                            ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                        ],
                        inner,
                    );
                }
            }
            None => {
                let (section, title, send) = if custom_row {
                    ("custom answer", "Custom answer", "Send custom answer")
                } else {
                    ("in your own words", "Answer in words", "Send the answer")
                };
                card.section(section, None, inner);
                text_box(card, &format!("{title} to {from}"), drafts, "", inner);
                if !choosing
                    && let Some(others) = noted_labels(request, &drafts.noted_answers, None)
                {
                    card.wrap(
                        &[text::run(
                            format!("Enter sends this answer, plus your notes on {others}."),
                            theme::dim(),
                        )],
                        inner,
                    );
                }
                if !choosing {
                    card.buttons_fit(
                        &[
                            ("enter", send, Hit::Enter, theme::ACCENT),
                            ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                        ],
                        inner,
                    );
                }
            }
        }
    }
    if choosing {
        let answer = highlighted.and_then(|index| request.answers.get(index));
        let required =
            answer.is_some_and(|answer| answer.outcome.as_deref() == Some("request_changes"));
        // An answer that needs words has its box on the card from the moment it is highlighted:
        // typing starts it.
        if let Some(answer) = answer.filter(|_| required && !editor_open) {
            card.section("your note", None, inner);
            text_box(
                card,
                &format!("Note for “{}” · {}", answer.label, outcome_word(answer)),
                drafts,
                &format!(
                    "what should change? Start typing; Enter sends it with “{}”",
                    answer.label
                ),
                inner,
            );
        }
        // What Enter sends: the highlighted answer and every note on the other answers.
        let own = answer.is_some_and(|answer| drafts.noted_answers.contains(&answer.id));
        let others = noted_labels(request, &drafts.noted_answers, answer.map(|answer| answer.id.as_str()));
        let send = match (answer, others) {
            (None, Some(others)) => {
                Some(format!("Enter writes a custom answer; your notes on {others} go with it."))
            }
            (Some(answer), Some(others)) if required => Some(format!(
                "Your notes on {others} go with “{}” once you write what should change.",
                answer.label
            )),
            (Some(answer), Some(others)) => Some(format!(
                "Enter sends “{}” · {}{}, plus your notes on {others}.",
                answer.label,
                outcome_word(answer),
                if own { " with its note" } else { "" }
            )),
            (Some(answer), None) if own && !required => Some(format!(
                "Enter sends “{}” · {} with its note.",
                answer.label,
                outcome_word(answer)
            )),
            _ => None,
        };
        if let Some(send) = send {
            card.wrap(&[text::run(send, theme::dim())], inner);
        }
        let enter = match answer {
            None => "Write custom answer",
            Some(_) if required => "Write your words",
            Some(_) => "Send this answer",
        };
        let mut buttons = vec![("enter", enter, Hit::Enter, theme::ACCENT)];
        if can_note(request, highlighted) {
            buttons.push(("c", if answer.is_some() { "Add note" } else { "Write custom answer" }, Hit::Key('c'), theme::ACCENT));
        }
        buttons.push(("↑↓", "Another", Hit::Key('a'), theme::OVERLAY1));
        buttons.push(("esc", "Not now", Hit::Escape, theme::OVERLAY1));
        card.buttons_fit(&buttons, inner);
    } else if editor_open {
        // The editor above carries its own Send and Cancel.
    } else if named == 0 {
        // A feedback ask has no answers to choose: words are the answer, and it can be set
        // aside, with word to its asker, when there is nothing to say (Nathan, 2026-10-05: a
        // feedback card offered "Choose an answer" and no way to answer or dismiss it).
        let label = format!("Close this: tell {from} there is nothing for you to do");
        if !confirm_row(card, drafts, &label) {
            let mut buttons = Vec::new();
            if can_note(request, None) {
                buttons.push(("c", "Answer in words", Hit::Key('c'), theme::ACCENT));
            }
            buttons.push((
                "x",
                "Dismiss: nothing to do",
                Hit::Key('x'),
                theme::OVERLAY1,
            ));
            card.buttons(&buttons);
        }
    } else {
        // A free answer is the custom row above: it is highlighted like any other answer.
        let mut buttons = vec![("a", "Choose an answer", Hit::Key('a'), theme::ACCENT)];
        if drafts.confirm.is_some() {
            card.buttons_fit(&buttons, inner);
            card.blank();
            let label = format!("Dismiss this: tell {from} you set it aside without answering");
            confirm_row(card, drafts, &label);
        } else {
            buttons.push(("x", "Dismiss without answering", Hit::Key('x'), theme::OVERLAY1));
            card.buttons_fit(&buttons, inner);
        }
    }
    if !request.why_person.is_empty() {
        card.blank();
        card.wrap(
            &text::inline(&format!("Why you: {}", request.why_person), theme::dim()),
            inner,
        );
    }
}

/// Whether `c` opens a note for the highlighted row: a named answer, the custom answer entry,
/// or, on a request without named answers, the words that are its answer.
pub(super) fn can_note(
    request: &fractal_client::StructuredRequest,
    highlighted: Option<usize>,
) -> bool {
    let named = request.answers.len();
    highlighted.is_some_and(|index| index < named)
        || (highlighted == Some(named) && request.custom)
        || (named == 0 && (request.entry_type == "feedback" || request.custom))
}

/// The answers with a saved note, except `except`, in the request's order, as the send line
/// names them: “A”, “A” and “B”, or “A”, “B” and “C”. None when there are none.
fn noted_labels(
    request: &fractal_client::StructuredRequest,
    noted: &[String],
    except: Option<&str>,
) -> Option<String> {
    let labels = request
        .answers
        .iter()
        .filter(|answer| Some(answer.id.as_str()) != except && noted.contains(&answer.id))
        .map(|answer| format!("“{}”", answer.label))
        .collect::<Vec<_>>();
    match labels.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

/// What a named answer does, in words for its note and send line. A choice only selects.
fn outcome_word(answer: &fractal_client::RequestAnswerOption) -> String {
    match answer.outcome.as_deref() {
        None => "selected".to_owned(),
        Some(outcome) => outcome.replace('_', " "),
    }
}

/// A radio row: the dot says it is a choice, the filled one is the choice, and the chosen row
/// is lifted off the card so it cannot be missed.
fn answer_row(
    card: &mut Doc,
    hit: Hit,
    label: &str,
    tags: &[(&str, Color)],
    consequence: &str,
    chosen: bool,
    inner: usize,
) {
    let fill = |style: Style| {
        if chosen {
            style.bg(theme::SURFACE0)
        } else {
            style
        }
    };
    let mut head = vec![
        text::run(
            if chosen { " ◉ " } else { " ○ " },
            fill(theme::strong(if chosen {
                theme::ACCENT
            } else {
                theme::OVERLAY1
            })),
        ),
        text::run(
            label.to_owned(),
            fill(if chosen {
                theme::strong(theme::ACCENT)
            } else {
                theme::strong(theme::TEXT)
            }),
        ),
    ];
    for (tag, color) in tags {
        head.push(text::run(format!("  {tag}"), fill(theme::fg(*color))));
    }
    head.push(text::run(format!("  {consequence}"), fill(theme::dim())));
    let from_line = card.lines.len();
    let mut wrapped = text::wrap(
        &head,
        inner,
        &[],
        &[text::run("    ", fill(theme::dim()))],
        None,
    );
    if chosen {
        for line in &mut wrapped {
            let used: usize = line.spans.iter().map(|span| span.width()).sum();
            line.spans.push(Span::styled(
                " ".repeat(inner.saturating_sub(used)),
                Style::default().bg(theme::SURFACE0),
            ));
        }
    }
    card.lines(wrapped);
    card.targets.push(crate::ui::doc::Target {
        line: from_line,
        column: 0,
        width: inner as u16,
        hit,
    });
}

/// A question as agents write one, "Recommend: …⏎Why: …⏎Answer …", given room to read: a blank
/// line between consecutive lines of prose, and a short leading label ("Why:") in bold. Lists,
/// quotes, tables, headings and code keep their lines together (Nathan, 2026-10-02).
pub fn spaced(question: &str) -> String {
    let block = |line: &str| {
        let line = line.trim_start();
        line.starts_with(['-', '*', '>', '|', '#'])
            || line.starts_with("```")
            || line.split_once(". ").is_some_and(|(digits, _)| {
                !digits.is_empty() && digits.len() < 4 && digits.chars().all(|c| c.is_ascii_digit())
            })
    };
    let label = |line: &str| -> String {
        match line.split_once(": ") {
            Some((head, rest))
                if !head.is_empty()
                    && head.len() <= 24
                    && head.split_whitespace().count() <= 3
                    && head.starts_with(|c: char| c.is_uppercase())
                    && !head.contains(['*', '`', '[']) =>
            {
                format!("**{head}:** {rest}")
            }
            _ => line.to_owned(),
        }
    };
    let mut out = Vec::new();
    let mut fenced = false;
    let mut previous_prose = false;
    for line in question.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let prose = !fenced && !line.trim().is_empty() && !block(line);
        if prose && previous_prose {
            out.push(String::new());
        }
        out.push(if prose { label(line) } else { line.to_owned() });
        previous_prose = prose;
    }
    out.join("\n")
}

fn report(text: &str, width: usize) -> Option<Vec<Line<'static>>> {
    const TELLING: [&str; 14] = [
        "status", "state", "outcome", "result", "error", "reason", "message", "summary", "commit",
        "sha", "version", "host", "run", "url",
    ];
    const OTHERS: usize = 4;
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let serde_json::Value::Object(fields) =
        serde_json::from_str::<serde_json::Value>(text.get(start..=end)?).ok()?
    else {
        return None;
    };
    let mut lines = Vec::new();
    let before = text[..start].trim();
    if !before.is_empty() {
        lines.extend(text::markdown(before, width, theme::text()));
        lines.push(Line::default());
    }
    let shown = |value: &serde_json::Value| match value {
        serde_json::Value::String(text) => text.lines().next().unwrap_or_default().to_owned(),
        other => other.to_string(),
    };
    let key_width = fields
        .keys()
        .map(|key| text::width(key))
        .max()
        .unwrap_or(0)
        .min(14);
    let row = |key: &str, value: &serde_json::Value, style: Style| {
        let value = text::truncate(&shown(value), width.saturating_sub(key_width + 2));
        Line::from(vec![
            span(format!("{key:<key_width$}  "), theme::dim()),
            span(value, style),
        ])
    };
    let mut telling = TELLING
        .iter()
        .filter_map(|key| fields.get_key_value(*key))
        .collect::<Vec<_>>();
    if telling.is_empty() {
        telling = fields.iter().take(OTHERS).collect();
    }
    for (key, value) in &telling {
        let style = if key.as_str() == "error" || matches!(value.as_str(), Some("failed" | "error"))
        {
            theme::fg(theme::RED)
        } else {
            theme::text()
        };
        lines.push(row(key, value, style));
    }
    let rest = fields
        .iter()
        .filter(|(key, _)| !telling.iter().any(|(shown, _)| shown == key))
        .collect::<Vec<_>>();
    for (key, value) in rest.iter().take(OTHERS) {
        lines.push(row(key, value, theme::soft()));
    }
    if rest.len() > OTHERS {
        lines.push(Line::from(span(
            format!("… {} more fields", rest.len() - OTHERS),
            theme::dim(),
        )));
    }
    Some(lines)
}

fn confirm_row(doc: &mut Doc, drafts: &Drafts<'_>, label: &str) -> bool {
    if drafts.confirm.is_some() {
        doc.line(Line::from(vec![span(
            format!("{label}? "),
            theme::strong(theme::YELLOW),
        )]));
        doc.buttons(&[
            ("y", "Yes, do it", Hit::Key('y'), theme::GREEN),
            ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
        ]);
        true
    } else {
        false
    }
}

pub fn home_detail(world: &World, id: Option<&str>, width: usize, drafts: &Drafts<'_>) -> Doc {
    let mut doc = Doc::new();
    let Some(item) = id.and_then(|id| world.attention.items().iter().find(|item| item.id == id))
    else {
        match &world.attention {
            Load::Loading => doc.line(Line::from(span("Checking for alerts…", theme::dim()))),
            Load::Failed(error) => doc.wrap(
                &[run(
                    format!("Could not read your alerts: {error}"),
                    theme::fg(theme::RED),
                )],
                width,
            ),
            Load::Ready(_) => {
                doc.blank();
                if let Some(why) = alerts_out_of_date(world) {
                    doc.line(Line::from(vec![
                        span("◐ ", theme::strong(theme::YELLOW)),
                        span(HOME_UNKNOWN_TEXT, theme::bold()),
                    ]));
                    doc.wrap(&[run(why, theme::dim())], width);
                } else {
                    doc.line(Line::from(vec![
                        span("✓ ", theme::strong(theme::GREEN)),
                        span(HOME_IDLE_TEXT, theme::bold()),
                    ]));
                }
                doc.blank();
                doc.wrap(
                    &[run(
                        format!(
                            "{} missions are running without you. They are on the Missions tab.",
                            world.quiet_missions
                        ),
                        theme::dim(),
                    )],
                    width,
                );
            }
        }
        return doc;
    };
    let (glyph, color) = attention_style(&item.kind);
    let heavy = color == theme::PERSON;
    let mut card = Doc::new();
    let inner = width.saturating_sub(4);
    card.blank();
    card.wrap(&text::inline(&item.title, theme::bold()), inner);
    let mut meta = vec![span(
        format!("{glyph} {} ", item.kind.word()),
        theme::fg(color),
    )];
    if let Some(waiting) = &item.waiting {
        meta.push(span(format!("· {waiting} is waiting · "), theme::dim()));
    } else {
        meta.push(span("· ", theme::dim()));
    }
    meta.push(span(format!("{} ago", item.age), theme::dim()));
    card.line(Line::from(meta));
    if let Some(mission) = &item.mission {
        let label = world
            .missions
            .items()
            .iter()
            .find(|candidate| &candidate.id == mission)
            .map(|candidate| candidate.title.clone())
            .unwrap_or_else(|| mission.trim_start_matches("mission/").to_owned());
        link(&mut card, "mission  ", &label, mission);
    }
    // The step that waits on this ask, and what it is for: what answering lets go on.
    if let Some(blocked) = &item.blocked {
        card.lines(text::wrap(
            &[
                text::run(blocked.step.clone(), theme::soft()),
                text::run(
                    if blocked.goal.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", blocked.goal)
                    },
                    theme::dim(),
                ),
                text::run(" — it continues once you answer", theme::dim()),
            ],
            inner,
            &[text::run("waits    ", theme::dim())],
            &[text::run("         ", theme::dim())],
            None,
        ));
    }
    if let Some(agent) = &item.agent {
        let label = world
            .agents
            .items()
            .iter()
            .find(|candidate| &candidate.id == agent)
            .map(|candidate| candidate.name.clone())
            .unwrap_or_else(|| agent.trim_start_matches("agent/").to_owned());
        link(&mut card, "agent    ", &label, agent);
    }
    if let AttentionKind::Request { from, from_id, .. } = &item.kind
        && item.agent.as_deref() != Some(from_id.as_str())
    {
        card.field("from", from, inner, theme::soft());
    }
    card.blank();
    match &item.kind {
        AttentionKind::Review {
            question,
            because,
            look_at,
            step,
            ..
        } => {
            card.wrap(&text::inline(question, theme::text()), inner);
            card.blank();
            card.field("because", because, inner, theme::soft());
            for (label, value) in look_at {
                card.field(label, value, inner, theme::text());
            }
            if !step.is_empty() {
                card.field("step", step, inner, theme::soft());
            }
            card.blank();
            // There is always somewhere to write: the agent reads the words with either answer.
            let written = drafts.text.is_some_and(|text| !text.trim().is_empty());
            let label = if written {
                "Approve, with your notes"
            } else {
                "Approve"
            };
            if !confirm_row(&mut card, drafts, label) {
                text_box(
                    &mut card,
                    "your notes · the agent reads these",
                    drafts,
                    "c or click to write what you think, what should change, anything it should know",
                    inner,
                );
                if drafts.editing {
                    card.buttons(&[
                        (
                            "enter",
                            "Send back with these notes",
                            Hit::Enter,
                            theme::YELLOW,
                        ),
                        ("esc", "Stop writing", Hit::Escape, theme::OVERLAY1),
                    ]);
                } else if written {
                    card.buttons(&[
                        ("a", "Approve, with your notes", Hit::Key('a'), theme::GREEN),
                        ("c", "Keep writing", Hit::Key('c'), theme::OVERLAY1),
                        (
                            "enter",
                            "Send back with these notes",
                            Hit::Enter,
                            theme::YELLOW,
                        ),
                    ]);
                } else {
                    card.buttons(&[
                        ("a", "Approve", Hit::Key('a'), theme::GREEN),
                        (
                            "c",
                            "Write notes or request changes",
                            Hit::Key('c'),
                            theme::YELLOW,
                        ),
                    ]);
                }
            }
        }
        AttentionKind::Feedback {
            question,
            subject,
            excerpt,
            link,
        } => {
            card.wrap(&text::inline(question, theme::text()), inner);
            card.blank();
            let mut preview = Doc::new();
            preview.lines(text::markdown(
                &excerpt.join("\n"),
                inner.saturating_sub(4),
                theme::soft(),
            ));
            card.card(subject, theme::OVERLAY1, false, preview, inner);
            if let Some(link) = link {
                let url = link.starts_with("http://") || link.starts_with("https://");
                card.line(Line::from(vec![
                    span(
                        if url { "copy link  " } else { "reference  " },
                        theme::dim(),
                    ),
                    span(
                        link.clone(),
                        if url {
                            theme::fg(theme::BLUE).add_modifier(Modifier::UNDERLINED)
                        } else {
                            theme::dim()
                        },
                    ),
                ]));
            }
            card.blank();
            text_box(
                &mut card,
                "your feedback · the agent reads this",
                drafts,
                "Click here or press c to write feedback",
                inner,
            );
            if drafts.editing {
                card.buttons(&[
                    ("enter", "Send feedback", Hit::Enter, theme::ACCENT),
                    ("esc", "Stop editing", Hit::Escape, theme::OVERLAY1),
                ]);
            } else if !confirm_row(&mut card, drafts, "Say it looks good") {
                card.buttons(&[
                    ("c", "Write feedback", Hit::Key('c'), theme::ACCENT),
                    ("a", "Looks good as is", Hit::Key('a'), theme::GREEN),
                ]);
            }
        }
        AttentionKind::Launch {
            planner,
            name,
            preview,
        } => {
            card.wrap(
                &[run(
                    format!("{planner} proposes a new mission. Nothing runs until you approve it."),
                    theme::text(),
                )],
                inner,
            );
            card.blank();
            let preview = match preview {
                Load::Ready(preview) => Some(preview),
                Load::Loading => {
                    card.field("mission", name, inner, theme::bold());
                    card.blank();
                    card.line(Line::from(span(
                        "Loading the proposed mission…",
                        theme::dim(),
                    )));
                    card.blank();
                    None
                }
                Load::Failed(reason) => {
                    card.field("mission", name, inner, theme::bold());
                    card.blank();
                    let mut why = Doc::new();
                    for line in reason.lines() {
                        why.wrap(&text::inline(line, theme::text()), inner.saturating_sub(4));
                    }
                    card.card("nothing to approve yet", theme::YELLOW, false, why, inner);
                    card.blank();
                    None
                }
            };
            if let Some(preview) = preview {
                card.field("mission", &preview.name, inner, theme::bold());
                if !preview.workspace.is_empty() {
                    card.field("in", &preview.workspace, inner, theme::soft());
                }
                let asks = preview.steps.iter().filter(|step| step.asks_you).count();
                card.field(
                    "asks you",
                    &format!(
                        "{asks} time{} before it finishes",
                        if asks == 1 { "" } else { "s" }
                    ),
                    inner,
                    theme::fg(theme::PERSON),
                );
                card.blank();
                card.section("goals", None, inner);
                for goal in &preview.goals {
                    card.lines(text::wrap(
                        &text::inline(goal, theme::text()),
                        inner,
                        &[run("◆ ", theme::fg(theme::LAVENDER))],
                        &[run("  ", theme::dim())],
                        None,
                    ));
                }
                card.blank();
                card.section("steps", Some(preview.steps.len()), inner);
                card.lines(flow(
                    preview.steps.iter().map(|step| {
                        (
                            step.name.as_str(),
                            step.after.as_slice(),
                            if step.asks_you {
                                theme::PERSON
                            } else {
                                theme::SUBTEXT1
                            },
                        )
                    }),
                    inner,
                ));
                for step in &preview.steps {
                    card.line(Line::from(vec![
                        span(format!("  {:<12}", step.name), theme::text()),
                        span(
                            format!("{:<18}", step.assignee),
                            if step.asks_you {
                                theme::fg(theme::PERSON)
                            } else {
                                theme::soft()
                            },
                        ),
                        span(
                            if step.after.is_empty() {
                                "first".to_owned()
                            } else {
                                format!("after {}", step.after.join(", "))
                            },
                            theme::dim(),
                        ),
                    ]));
                }
                card.blank();
                card.section("agents", Some(preview.agents.len()), inner);
                for agent in &preview.agents {
                    card.line(Line::from(vec![
                        span(
                            format!("  {:<18} ", text::truncate(&agent.name, 18)),
                            theme::text(),
                        ),
                        span(
                            format!("{:<8}", agent.harness.name()),
                            theme::fg(harness_color(agent.harness)),
                        ),
                        span(format!("on {}", agent.host), theme::dim()),
                    ]));
                }
                card.blank();
            }
            if drafts.editing || drafts.text.is_some_and(|text| !text.is_empty()) {
                text_box(
                    &mut card,
                    "what the planner should change",
                    drafts,
                    "",
                    inner,
                );
                card.buttons(&[
                    ("enter", "Send to the planner", Hit::Enter, theme::YELLOW),
                    ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                ]);
            } else if !confirm_row(
                &mut card,
                drafts,
                if drafts.confirm == Some('a') {
                    "Approve and start this mission"
                } else {
                    "Cancel this launch"
                },
            ) {
                if preview.is_some() {
                    card.buttons(&[
                        ("a", "Approve launch", Hit::Key('a'), theme::GREEN),
                        ("c", "Ask for changes", Hit::Key('c'), theme::YELLOW),
                        ("d", "Cancel launch", Hit::Key('d'), theme::RED),
                    ]);
                } else {
                    card.buttons(&[
                        ("c", "Ask the planner", Hit::Key('c'), theme::YELLOW),
                        ("d", "Cancel launch", Hit::Key('d'), theme::RED),
                    ]);
                }
            }
        }
        AttentionKind::Revision { reason, changes } => {
            card.wrap(&text::inline(reason, theme::text()), inner);
            card.blank();
            card.section("changes to the plan", Some(changes.len()), inner);
            for (sign, change) in changes {
                let color = match sign {
                    '+' => theme::GREEN,
                    '-' => theme::RED,
                    _ => theme::YELLOW,
                };
                card.lines(text::wrap(
                    &[run(change.clone(), theme::fg(color))],
                    inner,
                    &[run(format!("{sign} "), theme::strong(color))],
                    &[run("  ", theme::dim())],
                    None,
                ));
            }
            card.blank();
            if drafts.editing || drafts.text.is_some_and(|text| !text.is_empty()) {
                text_box(
                    &mut card,
                    "what should change · the proposing agent reads this",
                    drafts,
                    "",
                    inner,
                );
                card.buttons(&[
                    ("enter", "Ask for these changes", Hit::Enter, theme::YELLOW),
                    ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                ]);
            } else if !confirm_row(
                &mut card,
                drafts,
                if drafts.confirm == Some('a') {
                    "Approve the revision"
                } else {
                    "Reject the revision"
                },
            ) {
                card.buttons(&[
                    ("a", "Approve revision", Hit::Key('a'), theme::GREEN),
                    ("c", "Ask for changes", Hit::Key('c'), theme::YELLOW),
                    ("j", "Reject", Hit::Key('j'), theme::RED),
                ]);
            }
        }
        AttentionKind::Fault {
            what,
            because,
            fix,
            source,
        } => {
            card.wrap(&text::inline(what, theme::text()), inner);
            card.blank();
            card.field("because", because, inner, theme::soft());
            if !source.is_empty() {
                card.field("source", source, inner, theme::soft());
            }
            if let Some(fix) = fix {
                card.blank();
                let mut suggestion = Doc::new();
                suggestion.wrap(&text::inline(fix, theme::text()), inner.saturating_sub(4));
                card.card("suggested fix", theme::GREEN, false, suggestion, inner);
            }
            card.blank();
            card.wrap(
                &text::inline("Act on the source to clear this fault.", theme::soft()),
                inner,
            );
        }
        AttentionKind::Request {
            from,
            structured: Some(request),
            images,
            ..
        } => structured_request(&mut card, from, request, images, drafts, inner),
        AttentionKind::Request { from, question, .. } => {
            let custom = item.actions.iter().any(|action| action == "custom.reply");
            // A report pasted in as JSON shows its telling fields, not a wall of braces.
            match report(question, inner) {
                Some(lines) => card.lines(lines),
                None => card.lines(text::markdown(&spaced(question), inner, theme::text())),
            }
            card.blank();
            // The answer goes back to the agent that asked, and its waiting step continues.
            if drafts.editing || drafts.text.is_some_and(|text| !text.is_empty()) {
                text_box(&mut card, &format!("answer {from}"), drafts, "", inner);
                card.buttons(&[
                    ("enter", "Send the answer", Hit::Enter, theme::ACCENT),
                    ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                ]);
            } else if custom {
                card.buttons(&[("c", "Reply with fields", Hit::Key('c'), theme::ACCENT)]);
            } else {
                let label = match drafts.confirm {
                    Some('y') => format!("Answer {from} “Yes”"),
                    Some('n') => format!("Answer {from} “No”"),
                    _ => format!("Close this: tell {from} there is nothing for you to do"),
                };
                if !confirm_row(&mut card, drafts, &label) {
                    // A yes-or-no question can be answered in one key.
                    if item.title.trim_end().ends_with('?') {
                        card.buttons(&[
                            ("y", "Yes", Hit::Key('y'), theme::GREEN),
                            ("n", "No", Hit::Key('n'), theme::RED),
                            ("c", "Answer in words", Hit::Key('c'), theme::ACCENT),
                            (
                                "x",
                                "Dismiss: nothing to do",
                                Hit::Key('x'),
                                theme::OVERLAY1,
                            ),
                        ]);
                    } else {
                        card.buttons(&[
                            ("c", "Answer", Hit::Key('c'), theme::ACCENT),
                            (
                                "x",
                                "Dismiss: nothing to do",
                                Hit::Key('x'),
                                theme::OVERLAY1,
                            ),
                        ]);
                    }
                }
            }
            card.wrap(
                &text::inline(
                    &if custom {
                        "Reply using the choices and fields shown above.".to_owned()
                    } else {
                        format!("{from} is waiting on you: answer it, or x to dismiss it when there is nothing for you to do. Either way the step it waits on continues.")
                    },
                    theme::dim(),
                ),
                inner,
            );
        }
        AttentionKind::Prompt {
            seat, text: prompt, ..
        } => {
            card.line(Line::from(vec![
                span("waits  ", theme::dim()),
                span(seat.clone(), theme::strong(theme::PERSON)),
            ]));
            card.blank();
            // A prompt the form asks in its own words is said once, as the form's question.
            if !super::alerts::form_asks(world, item, prompt) {
                card.lines(text::markdown(&spaced(prompt), inner, theme::text()));
                card.blank();
            }
            // The same form the band in its conversation expands to, with the call in full.
            super::alerts::prompt_form(
                &mut card,
                world,
                item,
                drafts.form,
                inner,
                None,
                super::alerts::Place::Inbox,
            );
        }
        AttentionKind::Login { text: login, seats } => {
            card.lines(text::markdown(&spaced(login), inner, theme::text()));
            if seats.len() > 1 {
                card.blank();
                card.section("seats that share this sign-in", Some(seats.len()), inner);
                for seat in seats {
                    link(&mut card, "         ", seat, seat);
                }
            }
            card.blank();
            card.wrap(
                &text::inline(
                    &format!("One sign-in in the seat's terminal ({}) fixes every seat; this clears when the next authenticated turn succeeds.", terminal_route()),
                    theme::dim(),
                ),
                inner,
            );
        }
        AttentionKind::Update {
            from,
            body,
            about,
            subjects,
        } => {
            card.line(Line::from(vec![
                span("from  ", theme::dim()),
                span(from.clone(), theme::strong(theme::PERSON)),
            ]));
            card.blank();
            card.lines(text::markdown(&spaced(body), inner, theme::text()));
            let links = std::iter::once(("about".to_owned(), about.clone()))
                .filter(|(_, about)| !about.is_empty())
                .chain(subjects.iter().cloned());
            let mut first = true;
            for (label, target) in links {
                if first {
                    card.blank();
                    first = false;
                }
                card.lines(text::wrap(
                    &[
                        text::run(label, theme::text()),
                        text::run(format!("  {target}"), theme::dim()),
                    ],
                    inner,
                    &[text::run("   ", theme::dim())],
                    &[text::run("   ", theme::dim())],
                    None,
                ));
            }
            card.blank();
            card.buttons(&[("x", "Dismiss", Hit::Key('x'), theme::GREEN)]);
            card.wrap(
                &text::inline(
                    "Nothing waits on this. It clears once you have read it: x, or leave it open a moment.",
                    theme::dim(),
                ),
                inner,
            );
        }
        AttentionKind::Unplaced { text: body, why, .. } => {
            card.lines(text::markdown(body, inner, theme::text()));
            card.blank();
            card.wrap(&text::inline(&super::adapt::human_presentation(why), theme::dim()), inner);
            // Only the answers st lists for the row: a request's answer, never one guessed from
            // its kind, so a gate here is answered with st.
            if let Some(answer) = unplaced_answer(&item.actions) {
                card.blank();
                if drafts.editing || drafts.text.is_some_and(|text| !text.is_empty()) {
                    text_box(&mut card, "answer", drafts, "", inner);
                    card.buttons(&[
                        ("enter", "Send the answer", Hit::Enter, theme::ACCENT),
                        ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                    ]);
                } else {
                    let label = if answer == "custom.reply" {
                        "Reply with fields"
                    } else {
                        "Answer"
                    };
                    card.buttons(&[("c", label, Hit::Key('c'), theme::ACCENT)]);
                }
            }
        }
        AttentionKind::Message { from, body } => {
            card.line(Line::from(vec![
                span("from  ", theme::dim()),
                span(from.clone(), theme::strong(theme::SAPPHIRE)),
            ]));
            card.blank();
            card.lines(text::markdown(body, inner, theme::text()));
            card.blank();
            if drafts.editing || drafts.text.is_some_and(|text| !text.is_empty()) {
                text_box(&mut card, &format!("reply to {from}"), drafts, "", inner);
                card.buttons(&[
                    ("enter", "Send reply", Hit::Enter, theme::ACCENT),
                    ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
                ]);
            } else {
                card.buttons(&[
                    ("c", "Reply", Hit::Key('c'), theme::ACCENT),
                    ("x", "Dismiss: mark read", Hit::Key('x'), theme::GREEN),
                ]);
            }
        }
    }
    card.blank();
    // What the item is about, as links wherever st names a graph subject.
    card.section("related", None, inner);
    if let Some(who) = &item.raised_by {
        card.line(Line::from(vec![
            span(format!("{:<10}", "raised by"), theme::dim()),
            span(who.clone(), theme::soft()),
        ]));
    }
    if item.related.is_empty() && item.mission.is_none() && item.agent.is_none() {
        card.line(Line::from(span(
            "Fractal names no agent, mission or step for this item.",
            theme::dim(),
        )));
    }
    for (target, state) in &item.related {
        let prefix = format!("{:<10}", target.split('/').next().unwrap_or("item"));
        if target.starts_with("agent/")
            || target.starts_with("mission/")
            || target.starts_with("attention/")
        {
            link(&mut card, &prefix, target, target);
        } else {
            card.line(Line::from(vec![
                span(prefix, theme::dim()),
                span(target.clone(), theme::soft()),
            ]));
        }
        if let Some(state) = state {
            card.line(Line::from(span(format!("{:<10}{state}", ""), theme::dim())));
        }
    }
    card.blank();
    // Every item: talk to whoever can act on it, or go to what it is about.
    if let Some(chat) = &drafts.chat {
        card.section(&format!("chat with {} about this", chat.to), None, inner);
        if chat.thread.is_empty() {
            card.wrap(
                &[run(
                    format!(
                        "{} gets this item as context: its title, mission and question.",
                        chat.to
                    ),
                    theme::dim(),
                )],
                inner,
            );
        } else {
            card.lines(chat.thread.iter().cloned());
        }
        card.blank();
        let box_drafts = Drafts {
            text: Some(chat.text),
            cursor: chat.cursor,
            editing: chat.editing,
            answering: None,
            note_answer: None,
            noted_answers: Vec::new(),
            answer_error: None,
            confirm: None,
            chat: None,
            form: Default::default(),
        };
        let first_target = card.targets.len();
        text_box(
            &mut card,
            &format!("message {}", chat.to),
            &box_drafts,
            "",
            inner,
        );
        card.buttons(&[
            ("enter", "Send", Hit::Enter, theme::ACCENT),
            ("esc", "Close chat", Hit::Escape, theme::OVERLAY1),
        ]);
        // The chat's box and Send are its own: a click on them never reaches a request's note.
        for target in &mut card.targets[first_target..] {
            target.hit = match target.hit {
                Hit::Composer => Hit::ChatComposer,
                Hit::Enter => Hit::ChatEnter,
                ref other => other.clone(),
            };
        }
    } else if !drafts.editing && drafts.confirm.is_none() {
        let mut buttons = vec![("t", "Chat about this", Hit::Key('t'), theme::SAPPHIRE)];
        if item.mission.is_some() || item.agent.is_some() {
            buttons.push((
                "g",
                if item.mission.is_some() {
                    "Go to the mission"
                } else {
                    "Go to the agent"
                },
                Hit::Key('g'),
                theme::OVERLAY1,
            ));
        }
        // Put off on every device of the person (`attention.snooze`), or brought back.
        buttons.push(if item.snoozed(world.at) {
            ("l", "Bring back now", Hit::Key('l'), theme::OVERLAY1)
        } else {
            ("l", super::SNOOZE_LABEL, Hit::Key('l'), theme::OVERLAY1)
        });
        card.buttons(&buttons);
    }
    card.blank();
    let title = format!("{} · waiting {}", item.kind.word(), item.age);
    doc.card(
        &title,
        if heavy { theme::PERSON } else { color },
        heavy,
        card,
        width,
    );
    doc
}

/// Steps drawn as a flow: each layer after the one it depends on.
fn flow<'a>(
    steps: impl Iterator<Item = (&'a str, &'a [String], Color)>,
    width: usize,
) -> Vec<Line<'static>> {
    fractal_mission_ui::flow(
        steps,
        width,
        &fractal_mission_ui::Palette::default(),
        &fractal_mission_ui::Glyphs::default(),
    )
}

// ------------------------------------------------------------------- agents

pub fn agent_order(world: &World) -> Vec<&Agent> {
    let mut agents = world.agents.items().iter().collect::<Vec<_>>();
    agents.sort_by(|a, b| {
        (a.unmanaged, a.state, a.name.to_lowercase()).cmp(&(
            b.unmanaged,
            b.state,
            b.name.to_lowercase(),
        ))
    });
    agents
}

fn agent_group(agent: &Agent) -> &'static str {
    if agent.unmanaged {
        return "found running · not started by Fractal";
    }
    match agent.state {
        AgentState::NeedsYou => "waiting on you",
        AgentState::NeedsLogin => "needs login",
        AgentState::Fault | AgentState::Down => "broken",
        AgentState::Working | AgentState::Compacting => "working",
        AgentState::Idle | AgentState::Starting => "idle",
        AgentState::Stopped | AgentState::Unknown => "stopped",
        AgentState::Retired => "retired",
    }
}

/// An agent's subagents as lines beneath its row, one each under a branch: what each does, its
/// type, and how long it has run. They are part of the agent's row, never rows of their own.
fn subagent_lines(agent: &Agent, indent: &str, width: usize) -> Vec<Line<'static>> {
    let count = agent.subagents.len();
    agent
        .subagents
        .iter()
        .enumerate()
        .map(|(index, subagent)| {
            let branch = if index + 1 == count { "└ " } else { "├ " };
            let lead = format!("{indent}{branch}");
            let age = if subagent.age.is_empty() {
                String::new()
            } else {
                format!(" {}", subagent.age)
            };
            let room = width.saturating_sub(text::width(&lead) + text::width(&age) + 1);
            Line::from(vec![
                span(lead, theme::fg(theme::SURFACE2)),
                span(
                    text::truncate(&subagent.label(), room),
                    theme::fg(theme::OVERLAY1),
                ),
                span(age, theme::dim()),
            ])
        })
        .collect()
}

/// Reserve the standing marker before the shell truncates the row against its right columns.
/// The last span is the name; tree callers include their final folder indent before fitting it.
fn mark_standing(
    agent: &Agent,
    first: &mut Vec<Span<'static>>,
    right: &[Span<'static>],
    width: usize,
) {
    if agent.lifecycle != Some(fractal_client::AgentLifecycle::Standing) {
        return;
    }
    const MARKER: &str = " \u{F0031}";
    let prefix_width = first[..first.len() - 1].iter().map(Span::width).sum::<usize>();
    let right_width = right.iter().map(Span::width).sum::<usize>();
    let room = width.saturating_sub(prefix_width + right_width + 2 + text::width(MARKER));
    if let Some(name) = first.last_mut() {
        name.content = text::truncate(&name.content, room).into();
    }
    first.push(span(MARKER, theme::dim()));
}

/// The right of an agent's row: `•` first when it said something since the person last read its
/// conversation here (st's `last_prose_at` against the read position), as the web and the phone
/// mark it; then its harness and how long ago it was active.
fn agent_row_right(world: &World, agent: &Agent) -> Vec<Span<'static>> {
    let mut right = Vec::with_capacity(3);
    if world.unread.contains(&agent.id) {
        right.push(span("• ", theme::strong(theme::ACCENT)));
    }
    right.push(span(agent.harness.name(), theme::fg(harness_color(agent.harness))));
    right.push(span(format!(" {:>4}", agent.activity), theme::dim()));
    right
}

pub fn agents_list(world: &World, spinner: &'static str, width: usize) -> Listing {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    if world.inventory_partial.iter().any(|name| name == "sessions") {
        items.push(Item::Note(Line::from(span(" More native sessions exist beyond this inventory window.", theme::dim()))));
    }
    let agents = agent_order(world);
    let mut current = "";
    for agent in &agents {
        let group = agent_group(agent);
        if group != current {
            current = group;
            let count = agents
                .iter()
                .filter(|other| agent_group(other) == group)
                .count();
            items.push(Item::Header {
                title: group.into(),
                count,
                color: if matches!(agent.state, AgentState::NeedsYou | AgentState::NeedsLogin)
                    && !agent.unmanaged
                {
                    theme::PERSON
                } else {
                    theme::OVERLAY1
                },
            });
        }
        let (glyph, color) = agent_glyph(agent.state, spinner);
        let path = agent.id.strip_prefix("agent/").unwrap_or(&agent.id);
        let mut first = vec![
            span(format!(" {glyph} "), theme::strong(color)),
            span(agent.name.clone(), theme::bold()),
        ];
        let right = agent_row_right(world, agent);
        mark_standing(agent, &mut first, &right, width);
        items.push(Item::Row {
            index: ids.len(),
            first,
            right,
            // What its step last reported leads; the path says who it is when nothing was said.
            second: match &agent.details.progress {
                Some(progress) => vec![span(
                    format!("   {}", text::truncate(progress, width.saturating_sub(4))),
                    theme::soft(),
                )],
                None => vec![span(
                    format!("   {}", text::truncate(path, width.saturating_sub(4))),
                    theme::dim(),
                )],
            },
            children: subagent_lines(agent, "   ", width),
        });
        ids.push(agent.id.clone());
    }
    Listing {
        items,
        ids,
        state: state_of(&world.agents, "Loading agents…", "No agents yet."),
        legend: legend(&[
            ("◆", theme::PERSON, "needs you"),
            (spinner, theme::WORKING, "working"),
            ("●", theme::IDLE, "idle"),
            ("✕", theme::FAULT, "broken"),
            ("○", theme::QUIET, "stopped"),
            ("?", theme::QUIET, "unmanaged"),
            ("•", theme::ACCENT, "new since you read"),
        ])
        .into_iter()
        .chain(world.agents.items().iter().any(|agent| agent.reported.is_some()).then(|| {
            Line::from(span(
                "This host's Fractal is older: agent states show as Fractal reports them.",
                theme::dim(),
            ))
        }))
        .collect(),
    }
}

/// The header strip above an agent's conversation.
/// What a person does for an agent whose harness is signed out of its provider. st clears the
/// state once the harness is signed in again, on the same run: no restart (Nathan, 2026-10-04).
pub fn login_guidance(agent: &Agent) -> String {
    let how = match agent.harness {
        Harness::Claude => format!("open its terminal ({}) and run /login", terminal_route()),
        Harness::Codex => format!("run `codex login` on {} (or in its terminal)", agent.host),
        _ => format!("open its terminal ({}) and log it in", terminal_route()),
    };
    let provider = match agent.harness {
        Harness::Claude => "Claude",
        Harness::Codex => "Codex",
        _ => "Its provider",
    };
    format!(
        "{provider} login required on {}: {how}. Messages wait until it is signed in; it carries on without a restart.",
        agent.host
    )
}

pub fn agent_header(world: &World, agent: &Agent, width: usize, spinner: &'static str) -> Doc {
    let mut doc = Doc::new();
    let (glyph, color) = agent_glyph(agent.state, spinner);
    let mut first = vec![
        span(format!(" {glyph} "), theme::strong(color)),
        span(agent.name.clone(), theme::bold()),
        span(format!("  {}", agent_shown_word(agent)), theme::fg(color)),
    ];
    // The model rides beside the harness, as the harness reported it.
    let model = agent
        .details
        .model
        .as_deref()
        .map(|model| format!(" · {model}"))
        .unwrap_or_default();
    let right = format!("{}{model} · {} ", agent.harness.name(), agent.host);
    let used = first.iter().map(Span::width).sum::<usize>();
    first.push(span(
        " ".repeat(width.saturating_sub(used + text::width(&right))),
        theme::dim(),
    ));
    first.push(span(
        agent.harness.name(),
        theme::fg(harness_color(agent.harness)),
    ));
    first.push(span(format!("{model} · {} ", agent.host), theme::dim()));
    doc.line(Line::from(first));
    let mut second = vec![span(format!("   {}", agent.id), theme::dim())];
    if let Some(tree) = &agent.worktree {
        second.push(span(format!("  ·  {tree}"), theme::dim()));
    }
    // The agent's actions, discoverable from where it is shown (ctrl+a too).
    let actions = "⋯ actions ";
    let used = second.iter().map(Span::width).sum::<usize>();
    if used + text::width(actions) + 2 <= width {
        let column = width - text::width(actions);
        second.push(span(" ".repeat(column - used), theme::dim()));
        second.push(span(actions, theme::fg(theme::ACCENT)));
        doc.targets.push(super::doc::Target {
            line: doc.lines.len(),
            column: column as u16,
            width: text::width(actions) as u16,
            hit: Hit::Actions(agent.id.clone()),
        });
    }
    doc.line(Line::from(second));
    if agent.state == AgentState::NeedsLogin {
        doc.lines(text::wrap(
            &text::inline(&login_guidance(agent), theme::fg(theme::PERSON)),
            width,
            &[text::run("   ⚿ ", theme::fg(theme::PERSON))],
            &[text::run("     ", theme::dim())],
            None,
        ));
    }
    if let Some(progress) = &agent.details.progress {
        doc.lines(text::wrap(
            &text::inline(progress, theme::text()),
            width,
            &[text::run("   ", theme::dim())],
            &[text::run("   ", theme::dim())],
            None,
        ));
    }
    if let (Some(mission), Some(step)) = (&agent.mission, &agent.step) {
        let title = world
            .missions
            .items()
            .iter()
            .find(|candidate| &candidate.id == mission)
            .map(|candidate| candidate.title.clone())
            .unwrap_or_else(|| mission.trim_start_matches("mission/").to_owned());
        link(
            &mut doc,
            "   mission ",
            &format!("{title} › {step}"),
            mission,
        );
    }
    doc
}

// ----------------------------------------------------------------- missions

pub fn mission_order(world: &World, system: bool) -> Vec<&Mission> {
    let mut missions = world
        .missions
        .items()
        .iter()
        .filter(|mission| system || !hidden_by_default(mission))
        .collect::<Vec<_>>();
    missions.sort_by_key(|mission| (display_rank(mission.word), mission.title.to_lowercase()));
    missions
}

/// The order the list reads in: what is running first, then what waits on the person, then what
/// is stuck, then what is paused, then what is finished (Nathan, 2026-10-11: the list was hard to
/// navigate, with running work between stuck work and the paused).
fn display_rank(word: Word) -> u8 {
    match word {
        Word::Working => 0,
        Word::Queued => 1,
        Word::Decision => 2,
        Word::Stalled => 3,
        Word::Unstaffed => 4,
        Word::Unclaimed => 5,
        Word::Watching => 6,
        Word::Held => 7,
        Word::Idle => 8,
        Word::Failed => 9,
        Word::Cancelled => 10,
        Word::Done => 11,
        Word::NotStarted => 12,
        Word::Unknown => 13,
    }
}

/// Missions a person rarely looks for: st's own, ones nobody has started, and failures from before
/// today, which age out of the list (Nathan, 2026-10-02). `x` shows them.
fn hidden_by_default(mission: &Mission) -> bool {
    mission.system
        || mission.word == Word::NotStarted
        || failed_before_today(mission)
        || finished_before_today(mission)
}

/// A mission that finished or was stopped before today: done is not news the next day.
fn finished_before_today(mission: &Mission) -> bool {
    matches!(mission.word, Word::Done | Word::Cancelled)
        && chrono::DateTime::parse_from_rfc3339(&mission.updated_at).is_ok_and(|at| {
            at.with_timezone(&chrono::Local).date_naive() < chrono::Local::now().date_naive()
        })
}

/// A failed mission whose last change was before today, on this machine's clock. A failure with
/// no known time stays in view.
fn failed_before_today(mission: &Mission) -> bool {
    mission.word == Word::Failed
        && chrono::DateTime::parse_from_rfc3339(&mission.updated_at).is_ok_and(|at| {
            at.with_timezone(&chrono::Local).date_naive() < chrono::Local::now().date_naive()
        })
}

/// What a step in hand last reported, on one line: the status of a run in progress.
fn mission_progress(mission: &Mission) -> Option<String> {
    mission
        .steps
        .iter()
        .filter(|step| step.state == StepState::Working)
        .find_map(|step| mission.step_metadata.get(&step.id)?.last_progress.as_deref())
        .map(|progress| progress.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|progress| !progress.is_empty())
}

pub fn missions_list(world: &World, width: usize, spinner: &'static str, system: bool) -> Listing {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    let missions = mission_order(world, system);
    let mut current = None;
    for mission in &missions {
        if current != Some(mission.word) {
            current = Some(mission.word);
            let (_, color) = word_style(mission.word, spinner);
            items.push(Item::Header {
                title: human_mission_label(mission),
                count: missions
                    .iter()
                    .filter(|other| other.word == mission.word)
                    .count(),
                color,
            });
        }
        let (glyph, color) = word_style(mission.word, spinner);
        let (done, total) = mission.progress();
        // st reports only current steps for many runs, so a 0/1 meter would claim too much.
        let right = if mission.word == Word::NotStarted {
            vec![]
        } else if done > 0 {
            let mut right = meter(done, total, 5, color);
            right.push(span(format!(" {done}/{total}"), theme::dim()));
            right
        } else {
            vec![span(
                format!("{total} step{}", if total == 1 { "" } else { "s" }),
                theme::dim(),
            )]
        };
        items.push(Item::Row {
            index: ids.len(),
            first: vec![
                span(format!(" {glyph} "), theme::strong(color)),
                span(mission.title.clone(), theme::bold()),
            ],
            right,
            // A run in progress says what its step last reported before where it lives.
            second: match mission_progress(mission) {
                Some(progress) => vec![span(
                    format!("   {}", text::truncate(&progress, width.saturating_sub(4))),
                    theme::soft(),
                )],
                None => vec![span(
                    format!(
                        "   {} · {}",
                        mission.id.trim_start_matches("mission/"),
                        mission.age
                    ),
                    theme::dim(),
                )],
            },
            children: Vec::new(),
        });
        ids.push(mission.id.clone());
    }
    if !system {
        let count = |hidden: fn(&Mission) -> bool| {
            world
                .missions
                .items()
                .iter()
                .filter(|mission| hidden(mission))
                .count()
        };
        let kinds = [
            (count(|mission| mission.system), "from Fractal"),
            (
                count(|mission| !mission.system && mission.word == Word::NotStarted),
                "not started",
            ),
            (
                count(|mission| !mission.system && failed_before_today(mission)),
                "failed before today",
            ),
            (
                count(|mission| !mission.system && finished_before_today(mission)),
                "finished before today",
            ),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, kind)| format!("{count} {kind}"))
        .collect::<Vec<_>>();
        if !kinds.is_empty() {
            items.push(Item::Note(Line::from(span(
                format!(" hidden: {} · x shows them", kinds.join(", ")),
                theme::dim(),
            ))));
        }
    }
    Listing {
        items,
        ids,
        state: state_of(&world.missions, "Loading missions…", "No missions yet."),
        legend: legend(&[
            ("◆", theme::PERSON, "needs you"),
            ("▲", theme::FAULT, "stalled"),
            ("◇", theme::WAITING, "unstaffed"),
            ("◌", theme::OVERLAY1, "queued"),
            ("◉", theme::SAPPHIRE, "watching"),
            (spinner, theme::WORKING, "working"),
            ("●", theme::IDLE, "idle"),
            ("✓", theme::DONE, "done"),
        ]),
    }
}

pub fn mission_detail(
    world: &World,
    id: Option<&str>,
    width: usize,
    spinner: &'static str,
    decision_card: Option<Doc>,
    expanded: &std::collections::HashSet<String>,
) -> Doc {
    let mut doc = Doc::new();
    let Some(mission) = id.and_then(|id| {
        world
            .missions
            .items()
            .iter()
            .find(|mission| mission.id == id)
    }) else {
        doc.line(Line::from(span(
            match world.missions {
                Load::Loading => "Loading missions…",
                _ => "Select a mission.",
            },
            theme::dim(),
        )));
        return doc;
    };
    let (glyph, color) = word_style(mission.word, spinner);
    let inner = width.saturating_sub(4);
    doc.line(Line::from(vec![
        span(
            format!(" {glyph} {} ", human_mission_label(mission)),
            Style::default()
                .fg(theme::CRUST)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        span(format!("  {}", mission.title), theme::bold()),
        span(
            if mission.host.is_empty() {
                format!("   {}", mission.age)
            } else {
                format!("   on {} · {}", mission.host, mission.age)
            },
            theme::dim(),
        ),
    ]));
    doc.line(Line::from(vec![
        span(format!(" {}", mission.id), theme::dim()),
        span(format!("  ·  {}", super::adapt::human_presentation(mission.word.explain())), theme::fg(color)),
    ]));
    if let Some(outcome) = &mission.outcome {
        doc.lines(text::wrap(
            &text::inline(outcome, theme::text()),
            inner,
            &[run(" outcome ", theme::dim())],
            &[run("         ", theme::dim())],
            None,
        ));
    }
    doc.blank();
    let mut goals = Doc::new();
    for goal in &mission.goals {
        goals.lines(text::wrap(
            &text::inline(goal, theme::text()),
            inner,
            &[run("◆ ", theme::fg(theme::LAVENDER))],
            &[run("  ", theme::dim())],
            None,
        ));
    }
    if !goals.lines.is_empty() {
        doc.card("goals", theme::LAVENDER, false, goals, width);
    }
    if let Some(card) = decision_card {
        // The same card Home shows, so the answer can be given right here.
        doc.append(card, 0);
    }
    what_you_can_do(&mut doc, world, mission, width, spinner);
    let steps = fractal_mission_ui::steps(
        mission,
        inner,
        spinner,
        expanded,
        &fractal_mission_ui::Palette::default(),
        &fractal_mission_ui::Glyphs::default(),
        &theme::conversation(),
    );
    let steps = Doc {
        lines: steps.lines,
        targets: steps
            .targets
            .into_iter()
            .map(|target| super::doc::Target {
                line: target.line,
                column: target.column,
                width: target.width,
                hit: match target.hit {
                    fractal_mission_ui::Intent::ToggleStep(key) => Hit::ToggleTool(key),
                },
            })
            .collect(),
        messages: steps.messages,
        entries: steps.entries,
    };
    doc.card("steps", theme::OVERLAY1, false, steps, width);
    let mut agents = Doc::new();
    for id in &mission.agents {
        if let Some(agent) = world.agents.items().iter().find(|agent| &agent.id == id) {
            let (glyph, color) = agent_glyph(agent.state, spinner);
            let label = format!("{glyph} {:<18} ", text::truncate(&agent.name, 18));
            agents.targets.push(super::doc::Target {
                line: agents.lines.len(),
                column: 0,
                width: text::width(&label) as u16,
                hit: Hit::Peek(agent.id.clone()),
            });
            agents.line(Line::from(vec![
                span(label, theme::fg(color)),
                span(format!("{:<11}", agent_shown_word(agent)), theme::fg(color)),
                span(
                    format!("{:<8}", agent.harness.name()),
                    theme::fg(harness_color(agent.harness)),
                ),
                span(
                    format!("on {} · {}", agent.host, agent.activity),
                    theme::dim(),
                ),
            ]));
        }
    }
    if agents.lines.is_empty() {
        agents.line(Line::from(span("No agents right now.", theme::dim())));
    }
    doc.card(
        &format!("agents · {}", mission.agents.len()),
        theme::OVERLAY1,
        false,
        agents,
        width,
    );
    if let Some(tree) = &mission.worktree {
        let mut place = Doc::new();
        place.line(Line::from(vec![
            span(tree.clone(), theme::text()),
            span(format!("  on {}", mission.host), theme::dim()),
        ]));
        doc.card("worktree", theme::OVERLAY1, false, place, width);
    }
    doc.buttons(&[("k", "The whole declaration", Hit::Key('k'), theme::OVERLAY1)]);
    doc
}

/// The mission as written. st does not send the declaration to clients yet.
pub fn mission_kdl(world: &World, id: Option<&str>, width: usize) -> Doc {
    let mut doc = Doc::new();
    let mission = id.and_then(|id| {
        world
            .missions
            .items()
            .iter()
            .find(|mission| mission.id == id)
    });
    doc.line(Line::from(vec![
        span(
            " declaration ",
            Style::default()
                .fg(theme::CRUST)
                .bg(theme::OVERLAY1)
                .add_modifier(Modifier::BOLD),
        ),
        span(
            format!(
                "  {}",
                mission.map(|mission| mission.id.as_str()).unwrap_or("")
            ),
            theme::dim(),
        ),
    ]));
    doc.buttons(&[("k", "Back to the mission", Hit::Key('k'), theme::ACCENT)]);
    doc.blank();
    match mission.and_then(|mission| mission.kdl.as_deref()) {
        Some(kdl) => {
            for line in text::sanitize(kdl).lines() {
                let trimmed = line.trim_start();
                let style = if trimmed.starts_with("//") {
                    theme::dim()
                } else if trimmed.starts_with("goal") || trimmed.starts_with("constraint") {
                    theme::fg(theme::LAVENDER)
                } else if trimmed.starts_with("gate") || trimmed.starts_with("reviewer") || trimmed.starts_with("question") {
                    theme::fg(theme::PERSON)
                } else if trimmed.starts_with("step") || trimmed.starts_with("mission") {
                    theme::strong(theme::PEACH)
                } else {
                    theme::fg(theme::SUBTEXT1)
                };
                doc.lines(text::wrap(&[run(line.to_owned(), style)], width, &[], &[run("    ", theme::dim())], None));
            }
        }
        None => doc.wrap(
            &[run(
                "Fractal does not send mission declarations to clients yet. Until it does, read it with: fractal missions show",
                theme::dim(),
            )],
            width,
        ),
    }
    doc
}

/// For a mission that is not moving: why, and what a person can do about it.
fn what_you_can_do(
    doc: &mut Doc,
    world: &World,
    mission: &Mission,
    width: usize,
    spinner: &'static str,
) {
    let inner = width.saturating_sub(4);
    let mut card = Doc::new();
    let broken = mission
        .agents
        .iter()
        .filter_map(|id| world.agents.items().iter().find(|agent| &agent.id == id))
        .find(|agent| matches!(agent.state, AgentState::Fault | AgentState::Stopped));
    let stuck = mission.steps.iter().find(|step| {
        matches!(
            step.state,
            StepState::Failed | StepState::Ready | StepState::Waiting
        )
    });
    let (title, color) = match mission.word {
        Word::Stalled | Word::Failed => ("what you can do", theme::FAULT),
        Word::Unstaffed => ("what you can do", theme::WAITING),
        // Between two steps the run is moving: say what is next, not that the person has no part.
        Word::Queued => ("up next", theme::OVERLAY1),
        _ => return,
    };
    card.blank();
    if mission.word == Word::Queued {
        let note = stuck
            .and_then(|step| step.note.clone())
            .or_else(|| stuck.map(|step| format!("{} is next.", step.name)))
            .unwrap_or_else(|| "The next step is waiting its turn.".into());
        card.wrap(&text::inline(&note, theme::text()), inner);
        card.blank();
        card.wrap(
            &[run(
                "It starts by itself when the agent is free.",
                theme::dim(),
            )],
            inner,
        );
        card.blank();
        doc.card(title, color, false, card, width);
        return;
    }
    if let Some(step) = stuck {
        let (glyph, step_color, word) = step_style(step.state, spinner);
        card.line(Line::from(vec![
            span(format!("{glyph} {} ", step.name), theme::strong(step_color)),
            span(
                match step.state {
                    StepState::Failed => "failed".to_owned(),
                    _ => format!("is {word}"),
                },
                theme::fg(step_color),
            ),
            span(
                if step.attempt > 1 {
                    format!(" after {} attempts", step.attempt)
                } else {
                    String::new()
                },
                theme::dim(),
            ),
        ]));
        for blocker in &step.blockers {
            card.wrap(&[run(format!("because {blocker}"), theme::soft())], inner);
        }
        card.blank();
    }
    if let Some(agent) = broken {
        card.wrap(
            &[run(
                format!(
                    "{} is {}. Fix the agent and the step can run again.",
                    agent.name,
                    screens_word(agent.state)
                ),
                theme::text(),
            )],
            inner,
        );
        card.blank();
        card.buttons(&[
            ("R", "Restart the agent", Hit::Key('R'), theme::GREEN),
            (
                "t",
                "Chat with it",
                Hit::Peek(agent.id.clone()),
                theme::SAPPHIRE,
            ),
        ]);
    }
    card.buttons(&[
        ("r", "Retry the step", Hit::Key('r'), theme::YELLOW),
        ("X", "Cancel this run", Hit::Key('X'), theme::RED),
    ]);
    card.blank();
    doc.card(title, color, true, card, width);
}

fn screens_word(state: AgentState) -> &'static str {
    agent_word(state)
}

// -------------------------------------------------------------------- fleet

pub fn fleet_list(world: &World) -> Listing {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    if world.inventory_supported == Some(false) {
        items.push(Item::Note(Line::from(span(" This member’s inventories are read, not followed.", theme::dim()))));
    }
    if world.inventory_partial.iter().any(|name| ["clients", "machines", "devices", "sessions"].contains(&name.as_str())) {
        items.push(Item::Note(Line::from(span(" This view is partial; more inventory rows exist beyond its window.", theme::dim()))));
    }
    for machine in world.machines.items() {
        let (glyph, color) = reach_style(machine.reach);
        let agents = world
            .agents
            .items()
            .iter()
            .filter(|agent| agent.host == machine.name)
            .count();
        items.push(Item::Row {
            index: ids.len(),
            first: vec![
                span(format!(" {glyph} "), theme::strong(color)),
                span(machine.name.clone(), theme::bold()),
                span(
                    if machine.you_are_here {
                        "  you are here".to_owned()
                    } else {
                        String::new()
                    },
                    theme::fg(theme::ACCENT),
                ),
            ],
            right: vec![span(
                format!("{agents} agent{}", if agents == 1 { "" } else { "s" }),
                theme::dim(),
            )],
            second: vec![span(format!("   {}", machine_line(machine)), theme::dim())],
            children: Vec::new(),
        });
        ids.push(machine.name.clone());
    }
    Listing {
        items,
        ids,
        state: state_of(&world.machines, "Loading machines…", "No machines yet."),
        legend: legend(&[
            ("●", theme::GREEN, "connected"),
            ("◐", theme::SAPPHIRE, "through another"),
            ("○", theme::RED, "offline"),
        ]),
    }
}

pub fn reach_style(reach: Reach) -> (&'static str, Color) {
    match reach {
        Reach::Here | Reach::Direct => ("●", theme::GREEN),
        Reach::Indirect => ("◐", theme::SAPPHIRE),
        Reach::Offline => ("○", theme::RED),
        Reach::Unknown => ("◌", theme::OVERLAY1),
    }
}

/// How a machine is reached and when it was last heard from, in words.
fn machine_line(machine: &Machine) -> String {
    let mut parts = vec![machine.reach.word().to_owned()];
    if !machine.platform.is_empty() {
        parts.push(machine.platform.clone());
    }
    match (machine.reach, machine.seen.as_str()) {
        (Reach::Here, _) => {}
        (_, "never") => parts.push("never heard from".into()),
        (_, seen) => parts.push(format!("heard {seen}")),
    }
    parts.join(" · ")
}

pub fn fleet_detail(world: &World, id: Option<&str>, width: usize, spinner: &'static str) -> Doc {
    let mut doc = Doc::new();
    let Some(machine) = id.and_then(|id| {
        world
            .machines
            .items()
            .iter()
            .find(|machine| machine.name == id)
    }) else {
        doc.line(Line::from(span("Select a machine.", theme::dim())));
        return doc;
    };
    let (glyph, color) = reach_style(machine.reach);
    doc.line(Line::from(vec![
        span(format!(" {glyph} "), theme::strong(color)),
        span(machine.name.clone(), theme::bold()),
        span(format!("  {}", machine_line(machine)), theme::dim()),
    ]));
    if let Some(load) = &machine.load {
        doc.line(Line::from(span(format!("   {load}"), theme::soft())));
    }
    doc.blank();
    let mut links = Doc::new();
    for (peer, up, detail) in &machine.links {
        links.line(Line::from(vec![
            span(
                if *up { "● " } else { "○ " },
                theme::fg(if *up { theme::GREEN } else { theme::RED }),
            ),
            span(format!("{peer:<12}"), theme::text()),
            span(detail.clone(), theme::dim()),
        ]));
    }
    if links.lines.is_empty() {
        links.line(Line::from(span(
            if machine.you_are_here {
                "This is the machine the Fractal terminal UI is talking to."
            } else {
                "Fractal reports no link to this machine."
            },
            theme::dim(),
        )));
    }
    doc.card("reaches", theme::OVERLAY1, false, links, width);
    let mut agents = Doc::new();
    for agent in agent_order(world)
        .into_iter()
        .filter(|agent| agent.host == machine.name)
    {
        let (glyph, color) = agent_glyph(agent.state, spinner);
        let label = format!("{glyph} {:<20} ", text::truncate(&agent.name, 20));
        agents.targets.push(super::doc::Target {
            line: agents.lines.len(),
            column: 0,
            width: text::width(&label) as u16,
            hit: Hit::Peek(agent.id.clone()),
        });
        agents.line(Line::from(vec![
            span(label, theme::fg(color)),
            span(
                format!("{:<8}", agent.harness.name()),
                theme::fg(harness_color(agent.harness)),
            ),
            span(agent.worktree.clone().unwrap_or_default(), theme::dim()),
        ]));
    }
    if agents.lines.is_empty() {
        agents.line(Line::from(span("No agents on this machine.", theme::dim())));
    }
    doc.card("agents here", theme::OVERLAY1, false, agents, width);
    if machine.you_are_here {
        doc.append(clients_card(world, width), 0);
        doc.append(devices_card(world, width), 0);
    }
    doc
}

// ------------------------------------------------------------------ clients

/// Who is connected to this member: each client as it describes itself, who it acts for, how
/// it came in, since when, and what it follows. A build older than the member's own carries a
/// quiet note, nothing more.
pub fn clients_card(world: &World, width: usize) -> Doc {
    let mut inner = Doc::new();
    match &world.clients {
        Load::Loading => inner.line(Line::from(span("Loading connected clients…", theme::dim()))),
        Load::Failed(error) => inner.wrap(
            &[run(
                format!("Could not read connected clients: {error}"),
                theme::fg(theme::RED),
            )],
            width.saturating_sub(4),
        ),
        Load::Ready(clients) if clients.is_empty() => {
            inner.line(Line::from(span("No clients connected.", theme::dim())))
        }
        Load::Ready(clients) => {
            for item in clients {
                let name = if item.client.is_empty() {
                    "unnamed client"
                } else {
                    item.client.as_str()
                };
                let head = vec![
                    span(
                        if item.connected { "● " } else { "○ " },
                        theme::fg(if item.connected {
                            theme::GREEN
                        } else {
                            theme::QUIET
                        }),
                    ),
                    span(name.to_owned(), theme::text()),
                ];
                inner.line(Line::from(head));
                // Who, how it came in, what it follows and the quiet "older" note wrap under the
                // name, so nothing clips at the card's edge.
                let mut about = vec![match &item.device {
                    Some(device) => format!("{} · {device}", item.who),
                    None => item.who.clone(),
                }];
                about.extend([item.via.clone(), item.member.clone(), item.when.clone()]);
                if !item.follows.is_empty() {
                    about.push(format!("follows {}", item.follows.join(", ")));
                }
                if item.older {
                    about.push("older than this member".to_owned());
                }
                inner.wrap(
                    &[run(format!("  {}", about.join(" · ")), theme::dim())],
                    width.saturating_sub(4),
                );
            }
        }
    }
    let mut doc = Doc::new();
    doc.card("connected clients", theme::OVERLAY1, false, inner, width);
    doc
}

// ---------------------------------------------------------------- worktrees

pub fn worktrees_list(world: &World) -> Listing {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    items.push(Item::Note(Line::from(vec![
        span(
            " DEMO ",
            Style::default()
                .fg(theme::CRUST)
                .bg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        span(" invented worktrees", theme::fg(theme::YELLOW)),
    ])));
    let mut trees = world.worktrees.items().iter().collect::<Vec<_>>();
    trees.sort_by(|a, b| (&a.host, &a.path).cmp(&(&b.host, &b.path)));
    let mut current = "";
    for tree in trees {
        if tree.host != current {
            current = &tree.host;
            items.push(Item::Header {
                title: format!("on {}", tree.host),
                count: world
                    .worktrees
                    .items()
                    .iter()
                    .filter(|other| other.host == tree.host)
                    .count(),
                color: theme::OVERLAY1,
            });
        }
        let mut right = Vec::new();
        if tree.ahead > 0 {
            right.push(span(format!("↑{} ", tree.ahead), theme::fg(theme::GREEN)));
        }
        if tree.behind > 0 {
            right.push(span(format!("↓{} ", tree.behind), theme::fg(theme::RED)));
        }
        if tree.dirty > 0 {
            right.push(span(format!("±{}", tree.dirty), theme::fg(theme::YELLOW)));
        }
        items.push(Item::Row {
            index: ids.len(),
            first: vec![
                span("  ", theme::dim()),
                span(tree.branch.clone(), theme::strong(theme::LAVENDER)),
            ],
            right,
            second: vec![span(
                format!("  {} · {} agents", tree.path, tree.agents.len()),
                theme::dim(),
            )],
            children: Vec::new(),
        });
        ids.push(format!("{}:{}", tree.host, tree.path));
    }
    Listing {
        items,
        ids,
        state: state_of(
            &world.worktrees,
            "Loading worktrees…",
            "No worktrees known.",
        ),
        legend: legend(&[
            ("↑", theme::GREEN, "ahead"),
            ("↓", theme::RED, "behind"),
            ("±", theme::YELLOW, "changed"),
        ]),
    }
}

pub fn worktree_detail(
    world: &World,
    id: Option<&str>,
    width: usize,
    spinner: &'static str,
) -> Doc {
    let mut doc = Doc::new();
    demo_banner(&mut doc, width);
    let Some(tree) = id.and_then(|id| {
        world
            .worktrees
            .items()
            .iter()
            .find(|tree| format!("{}:{}", tree.host, tree.path) == id)
    }) else {
        doc.line(Line::from(span("Select a worktree.", theme::dim())));
        return doc;
    };
    doc.line(Line::from(vec![
        span(format!(" {}", tree.branch), theme::strong(theme::LAVENDER)),
        span(format!("  {} on {}", tree.path, tree.host), theme::dim()),
    ]));
    doc.line(Line::from(span(
        format!(
            "   {} ahead · {} behind · {} changed files",
            tree.ahead, tree.behind, tree.dirty
        ),
        theme::soft(),
    )));
    doc.blank();
    let mut agents = Doc::new();
    for id in &tree.agents {
        if let Some(agent) = world.agents.items().iter().find(|agent| &agent.id == id) {
            let (glyph, color) = agent_glyph(agent.state, spinner);
            let label = format!("{glyph} {:<20} ", text::truncate(&agent.name, 20));
            agents.targets.push(super::doc::Target {
                line: agents.lines.len(),
                column: 0,
                width: text::width(&label) as u16,
                hit: Hit::Peek(agent.id.clone()),
            });
            agents.line(Line::from(vec![
                span(label, theme::fg(color)),
                span(agent_shown_word(agent), theme::fg(color)),
            ]));
        }
    }
    if agents.lines.is_empty() {
        agents.line(Line::from(span("Nobody is working here.", theme::dim())));
    }
    doc.card("agents here", theme::OVERLAY1, false, agents, width);
    let mut missions = Doc::new();
    for id in &tree.missions {
        if let Some(mission) = world
            .missions
            .items()
            .iter()
            .find(|mission| &mission.id == id)
        {
            let (glyph, color) = word_style(mission.word, spinner);
            let label = format!("{glyph} {:<26}", mission.title);
            missions.targets.push(super::doc::Target {
                line: missions.lines.len(),
                column: 0,
                width: text::width(&label) as u16,
                hit: Hit::Peek(mission.id.clone()),
            });
            missions.line(Line::from(vec![
                span(label, theme::fg(color)),
                span(human_mission_label(mission), theme::fg(color)),
            ]));
        }
    }
    if missions.lines.is_empty() {
        missions.line(Line::from(span(
            "No missions use this worktree.",
            theme::dim(),
        )));
    }
    doc.card("missions here", theme::OVERLAY1, false, missions, width);
    doc
}

/// Every worktree screen starts with this: st does not track worktrees yet.
fn demo_banner(doc: &mut Doc, width: usize) {
    doc.lines(text::wrap(
        &[run(
            "Fractal does not track worktrees yet. Everything on this tab is invented.",
            theme::fg(theme::YELLOW),
        )],
        width,
        &[
            run(
                " DEMO DATA ",
                Style::default()
                    .fg(theme::CRUST)
                    .bg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            run(" ", theme::dim()),
        ],
        &[run("            ", theme::dim())],
        None,
    ));
    doc.blank();
}

// ------------------------------------------------------------------ popovers

/// The chord (with Ctrl) that opens an agent's actions menu from its conversation.
pub const ACTIONS_CHORD: char = 'a';
/// The actions menu's key for attaching the agent's terminal in its tab.
pub const TERMINAL_ACTION: char = 't';

/// How to get to an agent's terminal from where its conversation shows, said as the keys that
/// do it: the actions menu, then its Terminal row. The Attach button lives only on a detached
/// terminal pane, where Enter does the same.
pub fn terminal_route() -> String {
    format!(
        "Ctrl+{} then {TERMINAL_ACTION} from the conversation",
        ACTIONS_CHORD.to_ascii_uppercase()
    )
}

/// The small card a clicked reference opens: enough to recognise the subject, and a way
/// to go to it.
/// One row of the agent actions menu: its key, what it says, and what it does.
pub struct AgentAction {
    pub key: char,
    pub label: &'static str,
    pub what: &'static str,
}

/// What can be done to `agent` now, in the order the menu lists it (Nathan, 2026-10-04: make
/// restart, retire and the rest discoverable). Ones that change the seat ask y first.
pub fn agent_actions(agent: &Agent) -> Vec<AgentAction> {
    let row = |key, label, what| AgentAction { key, label, what };
    let mut rows = Vec::new();
    let retired = agent.state == AgentState::Retired;
    let running = !matches!(agent.state, AgentState::Stopped | AgentState::Retired | AgentState::Unknown);
    if agent.state == AgentState::Working {
        rows.push(row('i', "Interrupt", "stop its current turn, as Esc does in its own screen"));
    }
    if running {
        rows.push(row('r', "Restart", "a new process for the same seat, on its declaration"));
        rows.push(row('p', "Suspend", "stop at a quiet moment, keeping its session to resume"));
        rows.push(row('x', "Stop", "stop its process; start brings it back"));
    } else if !retired {
        rows.push(row('s', "Start", "start it again on its declaration"));
        rows.push(row('u', "Resume", "resume a suspended seat on the same session"));
    }
    if agent.terminal {
        rows.push(row(
            TERMINAL_ACTION,
            "Terminal",
            "attach its terminal in this tab",
        ));
    }
    rows.push(row('d', "Details", "what it is doing and what is queued (ctrl+d too)"));
    rows.push(row('f', "Find", "find in its conversation (ctrl+f too)"));
    rows.push(row('c', "Copy id", "copy its id to the clipboard"));
    rows
}

/// The agent actions menu's card.
pub fn agent_actions_doc(agent: &Agent, width: usize, spinner: &'static str) -> Doc {
    let mut doc = Doc::new();
    doc.blank();
    let (glyph, color) = agent_glyph(agent.state, spinner);
    doc.line(Line::from(vec![
        span(format!("{glyph} "), theme::strong(color)),
        span(agent.name.clone(), theme::bold()),
        span(format!("  {}", agent_shown_word(agent)), theme::fg(color)),
        span(format!("  {} · {}", agent.harness.name(), agent.host), theme::dim()),
    ]));
    doc.blank();
    for action in agent_actions(agent) {
        let line = doc.lines.len();
        let label = format!("{:<10}", action.label);
        doc.line(Line::from(vec![
            span(format!(" {} ", action.key), theme::strong(theme::CRUST).bg(theme::ACCENT)),
            span(format!(" {label}"), theme::bold()),
            span(text::truncate(action.what, width.saturating_sub(16)), theme::dim()),
        ]));
        doc.targets.push(super::doc::Target {
            line,
            column: 0,
            width: width.min(u16::MAX as usize) as u16,
            hit: Hit::Key(action.key),
        });
    }
    doc.blank();
    doc.wrap(
        &text::inline(
            "Interrupt, restart, suspend and retire ask y first. Esc closes.",
            theme::dim(),
        ),
        width,
    );
    doc
}

pub fn peek(world: &World, subject: &str, width: usize, spinner: &'static str) -> Doc {
    let mut doc = Doc::new();
    doc.blank();
    if let Some(agent) = world
        .agents
        .items()
        .iter()
        .find(|agent| agent.id == subject)
    {
        let (glyph, color) = agent_glyph(agent.state, spinner);
        doc.line(Line::from(vec![
            span(format!("{glyph} "), theme::strong(color)),
            span(agent.name.clone(), theme::bold()),
            span(format!("  {}", agent_shown_word(agent)), theme::fg(color)),
        ]));
        doc.line(Line::from(span(agent.id.clone(), theme::dim())));
        doc.blank();
        doc.field(
            "harness",
            agent.harness.name(),
            width,
            theme::fg(harness_color(agent.harness)),
        );
        doc.field("host", &agent.host, width, theme::soft());
        if let Some(custody) = &agent.details.mail_custody {
            doc.field("mail", custody, width, theme::text());
        }
        if let Some(tree) = &agent.worktree {
            doc.field("worktree", tree, width, theme::soft());
        }
        if let Some(progress) = &agent.details.progress {
            doc.field("now", progress, width, theme::text());
        }
        if let (Some(mission), Some(step)) = (&agent.mission, &agent.step) {
            let title = world
                .missions
                .items()
                .iter()
                .find(|candidate| &candidate.id == mission)
                .map(|candidate| candidate.title.clone())
                .unwrap_or_else(|| mission.trim_start_matches("mission/").to_owned());
            doc.field("doing", &format!("{title} › {step}"), width, theme::text());
        }
        doc.field(
            "last seen",
            &format!("{} ago", agent.activity),
            width,
            theme::soft(),
        );
        doc.blank();
        if agent.unmanaged {
            doc.buttons(&[("g", "Go to agent", Hit::Key('g'), theme::ACCENT)]);
        } else {
            doc.buttons(&[
                ("g", "Go to agent", Hit::Key('g'), theme::ACCENT),
                ("t", "Message", Hit::Key('t'), theme::SAPPHIRE),
            ]);
        }
    } else if let Some(mission) = world
        .missions
        .items()
        .iter()
        .find(|mission| mission.id == subject)
    {
        let (glyph, color) = word_style(mission.word, spinner);
        doc.line(Line::from(vec![
            span(format!("{glyph} "), theme::strong(color)),
            span(mission.title.clone(), theme::bold()),
            span(format!("  {}", human_mission_label(mission)), theme::fg(color)),
        ]));
        doc.line(Line::from(span(mission.id.clone(), theme::dim())));
        doc.line(Line::from(span(super::adapt::human_presentation(mission.word.explain()).into_owned(), theme::fg(color))));
        doc.blank();
        let (done, total) = mission.progress();
        doc.field(
            "steps",
            &format!("{done} of {total} done"),
            width,
            theme::soft(),
        );
        for step in mission
            .steps
            .iter()
            .filter(|step| {
                !matches!(
                    step.state,
                    StepState::Done | StepState::Cancelled | StepState::Pending
                )
            })
            .take(3)
        {
            let (glyph, color, word) = step_style(step.state, spinner);
            doc.line(Line::from(vec![
                span(format!("          {glyph} "), theme::fg(color)),
                span(format!("{} ", step.name), theme::text()),
                span(word, theme::fg(color)),
                span(
                    format!("  {}", step.owner_identity().as_deref().unwrap_or("nobody")),
                    theme::dim(),
                ),
            ]));
        }
        let names = mission
            .agents
            .iter()
            .filter_map(|id| world.agents.items().iter().find(|agent| &agent.id == id))
            .map(|agent| agent.name.clone())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            doc.field("agents", &names.join(", "), width, theme::soft());
        }
        doc.blank();
        doc.buttons(&[("g", "Go to mission", Hit::Key('g'), theme::ACCENT)]);
    } else if let Some(item) = world
        .attention
        .items()
        .iter()
        .find(|item| item.id == subject)
    {
        let (glyph, color) = attention_style(&item.kind);
        doc.line(Line::from(vec![
            span(format!("{glyph} {} ", item.kind.word()), theme::fg(color)),
            span(item.title.clone(), theme::bold()),
        ]));
        doc.line(Line::from(span(
            format!("waiting {}", item.age),
            theme::dim(),
        )));
        doc.blank();
        doc.buttons(&[("g", "Open in Inbox", Hit::Key('g'), theme::PERSON)]);
    } else {
        doc.wrap(
            &[run(
                format!("{subject} is not in the current view."),
                theme::dim(),
            )],
            width,
        );
    }
    doc.blank();
    doc
}

// -------------------------------------------------------------- agent details

/// The pane beside a conversation: what the agent holds, what is next, and how it runs.
pub fn agent_details(world: &World, agent: &Agent, width: usize, spinner: &'static str) -> Doc {
    let mut doc = Doc::new();
    let details = &agent.details;
    let unknown = || span("Fractal has not said", theme::dim());
    // A close control on the pane itself, not only on the header rule.
    let close = "× close";
    doc.targets.push(super::doc::Target {
        line: 0,
        column: width.saturating_sub(text::width(close)) as u16,
        width: text::width(close) as u16,
        hit: Hit::Key('i'),
    });
    doc.line(Line::from(vec![
        span("details", theme::label()),
        span(
            " ".repeat(width.saturating_sub(7 + text::width(close))),
            theme::dim(),
        ),
        span(close, theme::fg(theme::OVERLAY1)),
    ]));
    if let Some(fault) = &details.fault {
        let mut inner = Doc::new();
        inner.wrap(&text::inline(&super::adapt::human_presentation(fault), theme::text()), width.saturating_sub(4));
        doc.card("broken", theme::FAULT, true, inner, width);
        doc.blank();
    }
    if let Some(custody) = &details.mail_custody {
        doc.field("mail", custody, width, theme::text());
    }
    if agent.state == AgentState::NeedsLogin {
        let mut inner = Doc::new();
        inner.wrap(
            &text::inline(&login_guidance(agent), theme::text()),
            width.saturating_sub(4),
        );
        doc.card("needs login", theme::PERSON, true, inner, width);
        doc.blank();
    }
    doc.section("now", None, width);
    // What its step last reported is the status; the step and its goal say where it comes from.
    if let Some(progress) = &details.progress {
        doc.lines(text::wrap(
            &text::inline(progress, theme::bold()),
            width,
            &[],
            &[],
            None,
        ));
    }
    match (&agent.mission, &agent.step) {
        (Some(mission), Some(step)) => {
            let title = world
                .missions
                .items()
                .iter()
                .find(|candidate| &candidate.id == mission)
                .map(|candidate| candidate.title.clone())
                .unwrap_or_else(|| mission.trim_start_matches("mission/").to_owned());
            link(&mut doc, "", &format!("{title} › {step}"), mission);
            if let Some(goal) = &details.goal {
                doc.lines(text::wrap(
                    &text::inline(goal, theme::soft()),
                    width,
                    &[],
                    &[],
                    None,
                ));
            }
            if let Some(claimed) = &details.claimed {
                doc.line(Line::from(span(
                    format!("held since {claimed}"),
                    theme::dim(),
                )));
            }
        }
        _ => doc.line(Line::from(span("No step right now.", theme::dim()))),
    }
    doc.blank();
    doc.section("next", Some(details.queued as usize), width);
    // Each queued step opens its mission when clicked.
    let open_mission = |doc: &mut Doc, lines: Vec<Line<'static>>, mission: Option<&String>| {
        if let Some(mission) = mission.filter(|mission| !mission.is_empty()) {
            for offset in 0..lines.len() {
                doc.targets.push(super::doc::Target {
                    line: doc.lines.len() + offset,
                    column: 2,
                    width: width.saturating_sub(2) as u16,
                    hit: Hit::Open(mission.clone()),
                });
            }
        }
        doc.lines(lines);
    };
    match &details.next {
        Some(next) => open_mission(
            &mut doc,
            text::wrap(
                &[run(next.clone(), theme::text())],
                width,
                &[run("› ", theme::fg(theme::ACCENT))],
                &[run("  ", theme::dim())],
                None,
            ),
            details.next_mission.as_ref(),
        ),
        None => doc.line(Line::from(span("Nothing queued.", theme::dim()))),
    }
    for (index, item) in details.queue.iter().enumerate() {
        if Some(item) == details.next.as_ref() {
            continue;
        }
        open_mission(
            &mut doc,
            text::wrap(
                &[run(item.clone(), theme::soft())],
                width,
                &[run("· ", theme::dim())],
                &[run("  ", theme::dim())],
                None,
            ),
            details.queue_missions.get(index),
        );
    }
    if !agent.subagents.is_empty() {
        doc.blank();
        doc.section("subagents", Some(agent.subagents.len()), width);
        for subagent in &agent.subagents {
            let mut spans = vec![run(subagent.label(), theme::soft())];
            if !subagent.age.is_empty() {
                spans.push(run(format!(" · {}", subagent.age), theme::dim()));
            }
            doc.lines(text::wrap(
                &spans,
                width,
                &[run("· ", theme::dim())],
                &[run("  ", theme::dim())],
                None,
            ));
        }
    }
    doc.blank();
    doc.section("runs as", None, width);
    let (glyph, color) = agent_glyph(agent.state, spinner);
    doc.line(Line::from(vec![
        span(format!("{glyph} "), theme::fg(color)),
        span(agent_shown_word(agent), theme::fg(color)),
    ]));
    let field = |doc: &mut Doc, label: &str, value: Option<&str>| {
        let mut spans = vec![span(format!("{label:<9}"), theme::dim())];
        match value {
            Some(value) => spans.push(span(value.to_owned(), theme::soft())),
            None => spans.push(unknown()),
        }
        doc.line(Line::from(spans));
    };
    field(&mut doc, "harness", Some(agent.harness.name()));
    field(&mut doc, "model", details.model.as_deref());
    // Beside the model, only when the seat is configured with one: an unset effort is the
    // harness's own default, which is not worth a row of "unknown".
    if let Some(effort) = details.effort.as_deref() {
        field(&mut doc, "effort", Some(effort));
    }
    field(&mut doc, "state", details.harness_state.as_deref());
    field(&mut doc, "runtime", details.runtime.as_deref());
    field(&mut doc, "host", Some(&agent.host));
    field(&mut doc, "worktree", agent.worktree.as_deref());
    if let Some(under) = &details.under {
        field(&mut doc, "under", Some(under));
    }
    doc.blank();
    doc.lines(text::wrap(
        &[run(agent.id.clone(), theme::dim())],
        width,
        &[],
        &[],
        None,
    ));
    doc
}

// ---------------------------------------------------------------- tree views

/// A listing laid out as the graph's path tree: folders from the id, one line per leaf.
fn tree_listing(
    mut leaves: Vec<(Vec<String>, String, Vec<Span<'static>>, Vec<Span<'static>>)>,
    state: ListState,
    legend: Vec<Line<'static>>,
) -> Listing {
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let compact = compact_folders(&leaves.iter().map(|leaf| leaf.0.clone()).collect::<Vec<_>>());
    let mut items = Vec::new();
    let mut ids = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for ((_, id, first, right), folders) in leaves.into_iter().zip(compact) {
        let folders = &folders[..];
        let shared = open.iter().zip(folders).take_while(|(a, b)| a == b).count();
        open.truncate(shared);
        for (depth, folder) in folders.iter().enumerate().skip(shared) {
            items.push(Item::Folder(Line::from(vec![
                span(
                    format!(" {}▾ ", "  ".repeat(depth)),
                    theme::fg(theme::SURFACE2),
                ),
                span(format!("{folder}/"), theme::fg(theme::OVERLAY1)),
            ])));
            open.push(folder.clone());
        }
        let mut row = vec![span("  ".repeat(folders.len()), theme::dim())];
        row.extend(first);
        items.push(Item::Row {
            index: ids.len(),
            first: row,
            right,
            second: vec![],
            children: Vec::new(),
        });
        ids.push(id);
    }
    Listing {
        items,
        ids,
        state,
        legend,
    }
}

/// Each path's folders, with a folder that holds only one folder joined to it on one line
/// (Nathan, 2026-10-01): `fleet/smalltalk/operations/` rather than three nested folders.
fn compact_folders(paths: &[Vec<String>]) -> Vec<Vec<String>> {
    use std::collections::{BTreeMap, BTreeSet};
    // What each folder holds: its subfolders by name, and a leaf as the empty name.
    let mut holds = BTreeMap::<Vec<String>, BTreeSet<String>>::new();
    for path in paths {
        let folders = &path[..path.len().saturating_sub(1)];
        for depth in 0..folders.len() {
            let child = folders.get(depth + 1).cloned().unwrap_or_default();
            holds
                .entry(folders[..=depth].to_vec())
                .or_default()
                .insert(child);
        }
    }
    paths
        .iter()
        .map(|path| {
            let folders = &path[..path.len().saturating_sub(1)];
            let mut compact = Vec::new();
            let mut depth = 0;
            while depth < folders.len() {
                let mut name = folders[depth].clone();
                while depth + 1 < folders.len()
                    && holds
                        .get(&folders[..=depth])
                        .is_some_and(|children| children.len() == 1)
                {
                    depth += 1;
                    name = format!("{name}/{}", folders[depth]);
                }
                compact.push(name);
                depth += 1;
            }
            compact
        })
        .collect()
}

pub fn agents_tree(world: &World, spinner: &'static str, width: usize) -> Listing {
    let leaves = agent_order(world)
        .into_iter()
        .map(|agent| {
            let (glyph, color) = agent_glyph(agent.state, spinner);
            let path = agent
                .id
                .trim_start_matches("agent/")
                .split('/')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (
                path,
                agent.id.clone(),
                vec![
                    span(format!("{glyph} "), theme::strong(color)),
                    span(agent.name.clone(), theme::bold()),
                ],
                agent_row_right(world, agent),
            )
        })
        .collect();
    let mut listing = tree_listing(
        leaves,
        state_of(&world.agents, "Loading agents…", "No agents yet."),
        vec![],
    );
    // Each agent's subagents hang beneath its leaf, indented past its glyph.
    let agents = world.agents.items();
    for item in &mut listing.items {
        if let Item::Row {
            index,
            first,
            right,
            children,
            ..
        } = item
            && let Some(agent) = agents.iter().find(|agent| agent.id == listing.ids[*index])
        {
            let indent = format!("{}  ", first.first().map_or("", |span| &*span.content));
            mark_standing(agent, first, right, width);
            *children = subagent_lines(agent, &indent, width);
        }
    }
    listing.legend = agents_list(world, spinner, 40).legend;
    listing
}

pub fn missions_tree(world: &World, spinner: &'static str, system: bool) -> Listing {
    let leaves = mission_order(world, system)
        .into_iter()
        .map(|mission| {
            let (glyph, color) = word_style(mission.word, spinner);
            let path = mission
                .id
                .trim_start_matches("mission/")
                .split('/')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let (done, total) = mission.progress();
            (
                path.clone(),
                mission.id.clone(),
                vec![
                    span(format!("{glyph} "), theme::strong(color)),
                    span(path.last().cloned().unwrap_or_default(), theme::bold()),
                ],
                vec![span(
                    format!("{} {done}/{total}", human_mission_label(mission)),
                    theme::fg(color),
                )],
            )
        })
        .collect();
    let mut listing = tree_listing(
        leaves,
        state_of(&world.missions, "Loading missions…", "No missions yet."),
        vec![],
    );
    listing.legend = missions_list(world, 40, spinner, system).legend;
    listing
}


// ------------------------------------------------------------- new mission

pub const NEW_MISSION_FIELDS: [(&str, &str); 4] = [
    ("title", "A short name, like 'Nightly dependency audit'"),
    (
        "what you want",
        "Describe the outcome; the planner turns it into a mission",
    ),
    (
        "mission id",
        "Where it lives in the graph, like fleet/harbor/nightly-audit",
    ),
    ("workspace", "The directory the agents work in"),
];

/// The form behind Missions' New mission: it creates a launch, which a planner turns into a
/// proposed mission on Home. Nothing runs until the person approves it there.
/// `cursor` is where typing goes in the focused field.
pub fn new_mission_form(fields: &[String; 4], focus: usize, cursor: usize, width: usize) -> Doc {
    let mut inner = Doc::new();
    let w = width.saturating_sub(4);
    inner.blank();
    inner.wrap(
        &[run(
            "Say what you want. A planner turns it into a proposed mission, which appears in Inbox for you to approve; nothing runs before that.",
            theme::soft(),
        )],
        w,
    );
    inner.blank();
    for (index, (label, hint)) in NEW_MISSION_FIELDS.iter().enumerate() {
        let focused = index == focus;
        let mut body = Doc::new();
        let value = &fields[index];
        if value.is_empty() && !focused {
            body.line(Line::from(span(*hint, theme::dim())));
        } else {
            for runs in super::edit::lines(value, focused.then_some(cursor), theme::text()) {
                body.lines(text::wrap(&runs, w.saturating_sub(4), &[], &[], None));
            }
        }
        let start = inner.lines.len();
        inner.card(
            label,
            if focused {
                theme::ACCENT
            } else {
                theme::OVERLAY1
            },
            false,
            body,
            w,
        );
        for line in start..inner.lines.len() {
            inner.targets.push(super::doc::Target {
                line,
                column: 0,
                width: w as u16,
                hit: Hit::Field(index),
            });
        }
    }
    inner.blank();
    inner.buttons(&[
        ("tab", "Next field", Hit::Key('\t'), theme::OVERLAY1),
        ("enter", "Create the launch", Hit::Enter, theme::GREEN),
        ("esc", "Cancel", Hit::Escape, theme::OVERLAY1),
    ]);
    inner.blank();
    let mut doc = Doc::new();
    doc.card("new mission", theme::ACCENT, false, inner, width);
    doc
}

// ------------------------------------------------------------------ names

/// A name nobody has to think of: `amber-otter`. The person can change it.
pub fn random_name() -> String {
    const FIRST: [&str; 16] = [
        "amber", "brisk", "calm", "clever", "dusky", "eager", "gentle", "keen", "lucky", "merry",
        "nimble", "quiet", "rapid", "steady", "sunny", "witty",
    ];
    const SECOND: [&str; 16] = [
        "badger", "comet", "falcon", "fern", "harbor", "heron", "lantern", "maple", "otter",
        "pebble", "quartz", "raven", "sparrow", "tide", "willow", "wren",
    ];
    let bits = uuid::Uuid::now_v7().as_u128();
    // The low bits of a v7 id are random.
    format!(
        "{}-{}",
        FIRST[(bits & 15) as usize],
        SECOND[((bits >> 4) & 15) as usize]
    )
}

// ------------------------------------------------------------------ devices

pub fn devices_card(world: &World, width: usize) -> Doc {
    let mut inner = Doc::new();
    match &world.devices {
        Load::Loading => inner.line(Line::from(span("Loading your devices…", theme::dim()))),
        Load::Failed(error) => inner.wrap(
            &[run(
                format!("Could not read devices: {error}"),
                theme::fg(theme::RED),
            )],
            width.saturating_sub(4),
        ),
        Load::Ready(devices) if devices.is_empty() => {
            inner.line(Line::from(span("No paired devices.", theme::dim())))
        }
        Load::Ready(devices) => {
            for device in devices {
                inner.line(Line::from(vec![
                    span(
                        if device.state == "active" {
                            "● "
                        } else {
                            "○ "
                        },
                        theme::fg(if device.state == "active" {
                            theme::GREEN
                        } else {
                            theme::QUIET
                        }),
                    ),
                    span(format!("{:<22}", device.name), theme::text()),
                    span(format!("{:<24}", device.scopes.join(", ")), theme::soft()),
                    span(format!("expires {}", device.expires), theme::dim()),
                ]));
                if device.state == "active" {
                    inner.buttons(&[("", "Revoke", Hit::Revoke(device.id.clone()), theme::RED)]);
                }
            }
        }
    }
    let mut doc = Doc::new();
    doc.card("your devices", theme::OVERLAY1, false, inner, width);
    doc
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    #[test]
    fn attention_identity_labels_named_st_are_not_product_names() {
        let mut world = crate::ui::demo::test_world();
        let Load::Ready(items) = &mut world.attention else {
            panic!("the demo world has attention items");
        };
        let item = items.first_mut().unwrap();
        item.raised_by = Some("st".into());
        item.kind = AttentionKind::Message { from: "st".into(), body: "hello".into() };
        let id = item.id.clone();
        let drafts = Drafts {
            text: None, cursor: 0, editing: false, confirm: None,
            answering: None, note_answer: None, noted_answers: Vec::new(), answer_error: None,
            chat: None, form: Default::default(),
        };
        let shown = home_detail(&world, Some(&id), 120, &drafts)
            .lines.iter().map(text::plain).collect::<Vec<_>>().join("\n");
        assert!(shown.contains("raised by st"), "{shown}");
        assert!(shown.contains("from  st"), "{shown}");
    }

    #[test]
    fn launch_assignee_labels_named_st_are_not_product_names() {
        let mut world = crate::ui::demo::test_world();
        let Load::Ready(items) = &mut world.attention else {
            panic!("the demo world has attention items");
        };
        let item = items.first_mut().unwrap();
        item.kind = AttentionKind::Launch {
            planner: "planner".into(),
            name: "audit".into(),
            preview: Load::Ready(MissionPreview {
                name: "audit".into(), goals: vec![], agents: vec![], workspace: String::new(),
                steps: vec![PreviewStep {
                    name: "scan".into(), assignee: "st".into(), after: vec![], asks_you: false,
                }],
            }),
        };
        let id = item.id.clone();
        let drafts = Drafts {
            text: None, cursor: 0, editing: false, confirm: None,
            answering: None, note_answer: None, noted_answers: Vec::new(), answer_error: None,
            chat: None, form: Default::default(),
        };
        let shown = home_detail(&world, Some(&id), 120, &drafts)
            .lines.iter().map(text::plain).collect::<Vec<_>>().join("\n");
        assert!(shown.contains(&format!("  {:<12}{:<18}", "scan", "st")), "{shown}");
    }

    #[test]
    fn mission_copy_is_fractal_only_at_the_rendering_boundary() {
        let mut world = crate::ui::demo::test_world();
        let Load::Ready(missions) = &mut world.missions else {
            panic!("the demo world has missions");
        };
        let mission = missions.iter_mut().find(|mission| !mission.steps.is_empty()).unwrap();
        mission.word = Word::Watching;
        mission.steps[0].owner = Some("st".into());
        mission.steps[0].owner_is_system = true;
        let id = mission.id.clone();
        let serialized = serde_json::to_value(&*mission).unwrap();
        assert_eq!(serialized["steps"][0]["owner"], "st");
        let shown = mission_detail(&world, Some(&id), 120, "", None, &Default::default())
            .lines.iter().map(text::plain).collect::<Vec<_>>().join("\n");
        assert!(shown.contains("Fractal keeps this open"), "{shown}");
        assert!(shown.contains(" Fractal "), "{shown}");
        let Load::Ready(missions) = &world.missions else {
            unreachable!();
        };
        let mission = missions.iter().find(|mission| mission.id == id).unwrap();
        assert_eq!(serde_json::to_value(mission).unwrap(), serialized);
    }

    #[test]
    fn identity_labels_in_mission_steps_show_fallback_ids_only_once() {
        for (owner_id, label) in [
            ("person/ada", "person/ada"),
            ("agent/example/unnamed", "agent/example/unnamed"),
            ("agent/example/maintainer", "Maintainer"),
        ] {
            let mut world = crate::ui::demo::world();
            let Load::Ready(missions) = &mut world.missions else {
                panic!("the demo world has missions");
            };
            let mission = missions.iter_mut().find(|mission| !mission.steps.is_empty()).unwrap();
            mission.steps.truncate(1);
            let step = &mut mission.steps[0];
            step.owner = Some(label.into());
            step.owner_id = Some(owner_id.into());
            step.owner_is_system = false;
            let expected = if label == owner_id {
                owner_id.to_owned()
            } else {
                format!("{label} · {owner_id}")
            };
            assert_eq!(step.owner_identity().as_deref(), Some(expected.as_str()));
            let id = mission.id.clone();
            for width in [80, 120] {
                let shown = mission_detail(&world, Some(&id), width, "", None, &Default::default())
                    .lines.iter().map(text::plain).collect::<Vec<_>>().join("\n");
                assert!(shown.contains(&expected), "{shown}");
                assert_eq!(shown.matches(owner_id).count(), 1, "{shown}");
            }
        }
    }

    #[test]
    fn only_standing_seats_have_a_dim_anchor_in_both_agent_views() {
        use fractal_client::AgentLifecycle::{Bounded, Owner, Standing, Unknown};

        for lifecycle in [Some(Standing), Some(Owner), Some(Bounded), Some(Unknown), None] {
            let mut world = super::super::demo::test_world();
            let mut agent = world.agents.items()[0].clone();
            agent.lifecycle = lifecycle;
            agent.state = AgentState::Idle;
            world.agents = Load::Ready(vec![agent]);
            for listing in [agents_list(&world, "⠋", 60), agents_tree(&world, "⠋", 60)] {
                let first = listing
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Row { first, .. } => Some(first),
                        _ => None,
                    })
                    .unwrap();
                let shown = first
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                assert_eq!(
                    shown.contains('\u{F0031}'),
                    lifecycle == Some(Standing),
                    "{shown}"
                );
                if lifecycle == Some(Standing) {
                    assert!(shown.ends_with("Atlas Builder \u{F0031}"), "{shown}");
                    assert_eq!(first.last().unwrap().style, theme::dim());
                }
            }
        }
    }

    #[test]
    fn a_narrow_agent_row_truncates_its_name_before_its_standing_marker() {
        let mut world = super::super::demo::test_world();
        let mut agent = world.agents.items()[0].clone();
        agent.lifecycle = Some(fractal_client::AgentLifecycle::Standing);
        agent.name = "Atlas Builder 界界界".into();
        world.agents = Load::Ready(vec![agent]);
        for width in [20, 24, 28] {
            for listing in [agents_list(&world, "⠋", width), agents_tree(&world, "⠋", width)] {
                let (first, right) = listing
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Row { first, right, .. } => Some((first, right)),
                        _ => None,
                    })
                    .unwrap();
                let right_width = right.iter().map(Span::width).sum::<usize>();
                // Exercise the shell's final truncation, not only the screen's name budget.
                let fitted =
                    super::super::truncate_spans(first, width.saturating_sub(right_width + 2));
                let shown = fitted
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                assert!(shown.ends_with("… \u{F0031}"), "{width}: {shown}");
                assert!(
                    text::width(&shown) + right_width + 2 <= width,
                    "{width}: {shown}"
                );
                let padding = " ".repeat(width.saturating_sub(text::width(&shown) + right_width + 1));
                let row = format!(
                    "{shown}{padding}{}",
                    right
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                );
                assert_eq!(text::width(&row), width - 1, "{row}");
            }
        }
    }

    #[test]
    fn a_working_step_says_when_its_idle_holder_was_nudged() {
        let mut world = crate::ui::demo::test_world();
        let Load::Ready(missions) = &mut world.missions else {
            panic!("the demo world has missions");
        };
        let mission = missions
            .iter_mut()
            .find(|mission| mission.steps.iter().any(|step| step.state == StepState::Working))
            .unwrap();
        let step = mission
            .steps
            .iter()
            .find(|step| step.state == StepState::Working)
            .unwrap()
            .id
            .clone();
        let id = mission.id.clone();
        let shown = |world: &World| {
            mission_detail(world, Some(&id), 120, "", None, &Default::default())
                .lines
                .iter()
                .map(text::plain)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(!shown(&world).contains("nudged"));
        let Load::Ready(missions) = &mut world.missions else {
            unreachable!()
        };
        missions
            .iter_mut()
            .find(|mission| mission.id == id)
            .unwrap()
            .step_metadata
            .entry(step)
            .or_default()
            .nudged = Some("40m".into());
        let detail = shown(&world);
        assert!(
            detail.contains("nudged 40m ago: idle holding it, nothing set to wake it"),
            "{detail}"
        );
    }

    #[test]
    fn a_request_holding_a_json_report_shows_what_happened_and_folds_the_rest() {
        let text = r#"The deploy finished: {"status":"failed","error":"unit st3.service did not start","commit":"8821eced","host":"willow","attempt":2,"log":"/var/log/x","duration_ms":1234,"members":["maple","cedar"],"plan":{"a":1},"notes":"long\nnotes"}"#;
        let shown = report(text, 60)
            .unwrap()
            .iter()
            .map(text::plain)
            .collect::<Vec<_>>();
        assert_eq!(shown[0], "The deploy finished:");
        let joined = shown.join("\n");
        for wanted in [
            "status",
            "failed",
            "error",
            "unit st3.service did not start",
            "commit",
            "8821eced",
        ] {
            assert!(joined.contains(wanted), "{joined}");
        }
        assert!(shown.last().unwrap().contains("more fields"), "{joined}");
        assert!(!joined.contains('{'), "no braces: {joined}");
        assert!(report("Can you look at the deploy?", 60).is_none());
    }

    #[test]
    fn a_folder_holding_only_a_folder_joins_it_on_one_line() {
        let path = |text: &str| text.split('/').map(str::to_owned).collect::<Vec<_>>();
        let compact = compact_folders(&[
            path("fleet/fractal/operations/2026-10-01/operator"),
            path("fleet/fractal/ci/watcher"),
            path("fleet/cos/standing/cos"),
            path("solo"),
        ]);
        assert_eq!(
            compact,
            [
                vec![
                    "fleet".to_owned(),
                    "fractal".into(),
                    "operations/2026-10-01".into()
                ],
                vec!["fleet".to_owned(), "fractal".into(), "ci".into()],
                vec!["fleet".to_owned(), "cos/standing".into()],
                Vec::<String>::new(),
            ]
        );
    }

    #[test]
    fn an_agents_subagents_hang_beneath_its_row_and_are_never_selectable() {
        let world = super::super::demo::test_world();
        for listing in [agents_list(&world, "⠋", 60), agents_tree(&world, "⠋", 60)] {
            let builder = listing
                .ids
                .iter()
                .position(|id| id == "agent/example/atlas/builder")
                .unwrap();
            let children = listing
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Row {
                        index, children, ..
                    } if *index == builder => Some(children),
                    _ => None,
                })
                .unwrap();
            let lines = children
                .iter()
                .map(super::super::text::plain)
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert!(
                lines[0].contains("├ map the parser's error paths · Explore 4m"),
                "{lines:?}"
            );
            assert!(
                lines[1].contains("└ run the slow tests · general-purpose 1m"),
                "{lines:?}"
            );
            // Only agents are selectable; their subagents add no rows of their own.
            assert_eq!(listing.ids.len(), world.agents.items().len());
            assert!(listing.ids.iter().all(|id| !id.contains("a3f9e1")));
        }
    }

    #[test]
    fn a_tree_has_one_line_per_folder_and_leaf_in_path_order() {
        let listing = agents_tree(&super::super::demo::test_world(), "⠋", 40);
        let kinds = listing
            .items
            .iter()
            .map(|item| match item {
                Item::Folder(line) => format!("F {}", super::super::text::plain(line).trim()),
                Item::Row { first, .. } => format!(
                    "R {}",
                    first
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                        .trim()
                ),
                Item::Note(_) => "N".into(),
                Item::Header { .. } => "H".into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds[0], "F ▾ example/", "{kinds:#?}");
        assert!(
            kinds
                .iter()
                .all(|kind| !kind.starts_with('N') && !kind.starts_with('H')),
            "{kinds:#?}"
        );
    }
}

#[cfg(test)]
mod mission_golden {
    use super::*;
    use sha2::{Digest, Sha256};

    // Regenerate after intended rendering changes with:
    // FRACTAL_TUI_UPDATE_GOLDEN=1 cargo test -p fractal-tui mission_golden
    #[test]
    fn mission_stories_golden() {
        let world = super::super::demo::test_world();
        let mut actual = String::new();
        for mission in world.missions.items() {
            for width in [40, 80, 120] {
                for open in [false, true] {
                    let expanded = if open {
                        mission
                            .steps
                            .iter()
                            .map(|step| format!("step:{}:{}", mission.id, step.name))
                            .collect()
                    } else {
                        std::collections::HashSet::new()
                    };
                    let doc =
                        mission_detail(&world, Some(&mission.id), width, "⠋", None, &expanded);
                    // The digest also locks styles, exact span boundaries and click targets.
                    let digest = Sha256::digest(format!("{doc:#?}").as_bytes());
                    actual.push_str(&format!(
                        "{} width={width} open={open} sha256={digest:x}\n",
                        mission.id
                    ));
                    for line in &doc.lines {
                        actual.push_str(&line.to_string());
                        actual.push('\n');
                    }
                }
            }
        }
        if std::env::var_os("FRACTAL_TUI_UPDATE_GOLDEN").is_some() {
            std::fs::write(
                test_env!("CARGO_MANIFEST_DIR", "/tests/fixtures/mission-stories.golden"),
                &actual,
            )
            .unwrap();
            return;
        }
        assert_eq!(
            actual,
            include_str!("../../tests/fixtures/mission-stories.golden")
        );
    }
}

#[cfg(test)]
mod request_notes {
    use super::*;
    use fractal_client::{RequestAnswerOption, StructuredRequest};

    fn option(id: &str, label: &str, outcome: Option<&str>) -> RequestAnswerOption {
        RequestAnswerOption {
            id: id.into(),
            label: label.into(),
            consequence: format!("then {label}"),
            outcome: outcome.map(Into::into),
            conditions: Vec::new(),
        }
    }

    fn request(kind: &str, answers: Vec<RequestAnswerOption>, custom: bool) -> StructuredRequest {
        StructuredRequest {
            version: 1,
            entry_type: kind.into(),
            question: "How should this go?".into(),
            answers,
            custom,
            ..Default::default()
        }
    }

    fn decision() -> StructuredRequest {
        request(
            "decision",
            vec![
                option("ship", "Ship it", Some("accept")),
                option("hold", "Hold it", Some("decline")),
                option("revise", "Revise first", Some("request_changes")),
            ],
            true,
        )
    }

    fn choice() -> StructuredRequest {
        request(
            "choice",
            vec![option("a", "Option A", None), option("b", "Option B", None)],
            true,
        )
    }

    struct Case {
        name: &'static str,
        request: StructuredRequest,
        text: Option<&'static str>,
        editing: bool,
        answering: Option<usize>,
        note: Option<&'static str>,
        noted: &'static [&'static str],
        error: Option<&'static str>,
        shows: &'static [&'static str],
        hides: &'static [&'static str],
    }

    fn case(name: &'static str, request: StructuredRequest) -> Case {
        Case {
            name,
            request,
            text: None,
            editing: false,
            answering: None,
            note: None,
            noted: &[],
            error: None,
            shows: &[],
            hides: &[],
        }
    }

    fn cases() -> Vec<Case> {
        vec![
            Case {
                answering: Some(0),
                shows: &["Custom answer", "Send this answer", "c  Add note", "Another"],
                ..case("decision accept highlighted", decision())
            },
            Case {
                text: Some("after lunch"),
                editing: true,
                note: Some("ship"),
                noted: &["ship"],
                shows: &[
                    "NOTE FOR “SHIP IT” · ACCEPT",
                    "Enter sends “Ship it” · accept with this note.",
                    "Send “Ship it” · accept",
                    "✎ note",
                ],
                hides: &["unsent", "plus your notes"],
                ..case("decision accept note", decision())
            },
            Case {
                text: Some("not this week"),
                editing: true,
                note: Some("hold"),
                noted: &["hold", "ship"],
                shows: &[
                    "NOTE FOR “HOLD IT” · DECLINE",
                    "Send “Hold it” · decline",
                    "Enter sends “Hold it” · decline with this note, plus your notes on “Ship it”.",
                ],
                hides: &["unsent"],
                ..case("decision decline note beside another note", decision())
            },
            Case {
                editing: true,
                note: Some("revise"),
                shows: &[
                    "NOTE FOR “REVISE FIRST” · REQUEST CHANGES",
                    "Send “Revise first” · request changes",
                    "what should change?",
                ],
                ..case("decision changes note", decision())
            },
            Case {
                answering: Some(2),
                shows: &[
                    "NOTE FOR “REVISE FIRST” · REQUEST CHANGES",
                    "Start typing",
                    "Write your words",
                    "Add note",
                ],
                ..case("decision changes highlighted", decision())
            },
            Case {
                text: Some("the cheaper one"),
                editing: true,
                note: Some("a"),
                noted: &["a"],
                shows: &["NOTE FOR “OPTION A” · SELECTED", "Send “Option A” · selected"],
                ..case("choice note", choice())
            },
            Case {
                answering: Some(2),
                shows: &["Custom answer", "Write custom answer"],
                hides: &["Add note"],
                ..case("choice custom highlighted", choice())
            },
            Case {
                text: Some("neither"),
                editing: true,
                shows: &["CUSTOM ANSWER TO PLANNER", "Send custom answer", "✎ draft"],
                hides: &["NOTE FOR"],
                ..case("choice custom answer", choice())
            },
            Case {
                text: Some("neither"),
                editing: true,
                noted: &["a", "b"],
                shows: &["Enter sends this answer, plus your notes on “Option A” and “Option B”."],
                ..case("choice custom answer beside notes", choice())
            },
            Case {
                answering: Some(3),
                noted: &["ship", "hold", "revise"],
                shows: &[
                    "Enter writes a custom answer; your notes on “Ship it”, “Hold it” and “Revise first” go with it.",
                ],
                ..case("decision custom highlighted beside notes", decision())
            },
            Case {
                answering: Some(0),
                noted: &["hold"],
                shows: &["Enter sends “Ship it” · accept, plus your notes on “Hold it”."],
                hides: &["with its note"],
                ..case("decision unnoted answer beside a note", decision())
            },
            Case {
                answering: Some(2),
                noted: &["ship"],
                shows: &[
                    "what should change? Start typing; Enter sends it with “Revise first”",
                    "Your notes on “Ship it” go with “Revise first” once you write what should change.",
                ],
                hides: &["Enter sends “Revise first”"],
                ..case("decision changes highlighted beside a note", decision())
            },
            Case {
                editing: true,
                shows: &["ANSWER IN WORDS TO PLANNER", "Send the answer"],
                hides: &["Custom answer"],
                ..case("feedback answer in words", request("feedback", Vec::new(), false))
            },
            Case {
                shows: &["c  Answer in words", "Dismiss: nothing to do"],
                hides: &["Custom answer"],
                ..case("feedback resting", request("feedback", Vec::new(), false))
            },
            Case {
                text: Some("after lunch"),
                answering: Some(1),
                note: Some("ship"),
                noted: &["ship", "hold"],
                error: Some("the asker already closed this request"),
                shows: &[
                    "✕ Not sent  the asker already closed this request",
                    "NOTE FOR “SHIP IT” · ACCEPT",
                    "This note stays with “Ship it” · accept.",
                    "Enter sends “Hold it” · decline with its note, plus your notes on “Ship it”.",
                    "c  Add note",
                ],
                hides: &["Enter sends “Ship it”"],
                ..case("decision error after highlight moved", decision())
            },
            Case {
                error: Some("the asker already closed this request"),
                shows: &["✕ Not sent  the asker already closed this request", "Choose an answer"],
                hides: &["Add note", "c  Answer in words"],
                ..case("decision error resting", decision())
            },
        ]
    }

    fn render(case: &Case, width: usize) -> Doc {
        let drafts = Drafts {
            text: case.text,
            cursor: case.text.map_or(0, str::len),
            editing: case.editing,
            confirm: None,
            answering: case.answering,
            note_answer: case.note.map(Into::into),
            noted_answers: case.noted.iter().map(|id| (*id).to_owned()).collect(),
            answer_error: case.error,
            chat: None,
            form: Default::default(),
        };
        let mut card = Doc::new();
        structured_request(&mut card, "planner", &case.request, &[], &drafts, width);
        card
    }

    #[test]
    fn notes_name_their_answer_and_outcome() {
        for case in cases() {
            let shown = render(&case, 120)
                .lines
                .iter()
                .map(text::plain)
                .collect::<Vec<_>>()
                .join("\n");
            for wanted in case.shows {
                assert!(shown.contains(wanted), "{}: {wanted}\n{shown}", case.name);
            }
            for unwanted in case.hides {
                assert!(!shown.contains(unwanted), "{}: {unwanted}\n{shown}", case.name);
            }
        }
    }

    #[test]
    fn every_noted_answer_is_marked_whatever_is_highlighted() {
        let all = cases();
        let case = all
            .iter()
            .find(|case| case.name == "decision error after highlight moved")
            .unwrap();
        let doc = render(case, 120);
        let marked = doc
            .lines
            .iter()
            .map(text::plain)
            .filter(|line| line.contains("✎ note"))
            .collect::<Vec<_>>();
        assert_eq!(marked.len(), 2, "{marked:?}");
        assert!(marked[0].contains("Ship it") && marked[1].contains("Hold it"), "{marked:?}");
    }

    #[test]
    fn the_custom_entry_is_its_own_answer_row_only_beside_named_answers() {
        let targets = |request: StructuredRequest| {
            render(&case("targets", request), 120)
                .targets
                .into_iter()
                .filter_map(|target| match target.hit {
                    Hit::Answer(index) => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(targets(decision()), [0, 1, 2, 3]);
        let mut fixed = decision();
        fixed.custom = false;
        assert_eq!(targets(fixed), [0, 1, 2]);
        assert!(targets(request("choice", Vec::new(), true)).is_empty());
    }

    #[test]
    fn can_note_follows_the_highlighted_row() {
        let decision = decision();
        assert!(can_note(&decision, Some(0)));
        assert!(can_note(&decision, Some(2)));
        assert!(can_note(&decision, Some(3)), "the custom entry");
        assert!(!can_note(&decision, Some(4)));
        assert!(!can_note(&decision, None));
        let mut fixed = decision.clone();
        fixed.custom = false;
        assert!(!can_note(&fixed, Some(3)));
        assert!(can_note(&request("feedback", Vec::new(), false), None));
        assert!(can_note(&request("choice", Vec::new(), true), None));
        assert!(!can_note(&request("choice", Vec::new(), false), None));
    }

    /// The answer rows, the refusal and the key bar of a card whose note stayed with “Ship it”
    /// while the highlight moved to “Hold it”, exactly as a person reads them.
    #[test]
    fn a_moved_highlight_keeps_notes_and_the_refusal_in_view() {
        let all = cases();
        let case = all
            .iter()
            .find(|case| case.name == "decision error after highlight moved")
            .unwrap();
        let shown = render(case, 120)
            .lines
            .iter()
            .map(|line| text::plain(line).trim_end().to_owned())
            .filter(|line| {
                line.starts_with("○")
                    || line.starts_with("◉")
                    || line.starts_with("✕")
                    || line.starts_with(" enter")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                "○ Ship it  ✎ note  then Ship it",
                "◉ Hold it  ✎ note  then Hold it",
                "○ Revise first  needs your words  then Revise first",
                "○ Custom answer  write your own answer instead of these",
                "✕ Not sent  the asker already closed this request",
                " enter  Send this answer    c  Add note    ↑↓  Another    esc  Not now",
            ]
        );
    }
}
