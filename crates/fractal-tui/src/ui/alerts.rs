//! Alerts in the conversation of the agent they belong to: what waits on the person, above the
//! message box, folded to one line until the person expands it in place to the whole card.
//!
//! An alert is anything that blocks or waits on an answer: an ask, a human gate, a launch or
//! revision approval, a native harness prompt or a harness login. It clears itself when it is
//! answered anywhere, so the band shows exactly what st lists and nothing it remembers.
//!
//! A native prompt is a form that reads like the harness's own dialog: its question line, its
//! options numbered in its order with one marked, and the keys in a footer. The same form is the
//! prompt's card in Inbox and its card in the band, built by [`prompt_form`] from the shared model
//! in `fractal_ui_model::prompts` (web, tui and ios offer the same answers in the same words).
//! Focus only focuses: arrows and Tab move the mark and answer nothing. An answer takes its own
//! number or a click on its own row, and Enter takes the marked one only when Enter may (the
//! call or plan shown in full, the person's own choices, or a safe way past); never a refusal.

use super::doc::{AlertTap, Target};
use super::*;
use ratatui::style::{Color, Modifier};
use std::collections::{BTreeMap, BTreeSet};
use fractal_ui_model::prompts::{self, Call, Form, Kind, Question};

/// The most alerts the band shows expanded before it says how many more wait in Inbox.
const SHOWN: usize = 2;
/// The lines of a pending call the band shows. A call that is longer is cut with an ellipsis, and
/// then `allow` is withheld: a person allows only what they can read in full. Inbox shows it all.
const CALL_LINES: usize = 4;
/// The lines of a plan the band shows before it is cut and `approve` is withheld likewise.
const PLAN_LINES: usize = 24;

/// Where a prompt's form is drawn: its words for the keys differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Place {
    /// The band above a conversation's message box: Ctrl+N gives it the keys, Esc folds it.
    Band,
    /// The prompt's card in Inbox: `a` gives it the keys.
    Inbox,
    /// The band over an attached terminal: every key goes to the terminal, so only a click
    /// reaches it.
    Terminal,
}

/// The terminal an answer the form cannot give goes to, as said from `place`: the one under the
/// band, or the keys that reach it from the conversation (no Attach control is drawn there).
fn the_terminal(place: Place) -> String {
    match place {
        Place::Terminal => "this terminal".to_owned(),
        Place::Band | Place::Inbox => format!("the terminal ({})", screens::terminal_route()),
    }
}

/// One numbered row of a prompt's form, in the order it is drawn and numbered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FormRow {
    /// An option of a harness question: its question's and its own place in the pending call.
    Choose(usize, usize),
    /// An answer st sends.
    Answer(String),
}

/// What the person has done on a form so far: the mark, whether it has the keys, and a question's
/// choices.
#[derive(Clone, Copy, Default)]
pub(crate) struct FormState<'a> {
    pub mark: Option<usize>,
    pub keys: bool,
    pub chosen: Option<&'a BTreeMap<usize, BTreeSet<usize>>>,
}

/// A native prompt read as a form: the call it asks about, the shared model's form, and its
/// rows.
pub(super) struct PromptModel<'w> {
    calls: Vec<&'w PendingCall>,
    /// st names the call it asks about.
    named: bool,
    /// st lets the call it serves be allowed or answered from here.
    allowed: bool,
    /// Why st serves no calls beside it, when it cannot.
    unread: Option<&'w str>,
    form: Form,
    questions: Option<Vec<Question>>,
    pub rows: Vec<FormRow>,
}

impl PromptModel<'_> {
    /// The row marked when the person has not moved the mark: a question's first option, else
    /// the form's own mark.
    pub fn default_mark(&self) -> Option<usize> {
        match &self.questions {
            Some(_) => Some(0),
            None => self.form.mark,
        }
    }

    /// Why an answer that lets the call through is withheld when st does not allow it.
    fn withheld(&self) -> &'static str {
        if self.calls.iter().any(|call| !call.complete) {
            "too long for Fractal to show in full"
        } else {
            "Fractal does not let this through from here"
        }
    }

    /// Every question has a choice, so the answers can be sent.
    fn complete(&self, chosen: Option<&BTreeMap<usize, BTreeSet<usize>>>) -> bool {
        self.questions.as_ref().is_some_and(|asked| {
            (0..asked.len()).all(|index| {
                chosen
                    .and_then(|chosen| chosen.get(&index))
                    .is_some_and(|picked| !picked.is_empty())
            })
        })
    }
}

/// The call to show beside a prompt's answers, from those st serves beside it
/// (`presentation.prompt.calls`): the exact call it asks about, when st names it. A prompt that
/// names none has no call beside it, so it is never allowed, approved or answered from here: no
/// unanswered call is guessed at. `Err` says why st serves none.
fn prompt_calls(item: &Attention) -> Result<Vec<&PendingCall>, &str> {
    let AttentionKind::Prompt { calls, call, .. } = &item.kind else {
        return Ok(Vec::new());
    };
    let calls = calls.as_ref().map_err(String::as_str)?;
    Ok(calls
        .iter()
        .filter(|pending| call.as_deref() == Some(pending.id.as_str()))
        .collect())
}

/// A native prompt as a form, when st lists answers for it.
pub(super) fn prompt_model<'w>(world: &'w World, item: &'w Attention) -> Option<PromptModel<'w>> {
    let AttentionKind::Prompt {
        seat_id,
        answers,
        text,
        call,
        allow,
        ..
    } = &item.kind
    else {
        return None;
    };
    if answers.is_empty() {
        return None;
    }
    let harness = world
        .agents
        .items()
        .iter()
        .find(|agent| &agent.id == seat_id)
        .map_or(Harness::Unknown, |agent| agent.harness);
    let (calls, unread) = match prompt_calls(item) {
        Ok(calls) => (calls, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let shared: Vec<Call<'_>> = calls
        .iter()
        .map(|call| Call {
            tool: &call.tool,
            input: &call.input,
        })
        .collect();
    let form = prompts::prompt_form(
        match harness {
            Harness::Claude => prompts::Harness::Claude,
            Harness::Codex => prompts::Harness::Codex,
            _ => prompts::Harness::Other,
        },
        answers,
        text,
        &shared,
    );
    let questions = match (form.kind, shared.as_slice()) {
        (Kind::Question, [only]) => prompts::questions(*only),
        _ => None,
    };
    let mut rows = Vec::new();
    for (index, question) in questions.iter().flatten().enumerate() {
        rows.extend((0..question.options.len()).map(|option| FormRow::Choose(index, option)));
    }
    rows.extend(
        form.options
            .iter()
            .map(|option| FormRow::Answer(option.answer.clone())),
    );
    Some(PromptModel {
        calls,
        named: call.is_some(),
        allowed: *allow,
        unread,
        form,
        questions,
        rows,
    })
}

/// One option row: the mark, its number and its words, wrapped under themselves.
fn option_lines(marked: bool, keys: bool, number: usize, label: &str, style: Style, width: usize) -> Vec<Line<'static>> {
    let lead = if marked { "❯ " } else { "  " };
    let style = if marked {
        style.add_modifier(Modifier::BOLD).fg(theme::ACCENT)
    } else {
        style
    };
    let lead_style = if marked && keys {
        theme::strong(theme::ACCENT)
    } else if marked {
        theme::fg(theme::ACCENT)
    } else {
        theme::dim()
    };
    let number = format!("{number}. ");
    let indent = " ".repeat(text::width(lead) + text::width(&number));
    text::wrap(
        &[text::run(label.to_owned(), style)],
        width,
        &[text::run(lead, lead_style), text::run(number, if marked { lead_style } else { theme::dim() })],
        &[text::run(indent, theme::dim())],
        None,
    )
}

/// Whether `item`'s form asks `text` itself as its question, so a card shows it once.
pub(super) fn form_asks(world: &World, item: &Attention, text: &str) -> bool {
    prompt_model(world, item).is_some_and(|model| model.form.question.trim() == text.trim())
}

/// A prompt's form into `doc` at `width`: what it is about (the call or plan in full, or the
/// questions to choose from), the harness's question line, its numbered options with one marked,
/// and the keys. `limit` cuts a call (the band's few lines); a call that is cut withholds the
/// answers that would let it through. Returns whether the call or plan it is about is in view
/// in full, and how many of its lines are shown.
pub(super) fn prompt_form(
    doc: &mut Doc,
    world: &World,
    item: &Attention,
    state: FormState<'_>,
    width: usize,
    limit: Option<usize>,
    place: Place,
) -> (bool, usize) {
    let Some(model) = prompt_model(world, item) else {
        let words = match place {
            Place::Terminal => "Answer it in this terminal. This clears when the prompt is gone, however it was answered.".to_owned(),
            Place::Band | Place::Inbox => format!(
                "Answer it in the seat's terminal ({}). This clears when the prompt is gone, however it was answered.",
                screens::terminal_route()
            ),
        };
        doc.wrap(&text::inline(&words, theme::dim()), width);
        return (false, 0);
    };
    let id = &item.id;
    let note = |doc: &mut Doc, words: &str| doc.wrap(&[text::run(words.to_owned(), theme::dim())], width);
    // What it is about.
    let mut full = true;
    let mut call_shown = 0;
    match model.calls.as_slice() {
        [only] if model.questions.is_none() => {
            let limit = limit.map(|limit| {
                limit.min(if model.form.kind == Kind::Plan { PLAN_LINES } else { CALL_LINES })
            });
            // What would run sits behind a bar, apart from the options: a plan as markdown, as the
            // harness shows it; any other call field by field.
            let lines = match only.input.get("plan").and_then(serde_json::Value::as_str) {
                Some(plan) if model.form.kind == Kind::Plan => text::markdown(plan, width.saturating_sub(2), theme::text())
                    .into_iter()
                    .map(|mut line| {
                        line.spans.insert(0, Span::styled("▏ ", theme::fg(theme::SURFACE2)));
                        line
                    })
                    .collect(),
                _ => text::wrap(
                    &[text::run(prompts::call_lines(&only.input).join("\n"), theme::text())],
                    width.saturating_sub(2),
                    &[text::run("▏ ", theme::fg(theme::SURFACE2))],
                    &[text::run("▏ ", theme::fg(theme::SURFACE2))],
                    None,
                ),
            };
            doc.line(Line::from(Span::styled(only.tool.clone(), theme::bold())));
            let shown = limit.map_or(lines.len(), |limit| limit.min(lines.len()));
            full = shown == lines.len();
            call_shown = shown;
            for (index, mut line) in lines.into_iter().take(shown).enumerate() {
                if !full && index + 1 == shown {
                    line.spans.push(Span::styled(" …", theme::dim()));
                }
                doc.line(line);
            }
            doc.blank();
        }
        [] if matches!(model.form.kind, Kind::Permission | Kind::Plan | Kind::Question) => {
            note(
                doc,
                &match model.unread {
                    Some(why) => format!(
                        "{why}, so Fractal offers only what needs no reading. Or answer it in {}.",
                        the_terminal(place)
                    ),
                    None if model.named => format!(
                        "What it asks about is not in the conversation yet, so Fractal offers only what needs no reading. Or answer it in {}.",
                        the_terminal(place)
                    ),
                    None => format!(
                        "Fractal does not say which call this asks about, so it offers only what needs no reading. Or answer it in {}.",
                        the_terminal(place)
                    ),
                },
            );
            doc.blank();
        }
        _ => {}
    }
    let mark = state.mark.or(model.default_mark());
    let mut number = 0;
    // A question's options are chosen here, any number of them for a multi-select.
    for (index, question) in model.questions.iter().flatten().enumerate() {
        let header = if question.header.is_empty() {
            question.text.clone()
        } else {
            format!("{} · {}", question.header, question.text)
        };
        doc.lines(text::wrap(
            &[
                text::run(header, theme::bold()),
                text::run(if question.multi { "  (choose any)" } else { "" }, theme::dim()),
            ],
            width,
            &[],
            &[],
            None,
        ));
        let picked = state.chosen.and_then(|chosen| chosen.get(&index));
        for (option, choice) in question.options.iter().enumerate() {
            let on = picked.is_some_and(|picked| picked.contains(&option));
            let box_ = match (question.multi, on) {
                (true, true) => "[✓] ",
                (true, false) => "[ ] ",
                (false, true) => "✓ ",
                (false, false) => "",
            };
            let mut label = format!("{box_}{}", choice.label);
            if !choice.description.is_empty() {
                label.push_str(&format!(" · {}", choice.description));
            }
            let line = doc.lines.len();
            let style = if on { theme::fg(theme::GREEN) } else { theme::text() };
            doc.lines(option_lines(mark == Some(number), state.keys, number + 1, &label, style, width));
            doc.targets.push(Target {
                line,
                column: 0,
                width: width as u16,
                hit: Hit::Alert(id.clone(), AlertTap::Choose(index, option)),
            });
            number += 1;
        }
        doc.blank();
    }
    // The harness's question line, then its answers.
    doc.lines(text::wrap(&[text::run(model.form.question.clone(), theme::bold())], width, &[], &[], None));
    let complete = model.complete(state.chosen);
    for option in &model.form.options {
        let marked = mark == Some(number);
        // An answer is a row that answers only when it can: a call cut short is not let through,
        // and a question's answers go once every question has a choice.
        let withheld = match option.answer.as_str() {
            "allow" | "approve" if !full => Some("too long to show here in full"),
            // st serves the call cut short, or does not let it through from here.
            "allow" | "approve" | "answer" if !model.allowed => Some(model.withheld()),
            "answer" if !complete => Some("choose an answer to each question first"),
            _ => None,
        };
        let line = doc.lines.len();
        match withheld {
            Some(why) => doc.lines(option_lines(
                marked,
                state.keys,
                number + 1,
                &format!("{} · {why}", option.label),
                theme::dim(),
                width,
            )),
            None => {
                doc.lines(option_lines(marked, state.keys, number + 1, &option.label, theme::text(), width));
                doc.targets.push(Target {
                    line,
                    column: 0,
                    width: width as u16,
                    hit: Hit::Alert(id.clone(), AlertTap::Answer(option.answer.clone())),
                });
            }
        }
        number += 1;
    }
    if !full {
        let line = doc.lines.len();
        note(
            doc,
            &match place {
                Place::Band => format!("The call is too long to show here in full: click here to read it whole in Inbox, or answer it in {}.", the_terminal(place)),
                Place::Inbox => format!("The call is too long to show here in full: answer it in {}.", the_terminal(place)),
                Place::Terminal => "The call is too long to show here in full: click here to read it whole in Inbox, or answer it in this terminal.".to_owned(),
            },
        );
        // Outside Inbox the note is the way in to the whole call.
        if place != Place::Inbox {
            for line in line..doc.lines.len() {
                doc.targets.push(Target {
                    line,
                    column: 0,
                    width: width as u16,
                    hit: Hit::Alert(id.clone(), AlertTap::Open),
                });
            }
        }
    }
    if model.form.elsewhere.is_some() {
        let elsewhere = if place == Place::Terminal {
            "Other choices are available in this terminal.".to_owned()
        } else {
            format!("Other choices: {}.", screens::terminal_route())
        };
        note(doc, &elsewhere);
    }
    doc.blank();
    // The keys, as the harness's dialog lists its own.
    let marked = mark.and_then(|mark| model.rows.get(mark));
    let enter = match marked {
        Some(FormRow::Choose(question, _)) => Some(
            if model.questions.as_ref().is_some_and(|asked| asked[*question].multi) {
                "Enter to toggle"
            } else {
                "Enter to choose"
            },
        ),
        Some(FormRow::Answer(answer))
            if prompts::enter_answers(answer)
                && !(matches!(answer.as_str(), "allow" | "approve") && !full)
                && !(matches!(answer.as_str(), "allow" | "approve" | "answer") && !model.allowed)
                && !(answer == "answer" && !complete) =>
        {
            Some("Enter to select")
        }
        _ => None,
    };
    let count = model.rows.len();
    let numbers = if count > 1 {
        format!("1–{count} to choose")
    } else {
        "1 to choose".into()
    };
    let hints = if state.keys {
        let mut hints: Vec<String> = enter.map(str::to_owned).into_iter().collect();
        hints.push("↑/↓ to move".into());
        hints.push(numbers);
        hints.push(match place {
            Place::Band => "Esc to fold".into(),
            Place::Inbox | Place::Terminal => "Esc to stop".into(),
        });
        hints.join(" · ")
    } else {
        match place {
            Place::Band => "Ctrl+N to answer with the keys · or click an option".into(),
            Place::Inbox => "a to answer with the keys · or click an option".into(),
            Place::Terminal => "click an option".into(),
        }
    };
    doc.wrap(&[text::run(hints, theme::dim())], width);
    (full, call_shown)
}

/// A band card kept between frames: a hash of what it was drawn from, the card, and how many
/// lines of its call it shows.
pub(super) type KeptCard = (u64, Doc, usize);

/// One row of the band: its line, and the buttons on it by column.
pub(super) struct BandRow {
    pub(super) line: Line<'static>,
    pub(super) hits: Vec<(u16, u16, Hit)>,
    /// A card's right edge, drawn at its column: the blank between is not written cell by cell.
    pub(super) edge: Option<(u16, Span<'static>)>,
}

impl BandRow {
    /// The buttons in as few rows as fit `width`.
    fn button_rows(buttons: Vec<(String, Color, Hit)>, width: usize) -> Vec<Self> {
        let mut rows = Vec::new();
        let mut row: Vec<(String, Color, Hit)> = Vec::new();
        let mut used = 1;
        for button in buttons {
            let needed = text::width(&button.0) + 4;
            if !row.is_empty() && used + needed > width {
                rows.push(Self::buttons(std::mem::take(&mut row)));
                used = 1;
            }
            used += needed;
            row.push(button);
        }
        if !row.is_empty() {
            rows.push(Self::buttons(row));
        }
        rows
    }

    fn buttons(buttons: Vec<(String, Color, Hit)>) -> Self {
        let mut spans = vec![Span::raw(" ")];
        let mut hits = Vec::new();
        let mut column = 1u16;
        for (label, color, hit) in buttons {
            let shown = format!(" {label} ");
            let width = text::width(&shown) as u16;
            hits.push((column, width, hit));
            spans.push(Span::styled(
                shown,
                Style::default().fg(theme::CRUST).bg(color),
            ));
            spans.push(Span::raw("  "));
            column += width + 2;
        }
        Self {
            line: Line::from(spans),
            hits,
            edge: None,
        }
    }

    /// A document's lines as rows, each with the targets on it.
    fn from_doc(doc: Doc) -> Vec<Self> {
        let mut rows: Vec<Self> = doc
            .lines
            .into_iter()
            .map(|mut line| {
                // A card's inner line ends in plain padding and its border: keep the border as
                // an edge at its column and leave the padding unwritten, since the frame starts
                // blank.
                let mut edge = None;
                let count = line.spans.len();
                if count >= 3
                    && line.spans[count - 2].style == Style::default()
                    && line.spans[count - 2].content.chars().all(|c| c == ' ')
                {
                    let column = line.width() - line.spans[count - 1].width();
                    let border = line.spans.pop().unwrap_or_default();
                    line.spans.pop();
                    edge = Some((column as u16, border));
                }
                Self {
                    line,
                    hits: Vec::new(),
                    edge,
                }
            })
            .collect();
        for target in doc.targets {
            if let Some(row) = rows.get_mut(target.line) {
                row.hits.push((target.column, target.width, target.hit));
            }
        }
        rows
    }
}

/// Keep the inline reason to one row so it cannot displace the answer buttons.
/// The complete reason remains in the request's detail card.
fn recommendation_lines(label: &str, reason: &str, in_inbox: bool, width: usize) -> Vec<Line<'static>> {
    let prefix = text::truncate("★ recommended: ", width);
    let reason = reason.trim().replace(char::is_whitespace, " ");
    let reason = text::truncate(&reason, width.saturating_sub(text::width(&prefix)));
    let mut lines = vec![Line::from(vec![
        Span::styled(prefix, theme::strong(theme::GREEN)),
        Span::styled(reason, theme::soft()),
    ])];
    if in_inbox {
        lines.push(Line::from(Span::styled(
            text::truncate(&format!("★ {label} needs your words: open in Inbox"), width),
            theme::dim(),
        )));
    }
    lines
}

impl Ui {
    /// The open alerts that show in `agent`'s conversation, in the order Inbox lists them.
    pub(super) fn alerts_in(&self, agent: &str) -> Vec<&Attention> {
        self.world
            .attention
            .items()
            .iter()
            .filter(|item| {
                item.is_alert()
                    && item.is_in(agent)
                    && !self.snoozed.contains(&item.id)
                    && !self.closed.contains(&item.id)
            })
            .collect()
    }

    /// Alert `id`'s form state in a band at `place`: over a terminal the form never has the keys.
    fn band_form_state(&self, id: &str, place: Place) -> FormState<'_> {
        let mut state = self.form_state(id);
        state.keys &= place != Place::Terminal;
        state
    }

    /// What the person has done on alert `id`'s form.
    pub(super) fn form_state(&self, id: &str) -> FormState<'_> {
        FormState {
            mark: self.prompt_marks.get(id).copied(),
            keys: self.prompt_keys.as_deref() == Some(id),
            chosen: self
                .question_call_key(id)
                .and_then(|key| self.question_choices.get(&key)),
        }
    }

    /// Where alert `id`'s picks are kept: by the alert and its one question call. Read every
    /// frame, so it finds the call as the prompt's model does without building the form.
    fn question_call_key(&self, id: &str) -> Option<String> {
        let item = self.world.attention.items().iter().find(|item| item.id == id)?;
        let AttentionKind::Prompt { answers, .. } = &item.kind else {
            return None;
        };
        if answers.is_empty() {
            return None;
        }
        match prompt_calls(item).unwrap_or_default().as_slice() {
            [only] => Some(question_key(id, &only.id)),
            _ => None,
        }
    }

    /// Choose an option of a harness question alert's question, or for a multi-select toggle
    /// it. Nothing is sent until the person sends every question's choice.
    pub(super) fn choose_answer(&mut self, id: &str, question: usize, option: usize) {
        let Some(item) = self.world.attention.items().iter().find(|item| item.id == id) else {
            self.flash(GONE);
            return;
        };
        let Some((asked, key)) = prompt_model(&self.world, item).and_then(|model| {
            let key = match model.calls.as_slice() {
                [only] => question_key(id, &only.id),
                _ => return None,
            };
            Some((model.questions?, key))
        }) else {
            self.flash("The question is gone");
            return;
        };
        let Some(multi) = asked.get(question).map(|question| question.multi) else {
            return;
        };
        // Picks belong to this exact question call: a replaced question starts with none.
        let prefix = question_key(id, "");
        self.question_choices
            .retain(|other, _| *other == key || !other.starts_with(&prefix));
        let picked = self
            .question_choices
            .entry(key)
            .or_default()
            .entry(question)
            .or_default();
        if multi {
            if !picked.remove(&option) {
                picked.insert(option);
            }
        } else {
            picked.clear();
            picked.insert(option);
        }
    }

    /// The JSON a question alert sends: positions only, each question's position to the positions
    /// of the options chosen. The questions and labels stay in the conversation; the seat's hook
    /// turns positions into labels.
    fn question_answer(&self, id: &str) -> Option<String> {
        let item = self.world.attention.items().iter().find(|item| item.id == id)?;
        let asked = prompt_model(&self.world, item)?.questions?;
        let chosen = self.question_choices.get(&self.question_call_key(id)?)?;
        let mut answers = serde_json::Map::new();
        for (index, question) in asked.iter().enumerate() {
            let options = chosen
                .get(&index)?
                .iter()
                .filter(|option| **option < question.options.len())
                .copied()
                .collect::<Vec<_>>();
            if options.is_empty() {
                return None;
            }
            answers.insert(index.to_string(), serde_json::json!(options));
        }
        Some(serde_json::Value::Object(answers).to_string())
    }

    /// The band for `agent` at `width` columns and at most `room` rows: empty when nothing waits
    /// on the person there, one line that says what waits until the person expands it, then the
    /// cards in place. The line is a click or Ctrl+N away from them; no other key reaches it, so
    /// typing is never taken by an alert.
    pub(super) fn alert_band(
        &self,
        agent: &str,
        width: usize,
        room: usize,
        place: Place,
    ) -> Vec<BandRow> {
        let alerts = self.alerts_in(agent);
        let Some(first) = alerts.first() else {
            return Vec::new();
        };
        if self.alerts_open.contains(agent) {
            return self.alert_card(agent, width, room, place);
        }
        let count = alerts.len();
        let (glyph, color) = screens::attention_style(&first.kind);
        let lead = format!(" ◆ {count} alert{} · ", if count == 1 { "" } else { "s" });
        let tail = if place == Place::Terminal {
            " ▸ show "
        } else {
            " ▸ show · Ctrl+N "
        };
        let room = width
            .saturating_sub(text::width(&lead) + text::width(tail) + 4)
            .max(8);
        let spans = vec![
            Span::styled(lead, theme::strong(theme::PERSON)),
            Span::styled(format!("{glyph} "), theme::fg(color)),
            Span::styled(
                text::truncate(first_line(&first.title), room),
                theme::text(),
            ),
            Span::styled(tail, theme::dim()),
        ];
        vec![BandRow {
            line: Line::from(spans),
            hits: vec![(
                0,
                width as u16,
                Hit::Alert(agent.to_owned(), AlertTap::Toggle),
            )],
            edge: None,
        }]
    }

    /// Show or hide the cards of `agent`'s alerts. Folding them takes the keys back from them.
    pub(super) fn toggle_alerts(&mut self, agent: &str) {
        if !self.alerts_open.remove(agent) {
            self.alerts_open.insert(agent.to_owned());
        } else if self
            .prompt_keys
            .as_ref()
            .is_some_and(|id| self.alerts_in(agent).iter().any(|item| &item.id == id))
        {
            self.prompt_keys = None;
        }
    }

    /// One alert's card for the band, kept from the last frame while nothing it reads changed:
    /// the alert, its seat's pending calls and harness, the form's state, the width and the
    /// limit. Typing redraws every key, and the card should cost it nothing.
    fn band_card(&self, item: &Attention, width: usize, limit: usize, place: Place) -> (Doc, usize) {
        use std::hash::{Hash, Hasher};
        // Serialized, not formatted with Debug: this runs every frame, and serde is the cheaper.
        struct Feed<'a>(&'a mut std::collections::hash_map::DefaultHasher);
        impl std::io::Write for Feed<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.write(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (width, limit, place).hash(&mut hasher);
        let state = self.band_form_state(&item.id, place);
        let seat = match &item.kind {
            AttentionKind::Prompt { seat_id, .. } => Some(seat_id.as_str()),
            _ => None,
        };
        let (calls, allow) = match &item.kind {
            AttentionKind::Prompt { calls, allow, .. } => (Some(calls), *allow),
            _ => (None, false),
        };
        allow.hash(&mut hasher);
        if let Some(Err(why)) = calls {
            why.hash(&mut hasher);
        }
        let calls = calls.and_then(|calls| calls.as_ref().ok());
        let harness = seat.and_then(|seat| {
            self.world
                .agents
                .items()
                .iter()
                .find(|agent| agent.id == seat)
                .map(|agent| agent.harness)
        });
        let _ = serde_json::to_writer(Feed(&mut hasher), item);
        for call in calls.into_iter().flatten() {
            (&call.id, &call.tool).hash(&mut hasher);
            let _ = serde_json::to_writer(Feed(&mut hasher), &call.input);
        }
        (harness, state.mark, state.keys, state.chosen).hash(&mut hasher);
        let signature = hasher.finish();
        let key = (item.id.clone(), limit, place);
        if let Some((kept, doc, shown)) = self.band_cards.borrow().get(&key)
            && *kept == signature
        {
            return (doc.clone(), *shown);
        }
        let (doc, shown) = self.draw_band_card(item, width, limit, place);
        let mut cards = self.band_cards.borrow_mut();
        if cards.len() > 32 {
            cards.clear();
        }
        cards.insert(key, (signature, doc.clone(), shown));
        (doc, shown)
    }

    /// One alert's card for the band, as its card in Inbox draws it: its kind and age in the frame,
    /// its title, and for a prompt the same form, the call cut to `limit` lines. Also returns how
    /// many lines of the call it shows.
    fn draw_band_card(
        &self,
        item: &Attention,
        width: usize,
        limit: usize,
        place: Place,
    ) -> (Doc, usize) {
        let (_, color) = screens::attention_style(&item.kind);
        let heavy = color == theme::PERSON;
        let inner = width.saturating_sub(4).max(10);
        let mut card = Doc::new();
        card.lines(text::wrap(
            &[text::run(item.title.clone(), theme::bold())],
            inner,
            &[],
            &[],
            None,
        ));
        // Room between the title and what it says, as between what it says and its buttons: an
        // alert is read in pieces (Nathan, 2026-10-12: "still too compact when I expand it").
        if matches!(
            item.kind,
            AttentionKind::Prompt { .. } | AttentionKind::Request { .. } | AttentionKind::Login { .. }
        ) {
            card.blank();
        }
        let mut buttons: Vec<(String, Color, Hit)> = Vec::new();
        let mut call_shown = 0;
        match &item.kind {
            AttentionKind::Prompt { .. } => {
                (_, call_shown) = prompt_form(
                    &mut card,
                    &self.world,
                    item,
                    self.band_form_state(&item.id, place),
                    inner,
                    Some(limit),
                    place,
                );
            }
            AttentionKind::Request {
                structured: Some(request),
                ..
            } => {
                // A named answer is one tap, unless it needs the person's words: that one is
                // written in Inbox. The asker's recommended answer leads, marked with a star the
                // line above it explains, so it is never cut by the three that fit.
                let recommended = request.recommendation.as_ref().and_then(|wanted| {
                    request
                        .answers
                        .iter()
                        .find(|answer| answer.id == wanted.answer)
                        .map(|answer| (answer, wanted.reason.as_str()))
                });
                let tappable = |answer: &&fractal_client::RequestAnswerOption| {
                    answer.outcome.as_deref() != Some("request_changes")
                };
                if let Some((answer, reason)) = recommended {
                    let in_inbox = !tappable(&answer);
                    card.lines(recommendation_lines(&answer.label, reason, in_inbox, inner));
                }
                let lead = recommended.map(|(answer, _)| answer).filter(tappable);
                let rest = request
                    .answers
                    .iter()
                    .filter(tappable)
                    .filter(|answer| lead.is_none_or(|lead| lead.id != answer.id));
                for answer in lead.into_iter().chain(rest).take(3) {
                    let marked = lead.is_some_and(|lead| lead.id == answer.id);
                    let label = text::truncate(&answer.label, 28);
                    buttons.push((
                        if marked { format!("★ {label}") } else { label },
                        if marked { theme::GREEN } else { theme::ACCENT },
                        Hit::Alert(item.id.clone(), AlertTap::Answer(answer.id.clone())),
                    ));
                }
            }
            AttentionKind::Request { question, .. } => {
                card.line(Line::from(Span::styled(
                    text::truncate(first_line(question), inner),
                    theme::soft(),
                )));
                if item.title.trim_end().ends_with('?')
                    && !item.actions.iter().any(|action| action == "custom.reply")
                {
                    buttons.push((
                        "yes".into(),
                        theme::GREEN,
                        Hit::Alert(item.id.clone(), AlertTap::Answer("Yes".into())),
                    ));
                    buttons.push((
                        "no".into(),
                        theme::RED,
                        Hit::Alert(item.id.clone(), AlertTap::Answer("No".into())),
                    ));
                }
            }
            AttentionKind::Login { text: login, seats } => {
                card.wrap(
                    &[text::run(
                        format!(
                            "{} · {} seat{} share this sign-in. Sign in in the seat's terminal ({}).",
                            first_line(login),
                            seats.len().max(1),
                            if seats.len() > 1 { "s" } else { "" },
                            screens::terminal_route()
                        ),
                        theme::dim(),
                    )],
                    inner,
                );
            }
            _ => {}
        }
        if !buttons.is_empty() {
            card.blank();
        }
        for row in BandRow::button_rows(buttons, inner) {
            let line = card.lines.len();
            for (column, width, hit) in row.hits {
                card.targets.push(Target {
                    line,
                    column: column.saturating_sub(1),
                    width,
                    hit,
                });
            }
            let mut row_line = row.line;
            row_line.spans.remove(0);
            card.line(row_line);
        }
        let mut doc = Doc::new();
        doc.card(
            &card_title(item),
            if heavy { theme::PERSON } else { color },
            heavy,
            card,
            width,
        );
        (doc, call_shown)
    }

    /// The cards of `agent`'s alerts at `width` columns in at most `room` rows, under a line that
    /// folds them again. A prompt's call is cut to what fits, never its answers.
    fn alert_card(&self, agent: &str, width: usize, room: usize, place: Place) -> Vec<BandRow> {
        let alerts = self.alerts_in(agent);
        if alerts.is_empty() {
            return Vec::new();
        }
        let count = alerts.len();
        // The pane's title bar, filled across the width so the cards below read as one block
        // that belongs to the box under it; the whole bar folds it again.
        let left = format!(" ◆ {count} alert{} ", if count == 1 { "" } else { "s" });
        // Esc folds only while a form here has the keys; a click on the bar always does.
        let keys_here = place != Place::Terminal
            && self
                .prompt_keys
                .as_ref()
                .is_some_and(|keys| alerts.iter().any(|item| &item.id == keys));
        let right = if keys_here { " ▾ fold · Esc " } else { " ▾ fold " };
        let gap = width.saturating_sub(text::width(&left) + text::width(right));
        let bar = |style: Style| style.bg(theme::SURFACE0);
        let mut rows = vec![BandRow {
            line: Line::from(vec![
                Span::styled(left, bar(theme::strong(theme::PERSON))),
                Span::styled(" ".repeat(gap), bar(Style::default())),
                Span::styled(right, bar(theme::dim())),
            ]),
            hits: vec![(
                0,
                width as u16,
                Hit::Alert(agent.to_owned(), AlertTap::Toggle),
            )],
            edge: None,
        }];
        // The alert with the keys comes first, so its form is the one in view.
        let mut ordered = alerts.clone();
        if let Some(keys) = &self.prompt_keys
            && let Some(at) = ordered.iter().position(|item| &item.id == keys)
        {
            let item = ordered.remove(at);
            ordered.insert(0, item);
        }
        let mut shown = 0;
        for item in ordered.iter().take(SHOWN) {
            let left = room.saturating_sub(rows.len() + 1);
            let (mut card, call_shown) = self.band_card(item, width, PLAN_LINES, place);
            if card.lines.len() > left && call_shown > 0 {
                // Cut the call to what fits, keeping the form's answers and keys in view.
                let over = card.lines.len() - left;
                card = self
                    .band_card(item, width, call_shown.saturating_sub(over), place)
                    .0;
            }
            if shown > 0 && card.lines.len() > left {
                break;
            }
            // A form that still does not fit is never cut: its card says so and opens it in Inbox.
            if card.lines.len() > left {
                card = self.unfit_card(item, width, place);
                // Where not even that card's first rows fit, the bar's own label opens it.
                if left == 0 {
                    let label = format!(" ◆ {count} alert{} · answer in Inbox ", if count == 1 { "" } else { "s" });
                    let cut = text::width(&label).min(width);
                    let gap = width.saturating_sub(cut + text::width(right));
                    rows[0] = BandRow {
                        line: Line::from(vec![
                            Span::styled(label, bar(theme::strong(theme::PERSON))),
                            Span::styled(" ".repeat(gap), bar(Style::default())),
                            Span::styled(right, bar(theme::dim())),
                        ]),
                        hits: vec![
                            (0, cut as u16, Hit::Alert(item.id.clone(), AlertTap::Open)),
                            (cut as u16, width.saturating_sub(cut) as u16, Hit::Alert(agent.to_owned(), AlertTap::Toggle)),
                        ],
                        edge: None,
                    };
                }
            }
            rows.extend(BandRow::from_doc(card));
            shown += 1;
        }
        if count > shown {
            rows.push(BandRow {
                line: Line::from(Span::styled(
                    format!("   +{} more in Inbox", count - shown),
                    theme::dim(),
                )),
                hits: Vec::new(),
                edge: None,
            });
        }
        rows
    }

    /// The card of an alert whose form does not fit the band: its title, and a note that a click
    /// opens it whole in Inbox. Nothing on it answers, so no answer is taken from what is not shown.
    fn unfit_card(&self, item: &Attention, width: usize, place: Place) -> Doc {
        let (_, color) = screens::attention_style(&item.kind);
        let heavy = color == theme::PERSON;
        let inner = width.saturating_sub(4).max(10);
        let mut card = Doc::new();
        card.lines(text::wrap(&[text::run(item.title.clone(), theme::bold())], inner, &[], &[], None));
        let line = card.lines.len();
        card.wrap(
            &[text::run(
                match place {
                    Place::Terminal => "Its form does not fit here: click here to answer it in Inbox, or answer it in this terminal.".to_owned(),
                    Place::Band | Place::Inbox => format!("Its form does not fit here: click here to answer it in Inbox, or answer it in {}.", the_terminal(place)),
                },
                theme::dim(),
            )],
            inner,
        );
        for line in line..card.lines.len() {
            card.targets.push(Target {
                line,
                column: 0,
                width: inner as u16,
                hit: Hit::Alert(item.id.clone(), AlertTap::Open),
            });
        }
        let mut doc = Doc::new();
        doc.card(
            &card_title(item),
            if heavy { theme::PERSON } else { color },
            heavy,
            card,
            width,
        );
        // Every row of it opens it, its border too, so whatever rows fit carry the way in.
        doc.targets.clear();
        for line in 0..doc.lines.len() {
            doc.targets.push(Target {
                line,
                column: 0,
                width: width as u16,
                hit: Hit::Alert(item.id.clone(), AlertTap::Open),
            });
        }
        doc
    }

    /// Open an alert's whole card in Inbox, selected.
    pub(super) fn open_alert(&mut self, id: &str) {
        self.open_home();
        if let Some(index) = self
            .listing_for(0, 40)
            .ids
            .iter()
            .position(|item| item == id)
        {
            self.select(index);
        }
    }

    /// Inbox has the keys: its tab is focused and nothing floats over it.
    pub(super) fn inbox_in_view(&self) -> bool {
        !self.home_open() && self.focused_pane() == Some(Pane::List(0))
    }

    /// The agent whose conversation the person is in, with its band above the message box.
    pub(super) fn conversation_here(&self) -> Option<String> {
        if self.inbox_in_view() {
            return None;
        }
        self.composing_agent()
    }

    /// The prompt alert whose form takes the keys from here: in a conversation, the first of its
    /// prompts with answers; in Inbox, the one selected.
    fn form_here(&self) -> Option<String> {
        let answerable = |item: &&Attention| prompt_model(&self.world, item).is_some();
        if self.inbox_in_view() {
            let id = self.selected_id()?;
            self.world
                .attention
                .items()
                .iter()
                .filter(answerable)
                .find(|item| item.id == id)
                .map(|item| item.id.clone())
        } else {
            let agent = self.conversation_here()?;
            self.alerts_in(&agent)
                .into_iter()
                .find(answerable)
                .map(|item| item.id.clone())
        }
    }

    /// Ctrl+N, or `a` on a prompt in Inbox: give its form the keys (expanding the band in place
    /// in a conversation), or take them back and fold it.
    pub(super) fn toggle_form_keys(&mut self) {
        if self.prompt_keys.take().is_some() {
            if !self.inbox_in_view()
                && let Some(agent) = self.conversation_here()
            {
                self.alerts_open.remove(&agent);
            }
            return;
        }
        if !self.inbox_in_view()
            && let Some(agent) = self.conversation_here()
        {
            if self.alerts_in(&agent).is_empty() {
                self.flash("Nothing here waits on you");
                return;
            }
            self.alerts_open.insert(agent);
        }
        match self.form_here() {
            Some(id) => {
                self.editing = false;
                self.prompt_keys = Some(id);
            }
            None if self.inbox_in_view() => self.flash("This alert has no answers to choose from here"),
            None => {}
        }
    }

    /// A key while a prompt's form has the keys. Arrows and Tab move the mark; a number answers
    /// (or chooses) its row; Enter takes the marked row only when Enter may; Esc folds. Any other
    /// printable key goes back to typing, the key included. Returns whether the form took it.
    pub(super) fn form_key(&mut self, key: KeyEvent) -> bool {
        let Some(id) = self.prompt_keys.clone() else {
            return false;
        };
        let Some(item) = self.world.attention.items().iter().find(|item| item.id == id) else {
            self.prompt_keys = None;
            return false;
        };
        let Some(model) = prompt_model(&self.world, item) else {
            // Its answers went while it had the keys: Esc still folds it, and nothing else is taken.
            if key.code == KeyCode::Esc {
                self.toggle_form_keys();
                return true;
            }
            self.prompt_keys = None;
            return false;
        };
        let rows = model.rows.clone();
        let mark = self
            .prompt_marks
            .get(&id)
            .copied()
            .or(model.default_mark())
            .filter(|mark| *mark < rows.len());
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let count = rows.len().max(1);
        // A digit typed first on a form of ten or more rows waits for a second, or for Enter.
        let digit = self.prompt_digit.take().filter(|(form, _)| *form == id).map(|(_, first)| first);
        if let Some(first) = digit {
            match key.code {
                KeyCode::Char(second @ '0'..='9') if plain => {
                    self.take_row(&id, &rows, first * 10 + (second as usize - '0' as usize));
                    return true;
                }
                KeyCode::Enter if plain => {
                    self.take_row(&id, &rows, first);
                    return true;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Down | KeyCode::Tab | KeyCode::Right if plain => {
                self.prompt_marks
                    .insert(id, mark.map_or(0, |mark| (mark + 1) % count));
            }
            KeyCode::Up | KeyCode::BackTab | KeyCode::Left if plain => {
                self.prompt_marks
                    .insert(id, mark.map_or(0, |mark| (mark + count - 1) % count));
            }
            KeyCode::Esc => self.toggle_form_keys(),
            KeyCode::Enter if plain => match mark.and_then(|mark| rows.get(mark)) {
                Some(FormRow::Choose(question, option)) => self.choose_answer(&id, *question, *option),
                Some(FormRow::Answer(answer)) if prompts::enter_answers(answer) => {
                    self.answer_shown(&id, answer)
                }
                Some(FormRow::Answer(_)) | None => self.flash(format!(
                    "Enter takes only an answer about what is shown in full: press its number{}",
                    if rows.len() > 1 { format!(" (1–{})", rows.len()) } else { String::new() }
                )),
            },
            KeyCode::Char(' ') if plain => {
                if let Some(FormRow::Choose(question, option)) = mark.and_then(|mark| rows.get(mark)) {
                    self.choose_answer(&id, *question, *option);
                }
            }
            KeyCode::Char(digit @ '1'..='9') if plain => {
                let number = digit as usize - '0' as usize;
                if number * 10 <= rows.len() {
                    // Another number starts with it: wait for its second digit, or Enter.
                    self.prompt_digit = Some((id.clone(), number));
                    self.flash(format!(
                        "{number}… another digit for {}–{}, or Enter for {number}",
                        number * 10,
                        rows.len().min(number * 10 + 9)
                    ));
                } else {
                    self.take_row(&id, &rows, number);
                }
            }
            // Typing is never blocked: the key goes to the message box (or Inbox's own keys).
            KeyCode::Char(_) if plain => {
                self.prompt_keys = None;
                if !self.inbox_in_view() && self.conversation_here().is_some() {
                    self.editing = true;
                }
                return false;
            }
            _ if key.code == KeyCode::Char('n') && key.modifiers == KeyModifiers::CONTROL => {
                self.toggle_form_keys()
            }
            _ => {
                self.prompt_keys = None;
                return false;
            }
        }
        true
    }

    /// Take row `number` (from 1) of alert `id`'s form from the keys: choose a question's option,
    /// or answer, as a click on that row would.
    fn take_row(&mut self, id: &str, rows: &[FormRow], number: usize) {
        let index = number.wrapping_sub(1);
        match rows.get(index) {
            Some(FormRow::Choose(question, option)) => {
                self.prompt_marks.insert(id.to_owned(), index);
                self.choose_answer(id, *question, *option);
            }
            Some(FormRow::Answer(answer)) => {
                self.prompt_marks.insert(id.to_owned(), index);
                let answer = answer.clone();
                self.answer_shown(id, &answer);
            }
            None => self.flash(format!("There is no {number} here")),
        }
    }

    /// Answer `answer` on alert `id` from the keys, only when its row is on screen and answers:
    /// the same row a click would take. A call cut short or a question not yet chosen has none.
    fn answer_shown(&mut self, id: &str, answer: &str) {
        let wanted = Hit::Alert(id.to_owned(), AlertTap::Answer(answer.to_owned()));
        let shown = self.frame.borrow().hits.iter().any(|(_, hit)| *hit == wanted);
        if shown {
            self.answer_alert(id, answer);
        } else if answer == "answer" {
            self.flash("Choose an answer to each question first");
        } else {
            self.flash(format!("That answer is not in view in full here: open it in Inbox, or answer it in the terminal ({})", screens::terminal_route()));
        }
    }

    /// Send a typed answer to an alert: a prompt's answer, a structured request's named answer,
    /// or the yes or no of a question.
    pub(super) fn answer_alert(&mut self, id: &str, answer: &str) {
        let Some(item) = self
            .world
            .attention
            .items()
            .iter()
            .find(|item| item.id == id)
        else {
            self.flash(GONE);
            return;
        };
        let effect = match &item.kind {
            // St names no call: only deny goes from here, whatever asked for more.
            AttentionKind::Prompt { call: None, .. }
                if matches!(answer, "allow" | "approve" | "answer") =>
            {
                self.flash(format!("Fractal does not say which call this asks about: answer it in the terminal ({})", screens::terminal_route()));
                return;
            }
            // st allows only the one call it serves beside the prompt, whole. The card may have
            // changed since its buttons were drawn, so a tap is checked against it again.
            AttentionKind::Prompt { allow, .. }
                if matches!(answer, "allow" | "approve" | "answer")
                    && !(*allow && prompt_calls(item).is_ok_and(|calls| calls.len() == 1)) =>
            {
                self.flash(format!("Fractal does not let this call through from here: answer it in the terminal ({})", screens::terminal_route()));
                return;
            }
            AttentionKind::Prompt { answers, .. }
                if item.actions.iter().any(|a| a == "prompt.respond")
                    && answers.iter().any(|offered| offered == answer) =>
            {
                // A question's answer is the choices made here.
                let sent = if answer == "answer" {
                    self.question_answer(id)
                } else {
                    Some(answer.to_owned())
                };
                sent.map(|sent| {
                    (
                        Effect::Attention {
                            id: id.to_owned(),
                            action: "prompt.respond".into(),
                            reason: None,
                            answer: Some(sent),
                        },
                        format!("Answering “{}”…", self.answer_words(item, answer)),
                    )
                })
            }
            AttentionKind::Request {
                structured: Some(request),
                ..
            } => request
                .answers
                .iter()
                .find(|option| option.id == answer)
                .map(|option| {
                    (
                        Effect::Attention {
                            id: id.to_owned(),
                            action: "work.done".into(),
                            reason: Some(option.label.clone()),
                            answer: Some(option.id.clone()),
                        },
                        format!("Answering “{}”…", option.label),
                    )
                }),
            AttentionKind::Request {
                structured: None, ..
            } if matches!(answer, "Yes" | "No")
                && item.actions.iter().any(|a| a == "work.done") =>
            {
                Some((
                    Effect::Attention {
                        id: id.to_owned(),
                        action: "work.done".into(),
                        reason: Some(answer.to_owned()),
                        answer: None,
                    },
                    format!("Answering “{answer}”…"),
                ))
            }
            _ => None,
        };
        match effect {
            Some((effect, notice)) if self.live => {
                self.effects.push(effect);
                let prefix = question_key(id, "");
                self.question_choices.retain(|key, _| !key.starts_with(&prefix));
                self.prompt_marks.remove(id);
                if self.prompt_keys.as_deref() == Some(id) {
                    self.prompt_keys = None;
                }
                self.flash(notice);
            }
            Some((_, notice)) => self.flash(format!("{notice} demo: nothing was sent")),
            None => self.flash(format!(
                "Fractal does not offer that answer here: answer it in the terminal ({})",
                screens::terminal_route()
            )),
        }
    }

    /// The words a prompt's answer reads as on its form.
    fn answer_words(&self, item: &Attention, answer: &str) -> String {
        prompt_model(&self.world, item)
            .and_then(|model| {
                model
                    .form
                    .options
                    .into_iter()
                    .find(|option| option.answer == answer)
                    .map(|option| option.label)
            })
            .unwrap_or_else(|| answer.to_owned())
    }
}

/// What a client says when the alert it answers is no longer listed, in st's own words for a
/// prompt that is gone.
const GONE: &str = "That prompt is no longer waiting: it was answered or replaced";

/// The first non-empty line, without markdown's heading marks.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("")
}


/// Where the picks for a question alert's question call are kept: by the alert and the exact call,
/// so picks made for one question never answer another that replaced it.
fn question_key(alert: &str, call: &str) -> String {
    format!("{alert}\u{0}{call}")
}

/// An alert card's title: what it is and how long it has waited, then, for an ask, how long is
/// left and what then applies.
fn card_title(item: &Attention) -> String {
    let title = format!("{} · waiting {}", item.kind.word(), item.age);
    match &item.deadline {
        Some(deadline) => format!("{title} · {deadline}"),
        None => title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::glass::Open;
    use ratatui::{Terminal, backend::TestBackend};

    const SEAT: &str = "agent/example/atlas/builder";

    fn prompt(id: &str, answers: &[&str], text: &str) -> Attention {
        Attention {
            deadline: None,
            requested_at: None,
            id: id.into(),
            tier: Tier::Stopped,
            title: "builder is waiting for permission".into(),
            waiting: None,
            age: "1m".into(),
            mission: None,
            agent: Some(SEAT.into()),
            kind: AttentionKind::Prompt {
                seat: "builder".into(),
                seat_id: SEAT.into(),
                text: text.into(),
                question: answers.contains(&"answer"),
                answers: answers.iter().map(|answer| (*answer).to_owned()).collect(),
                episode: "episode-1".into(),
                call: None,
                calls: Ok(vec![]),
                allow: false,
            },
            actions: if answers.is_empty() {
                vec![]
            } else {
                vec!["prompt.respond".into()]
            },
            related: vec![],
            raised_by: None,
            blocked: None,
            conversations: vec![SEAT.into()],
        }
    }

    /// `item` as st sends it once it names the exact call the prompt asks about.
    fn naming(mut item: Attention, call: &str) -> Attention {
        if let AttentionKind::Prompt { call: named, .. } = &mut item.kind {
            *named = Some(call.to_owned());
        }
        item
    }

    fn call(tool: &str, input: serde_json::Value) -> PendingCall {
        PendingCall {
            id: format!("call-{tool}"),
            tool: tool.into(),
            input,
            complete: true,
        }
    }

    /// st serves `calls` beside each prompt, and allows the call the prompt names when it is
    /// among them and whole. A prompt that names none is allowed nothing.
    fn serve(world: &mut World, calls: Vec<PendingCall>) {
        let Load::Ready(items) = &mut world.attention else {
            panic!("no attention");
        };
        for item in items {
            if let AttentionKind::Prompt { calls: served, call, allow, answers, .. } = &mut item.kind {
                *allow = !answers.is_empty()
                    && call
                        .as_deref()
                        .is_some_and(|named| calls.iter().any(|pending| pending.id == named && pending.complete));
                *served = Ok(calls.clone());
            }
        }
    }

    /// The calls st serves beside the first prompt.
    fn served(world: &World) -> Vec<PendingCall> {
        world
            .attention
            .items()
            .iter()
            .find_map(|item| match &item.kind {
                AttentionKind::Prompt { calls: Ok(calls), .. } => Some(calls.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// One prompt kind with invented content: the answers st lists, the call it is about, and
    /// what its form must show.
    struct Case {
        name: &'static str,
        harness: Harness,
        answers: &'static [&'static str],
        text: &'static str,
        calls: Vec<PendingCall>,
        /// The options in order, as numbered rows.
        options: &'static [&'static str],
        /// The option marked at first.
        marked: Option<&'static str>,
        question: &'static str,
    }

    fn cases() -> Vec<Case> {
        vec![
            Case {
                name: "claude read outside the working directories",
                harness: Harness::Claude,
                answers: &["allow", "deny"],
                text: "Claude asks to use Read",
                calls: vec![call("Read", serde_json::json!({"file_path": "/srv/example/shared/notes.txt"}))],
                options: &["Yes, but ask again next time", "No, and ask again next time"],
                marked: Some("Yes, but ask again next time"),
                question: "Allow this read outside the working directories?",
            },
            Case {
                name: "claude command of several lines",
                harness: Harness::Claude,
                answers: &["allow", "deny"],
                text: "Claude asks to use Bash",
                calls: vec![call(
                    "Bash",
                    serde_json::json!({"command": "cd garden\nmake clean\nmake all", "description": "Rebuild"}),
                )],
                options: &["Yes", "No"],
                marked: Some("Yes"),
                question: "Do you want to proceed?",
            },
            Case {
                name: "claude plan",
                harness: Harness::Claude,
                answers: &["approve", "deny"],
                text: "Claude asks you to approve its plan",
                calls: vec![call(
                    "ExitPlanMode",
                    serde_json::json!({"plan": "1. Add a greeting to the garden page\n2. Test it"}),
                )],
                options: &["Yes", "No, keep planning"],
                marked: Some("Yes"),
                question: "Would you like to proceed?",
            },
            Case {
                name: "claude question with a multi-select",
                harness: Harness::Claude,
                answers: &["answer", "deny"],
                text: "Claude asks a question",
                calls: vec![call(
                    "AskUserQuestion",
                    serde_json::json!({"questions": [
                        {"question": "Which flowers?", "header": "Flowers", "multiSelect": true,
                         "options": [{"label": "Tulips", "description": "spring"},
                                     {"label": "Asters", "description": "autumn"}]}
                    ]}),
                )],
                options: &[
                    "[ ] Tulips · spring",
                    "[ ] Asters · autumn",
                    "Submit answers · choose an answer to each question first",
                    "No, decline to answer",
                ],
                marked: Some("[ ] Tulips · spring"),
                question: "Ready to submit your answers?",
            },
            Case {
                name: "claude new MCP server",
                harness: Harness::Claude,
                answers: &["use", "use-all", "skip"],
                text: "New MCP server found in this project: example-echo.",
                calls: vec![],
                options: &[
                    "Use this MCP server",
                    "Use this and all future MCP servers in this project",
                    "Continue without using this MCP server",
                ],
                marked: Some("Continue without using this MCP server"),
                question: "New MCP server found in this project: example-echo.",
            },
            Case {
                name: "codex approval",
                harness: Harness::Codex,
                answers: &["allow", "deny"],
                text: "Codex asks for approval",
                calls: vec![call("shell", serde_json::json!({"command": "npm test -- --watch=false"}))],
                options: &["Yes, proceed", "No"],
                marked: Some("Yes, proceed"),
                question: "Would you like to run the following command?",
            },
        ]
    }

    fn world_with(case: &Case) -> World {
        let mut world = demo::test_world();
        if let Load::Ready(agents) = &mut world.agents
            && let Some(agent) = agents.iter_mut().find(|agent| agent.id == SEAT)
        {
            agent.harness = case.harness;
        }
        // st names the call each prompt asks about.
        let item = prompt("attention/one", case.answers, case.text);
        let item = match case.calls.as_slice() {
            [only] => naming(item, &only.id),
            _ => item,
        };
        world.attention = Load::Ready(vec![item]);
        serve(&mut world, case.calls.clone());
        world
    }

    /// The conversation, as a person in a glass sees it.
    fn in_conversation(world: World) -> Ui {
        let mut ui = Ui::new(world);
        ui.glasses.sidebar.shown = false;
        ui.open_in_glass(Pane::Agent(Some(SEAT.into())), Open::Tab);
        ui
    }

    /// Inbox, with the alert's card selected.
    fn in_inbox(world: World) -> Ui {
        let mut ui = Ui::new(world);
        ui.glasses.sidebar.shown = false;
        ui.open_home();
        let index = ui.ids().iter().position(|id| id == "attention/one").unwrap();
        ui.select(index);
        ui
    }

    fn render(ui: &Ui, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(ui: &mut Ui, code: KeyCode, modifiers: KeyModifiers) {
        ui.key(KeyEvent::new(code, modifiers));
    }

    /// The numbered option rows on screen, in order, with whether each is marked.
    fn options(shown: &str) -> Vec<(bool, usize, String)> {
        shown
            .lines()
            .filter_map(|line| {
                // Inside a card's frame, an option row starts with its mark or two spaces.
                let line = match line.split_once('┃') {
                    Some((_, inside)) => inside,
                    None => line.split_once('│').map_or(line, |(_, inside)| inside),
                };
                let line = line.strip_prefix(' ').unwrap_or(line);
                let marked = line.starts_with("❯ ");
                let rest = line.strip_prefix("❯ ").or_else(|| line.strip_prefix("  "))?;
                if !rest.starts_with(|c: char| c.is_ascii_digit()) {
                    return None;
                }
                let (number, label) = rest.split_once(". ")?;
                let number = number.parse::<usize>().ok()?;
                let label = label.trim_end_matches(['│', '┃', ' ']).trim_end();
                Some((marked, number, label.to_owned()))
            })
            .collect()
    }

    /// The screen's words in one line, without card frames, so wrapped words read on.
    fn words_of(shown: &str) -> String {
        shown
            .replace(['┃', '│'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn check_form(case: &Case, shown: &str, width: u16, place: &str) {
        let rows = options(shown);
        let labels: Vec<&str> = rows.iter().map(|(_, _, label)| label.as_str()).collect();
        // A label too long for a narrow card wraps: its first line starts the same.
        assert_eq!(rows.len(), case.options.len(), "{} {place} at {width}: {labels:?}\n{shown}", case.name);
        for ((_, number, label), wanted) in rows.iter().zip(case.options) {
            assert!(wanted.starts_with(label.as_str()), "{} {place} at {width}: {label} for {wanted}\n{shown}", case.name);
            assert!(*number >= 1);
        }
        let numbers: Vec<usize> = rows.iter().map(|(_, number, _)| *number).collect();
        assert_eq!(numbers, (1..=rows.len()).collect::<Vec<_>>(), "{}", case.name);
        let marked: Vec<&str> = rows.iter().filter(|(marked, _, _)| *marked).map(|(_, _, label)| label.as_str()).collect();
        match case.marked {
            Some(wanted) => assert!(marked.len() == 1 && wanted.starts_with(marked[0]), "{} {place} at {width}: {marked:?}\n{shown}", case.name),
            None => assert!(marked.is_empty(), "{}", case.name),
        }
        let flat = words_of(shown);
        let question = case.question.split_whitespace().collect::<Vec<_>>().join(" ");
        let squeezed: String = flat.chars().filter(|c| c.is_alphanumeric()).collect();
        let wanted: String = question.chars().filter(|c| c.is_alphanumeric()).collect();
        assert!(squeezed.contains(&wanted), "{} {place} at {width}: question\n{shown}", case.name);
        assert!(!shown.contains("open in Inbox"), "{}\n{shown}", case.name);
    }

    /// Every prompt kind in Inbox and in its conversation, folded and expanded, at three widths:
    /// the same form, worded and numbered like the harness's, with its marked option and keys.
    #[test]
    fn every_prompt_kind_reads_like_its_harness_in_inbox_and_in_place() {
        for case in cases() {
            for width in [60u16, 80, 120] {
                // Inbox: the card carries the prompt's answers.
                let ui = in_inbox(world_with(&case));
                let shown = render(&ui, width, 60);
                check_form(&case, &shown, width, "Inbox");
                let flat = words_of(&shown);
                assert!(flat.contains("a to answer with the keys"), "{}\n{shown}", case.name);
                // What the prompt says is said once, however the form words its question.
                let squeezed: String = flat.chars().filter(|c| c.is_alphanumeric()).collect();
                let said: String = case.text.chars().filter(|c| c.is_alphanumeric()).take(24).collect();
                assert_eq!(squeezed.matches(&said).count(), 1, "{}\n{shown}", case.name);

                // The conversation, folded: one line, nothing to answer, a way in.
                let mut ui = in_conversation(world_with(&case));
                let shown = render(&ui, width, 60);
                assert!(shown.contains("◆ 1 alert") && shown.contains("Ctrl+N"), "{}\n{shown}", case.name);
                assert!(options(&shown).is_empty(), "{} folded\n{shown}", case.name);
                assert!(!shown.contains("open in Inbox"));

                // Expanded in place: the same card.
                ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
                let shown = render(&ui, width, 60);
                check_form(&case, &shown, width, "band");
                assert!(shown.contains("PROMPT · WAITING 1M"), "{}\n{shown}", case.name);
                let flat = words_of(&shown);
                assert!(flat.contains("Ctrl+N to answer with the keys"), "{}\n{shown}", case.name);

                // With the keys, the footer lists them as the harness does.
                ui.toggle_form_keys();
                let shown = render(&ui, width, 60);
                let flat = words_of(&shown);
                assert!(flat.contains("↑/↓ to move") && flat.contains("Esc to fold"), "{}\n{shown}", case.name);
                assert_eq!(
                    flat.contains("Enter to select") || flat.contains("Enter to toggle") || flat.contains("Enter to choose"),
                    case.marked.is_some(),
                    "{}\n{shown}",
                    case.name
                );
                // Esc folds it back to its line.
                press(&mut ui, KeyCode::Esc, KeyModifiers::NONE);
                let shown = render(&ui, width, 60);
                assert!(options(&shown).is_empty(), "{} folded again\n{shown}", case.name);
            }
        }
    }

    fn bash_world() -> World {
        let case = &cases()[1];
        world_with(case)
    }

    /// st's allow is checked again when a tap is dispatched: a card drawn while st allowed its call
    /// may have changed since, because st no longer allows it or can no longer read the seat's
    /// conversation. Then the form withholds the answer, a tap on it sends nothing, and deny still
    /// goes.
    #[test]
    fn an_allow_tap_is_refused_once_st_no_longer_allows_the_call() {
        let cases = cases();
        let tap = |answer: &str| Hit::Alert("attention/one".into(), AlertTap::Answer(answer.into()));
        for case in [&cases[1], &cases[2]] {
            let gated = case.answers[0];
            // While st allows it, the tap answers.
            let mut ui = in_inbox(world_with(case));
            ui.live = true;
            ui.click(tap(gated));
            assert!(matches!(ui.effects.as_slice(), [Effect::Attention { .. }]), "{gated}: {:?}", ui.effects);
            for unread in [false, true] {
                let mut world = world_with(case);
                let Load::Ready(items) = &mut world.attention else {
                    panic!("no attention");
                };
                if let AttentionKind::Prompt { allow, calls, .. } = &mut items[0].kind {
                    if unread {
                        *calls = Err("st cannot read this seat's conversation on host/example: it is out of reach".into());
                    } else {
                        *allow = false;
                    }
                }
                let mut ui = in_inbox(world);
                ui.live = true;
                let shown = render(&ui, 120, 60);
                let why = if unread { "st cannot read this seat's conversation" } else { "Fractal does not let this through" };
                assert!(shown.contains(why), "{gated} {unread}: {shown}");
                assert!(!ui.frame.borrow().hits.iter().any(|(_, hit)| *hit == tap(gated)), "{gated} {unread}: {shown}");
                ui.click(tap(gated));
                assert!(ui.effects.is_empty(), "{gated} {unread}: {:?}", ui.effects);
                ui.click(tap("deny"));
                assert!(
                    matches!(
                        ui.effects.as_slice(),
                        [Effect::Attention { action, answer: Some(answer), .. }] if action == "prompt.respond" && answer == "deny"
                    ),
                    "{gated} {unread}: {:?}",
                    ui.effects
                );
            }
        }
    }

    /// Focus only focuses: arrows and Tab move the mark and send nothing. Enter takes the marked
    /// allow, shown in full; a refusal takes its own number, never Enter.
    #[test]
    fn the_mark_moves_without_answering_and_enter_never_refuses() {
        let mut ui = in_conversation(bash_world());
        ui.live = true;
        render(&ui, 80, 40);
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        render(&ui, 80, 40);
        for code in [KeyCode::Down, KeyCode::Tab, KeyCode::Up, KeyCode::BackTab, KeyCode::Down] {
            press(&mut ui, code, KeyModifiers::NONE);
        }
        assert!(ui.effects.is_empty(), "moving the mark answers nothing");
        let shown = render(&ui, 80, 40);
        assert!(options(&shown).iter().any(|(marked, _, label)| *marked && label == "No"), "{shown}");
        press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
        assert!(ui.effects.is_empty(), "Enter never takes a refusal");
        press(&mut ui, KeyCode::Up, KeyModifiers::NONE);
        render(&ui, 80, 40);
        press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
        let [Effect::Attention { action, answer: Some(answer), .. }] = ui.effects.as_slice() else {
            panic!("{:?}", ui.effects);
        };
        assert_eq!((action.as_str(), answer.as_str()), ("prompt.respond", "allow"));

        // A refusal is its own number.
        let mut ui = in_conversation(bash_world());
        ui.live = true;
        render(&ui, 80, 40);
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        render(&ui, 80, 40);
        press(&mut ui, KeyCode::Char('2'), KeyModifiers::NONE);
        assert!(matches!(ui.effects.as_slice(), [Effect::Attention { answer: Some(answer), .. }] if answer == "deny"));
    }

    /// Typing is never blocked: a letter while the form has the keys goes to the message box.
    #[test]
    fn a_letter_while_the_form_has_the_keys_types_into_the_message_box() {
        let mut ui = in_conversation(bash_world());
        ui.live = true;
        render(&ui, 80, 40);
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(ui.prompt_keys.is_some());
        for letter in "hi".chars() {
            press(&mut ui, KeyCode::Char(letter), KeyModifiers::NONE);
        }
        assert!(ui.effects.is_empty());
        assert!(ui.prompt_keys.is_none());
        assert_eq!(ui.conversation_state.drafts.get(SEAT).map(String::as_str), Some("hi"));
        // And digits typed into the box stay text.
        press(&mut ui, KeyCode::Char('1'), KeyModifiers::NONE);
        assert!(ui.effects.is_empty());
        assert_eq!(ui.conversation_state.drafts.get(SEAT).map(String::as_str), Some("hi1"));
    }

    /// A call too long for the band is cut there and its allow withheld, by click and by key;
    /// Inbox shows it whole and offers allow.
    #[test]
    fn a_call_too_long_for_the_band_is_allowed_only_where_it_is_shown_whole() {
        let mut world = bash_world();
        serve(
            &mut world,
            vec![call("Bash", serde_json::json!({"command": (1..=12).map(|n| format!("echo step {n}")).collect::<Vec<_>>().join("\n")}))],
        );
        let allow = Hit::Alert("attention/one".into(), AlertTap::Answer("allow".into()));
        let mut ui = in_conversation(world.clone());
        ui.live = true;
        ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
        let shown = render(&ui, 80, 40);
        assert!(shown.contains("too long to show here in full"), "{shown}");
        assert!(!shown.contains("echo step 12"), "{shown}");
        assert!(!ui.frame.borrow().hits.iter().any(|(_, hit)| *hit == allow));
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        press(&mut ui, KeyCode::Char('1'), KeyModifiers::NONE);
        assert!(ui.effects.is_empty(), "the key cannot allow what is not shown");
        // Its note is the way to the whole call: a click opens the card in Inbox, selected.
        let open = Hit::Alert("attention/one".into(), AlertTap::Open);
        assert!(ui.frame.borrow().hits.iter().any(|(_, hit)| *hit == open));
        ui.click(open);
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/one"));
        assert!(render(&ui, 120, 60).contains("echo step 12"));

        let ui = in_inbox(world);
        let shown = render(&ui, 120, 60);
        assert!(shown.contains("echo step 12"), "{shown}");
        assert!(ui.frame.borrow().hits.iter().any(|(_, hit)| *hit == allow));
    }

    /// A question is answered with choices made here, by number or Enter, and sent together.
    #[test]
    fn a_question_is_chosen_by_number_and_sent_once_complete() {
        let case = &cases()[3];
        let mut ui = in_inbox(world_with(case));
        ui.live = true;
        render(&ui, 100, 60);
        press(&mut ui, KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(ui.prompt_keys.as_deref(), Some("attention/one"));
        // Enter on the marked option chooses it; nothing is sent.
        press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
        press(&mut ui, KeyCode::Char('2'), KeyModifiers::NONE);
        assert!(ui.effects.is_empty());
        let shown = render(&ui, 100, 60);
        assert!(shown.contains("[✓] Tulips") && shown.contains("[✓] Asters"), "{shown}");
        // Another question call replaces it: the picks made for the first never answer it.
        let asked = served(&ui.world)[0].clone();
        let first = ui.world.attention.items()[0].clone();
        ui.world.attention = Load::Ready(vec![naming(first.clone(), "call-replaced")]);
        serve(&mut ui.world, vec![PendingCall { id: "call-replaced".into(), ..asked.clone() }]);
        let shown = render(&ui, 100, 60);
        assert!(shown.contains("[ ] Tulips") && !shown.contains("[✓]"), "{shown}");
        // The first call again, as st serves it.
        ui.world.attention = Load::Ready(vec![first]);
        let shown = render(&ui, 100, 60);
        assert!(shown.contains("[✓] Tulips") && shown.contains("[✓] Asters"), "its own picks are kept: {shown}");
        press(&mut ui, KeyCode::Char('3'), KeyModifiers::NONE);
        let [Effect::Attention { answer: Some(answer), .. }] = ui.effects.as_slice() else {
            panic!("{:?}", ui.effects);
        };
        assert_eq!(serde_json::from_str::<serde_json::Value>(answer).unwrap(), serde_json::json!({"0": [0, 1]}));
    }

    #[test]
    fn a_click_on_an_option_answers_it_and_a_gone_prompt_says_so() {
        let mut ui = in_conversation(bash_world());
        ui.live = true;
        ui.click(Hit::Alert("attention/one".into(), AlertTap::Answer("deny".into())));
        assert!(matches!(ui.effects.as_slice(), [Effect::Attention { answer: Some(answer), .. }] if answer == "deny"));
        let mut ui = in_conversation(bash_world());
        ui.live = true;
        ui.world.attention = Load::Ready(vec![]);
        ui.click(Hit::Alert("attention/one".into(), AlertTap::Answer("allow".into())));
        assert!(ui.effects.is_empty());
        assert_eq!(ui.flash.as_ref().map(|(text, _)| text.as_str()), Some(GONE));
    }

    /// A prompt that names no call is only denied here, in Inbox and in place, even beside a lone
    /// waiting call: no unanswered call is guessed at, and dispatch refuses the rest too.
    #[test]
    fn a_prompt_that_names_no_call_is_only_denied_and_another_harness_answers_in_its_terminal() {
        for offered in [&["allow", "deny"][..], &["approve", "deny"], &["answer", "deny"]] {
            let mut world = bash_world();
            world.attention = Load::Ready(vec![prompt("attention/one", offered, "")]);
            let ui = in_inbox(world.clone());
            let shown = render(&ui, 120, 60);
            assert!(shown.contains("Fractal does not say which call this asks about"), "{shown}");
            assert!(!shown.contains("make clean"), "{shown}");
            let labels: Vec<String> = options(&shown).into_iter().map(|(_, _, label)| label).collect();
            assert!(matches!(labels.as_slice(), [only] if only.starts_with("No")), "{offered:?}: {labels:?}");
            let mut ui = in_conversation(world.clone());
            ui.live = true;
            ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
            let shown = render(&ui, 80, 40);
            assert!(shown.contains("Fractal does not say which call this asks about"), "{shown}");
            ui.click(Hit::Alert("attention/one".into(), AlertTap::Answer(offered[0].into())));
            assert!(ui.effects.is_empty(), "{offered:?}: {:?}", ui.effects);
            ui.click(Hit::Alert("attention/one".into(), AlertTap::Answer("deny".into())));
            assert!(
                matches!(ui.effects.as_slice(), [Effect::Attention { answer: Some(answer), .. }] if answer == "deny"),
                "{offered:?}: {:?}",
                ui.effects
            );
        }
        let mut world = bash_world();
        world.attention = Load::Ready(vec![prompt("attention/one", &[], "Pi asks for approval")]);
        let ui = in_inbox(world);
        let shown = render(&ui, 120, 60);
        assert!(shown.contains("Answer it in the seat's terminal"), "{shown}");
        assert!(options(&shown).is_empty());
    }

    /// A call auto mode denied is allowed once or for its kind, or dismissed: each its own option,
    /// and each click sends exactly that answer.
    #[test]
    fn a_denied_call_is_allowed_once_or_for_its_kind_or_dismissed() {
        let offered = ["approve-once", "approve-kind", "dismiss"];
        for answer in offered {
            let mut world = bash_world();
            world.attention = Load::Ready(vec![prompt("attention/one", &offered, "")]);
            let mut ui = in_conversation(world);
            ui.live = true;
            ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
            let shown = render(&ui, 120, 40);
            let labels: Vec<String> = options(&shown).into_iter().map(|(_, _, label)| label).collect();
            let expected = ["Yes, allow it once", "Yes, and allow calls of its kind for this seat", "No, leave it denied"];
            assert!(
                labels.len() == expected.len() && labels.iter().zip(expected).all(|(label, words)| label.starts_with(words)),
                "{labels:?}\n{shown}"
            );
            ui.click(Hit::Alert("attention/one".into(), AlertTap::Answer(answer.into())));
            assert!(
                matches!(
                    ui.effects.as_slice(),
                    [Effect::Attention { action, answer: Some(sent), .. }] if action == "prompt.respond" && sent == answer
                ),
                "{answer}: {:?}",
                ui.effects
            );
        }
    }

    /// A question world whose questions offer `options` choices each, `count` questions in all.
    fn questions_world(count: usize, options: usize) -> World {
        let case = &cases()[3];
        let asked: Vec<serde_json::Value> = (0..count)
            .map(|question| {
                serde_json::json!({
                    "question": format!("Which bed {question}?"), "header": format!("Bed {question}"), "multiSelect": false,
                    "options": (0..options).map(|option| serde_json::json!({"label": format!("Plot {option}"), "description": ""})).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut world = world_with(case);
        serve(&mut world, vec![call("AskUserQuestion", serde_json::json!({"questions": asked}))]);
        world
    }

    /// Every row has its own key, past nine too: on a form of ten or more rows a digit that starts
    /// a longer number waits for the second, or for Enter. A refusal is still never under Enter.
    #[test]
    fn every_row_past_nine_has_its_number() {
        // Eight choices, then Submit answers (9) and No (10).
        let mut ui = in_inbox(questions_world(1, 8));
        ui.live = true;
        let shown = render(&ui, 120, 60);
        assert!(shown.contains("10. No"), "{shown}");
        press(&mut ui, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(render(&ui, 120, 60).contains("1–10 to choose"));
        // 1 then Enter is row 1: chosen, nothing sent.
        press(&mut ui, KeyCode::Char('1'), KeyModifiers::NONE);
        assert!(ui.effects.is_empty());
        press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
        assert!(ui.effects.is_empty());
        assert!(render(&ui, 120, 60).contains("✓ Plot 0"));
        // 1 then 0 is row 10, the refusal.
        press(&mut ui, KeyCode::Char('1'), KeyModifiers::NONE);
        press(&mut ui, KeyCode::Char('0'), KeyModifiers::NONE);
        assert!(
            matches!(ui.effects.as_slice(), [Effect::Attention { answer: Some(answer), .. }] if answer == "deny"),
            "{:?}",
            ui.effects
        );
        // A digit no longer number starts with answers at once: 9 is Submit answers.
        let mut ui = in_inbox(questions_world(1, 8));
        ui.live = true;
        render(&ui, 120, 60);
        press(&mut ui, KeyCode::Char('a'), KeyModifiers::NONE);
        press(&mut ui, KeyCode::Char('3'), KeyModifiers::NONE);
        render(&ui, 120, 60);
        press(&mut ui, KeyCode::Char('9'), KeyModifiers::NONE);
        assert!(
            matches!(ui.effects.as_slice(), [Effect::Attention { answer: Some(answer), .. }] if answer.contains("\"0\"")),
            "{:?}",
            ui.effects
        );
    }

    /// A form too tall for the band is never cut: its card says so, offers nothing to answer, and
    /// a click opens it whole in Inbox.
    #[test]
    fn a_form_too_tall_for_the_band_opens_in_inbox() {
        let mut ui = in_conversation(questions_world(4, 4));
        ui.live = true;
        ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
        let shown = render(&ui, 80, 24);
        assert!(shown.contains("Its form does not fit here"), "{shown}");
        let hits: Vec<Hit> = ui.frame.borrow().hits.iter().map(|(_, hit)| hit.clone()).collect();
        assert!(!hits.iter().any(|hit| matches!(hit, Hit::Alert(_, AlertTap::Answer(_) | AlertTap::Choose(..)))), "{shown}");
        let open = Hit::Alert("attention/one".into(), AlertTap::Open);
        assert!(hits.contains(&open), "{shown}");
        // The keys take nothing that is not in view.
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        press(&mut ui, KeyCode::Char('9'), KeyModifiers::NONE);
        assert!(ui.effects.is_empty(), "{:?}", ui.effects);
        ui.click(open.clone());
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/one"));
        // However little room the band has, a row that fits opens it: the card's own rows, or,
        // when only the bar fits, the bar's label.
        let mut ui = in_conversation(questions_world(4, 4));
        ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
        for place in [Place::Band, Place::Terminal] {
            for room in 1..=6 {
                let rows = ui.alert_band(SEAT, 60, room, place);
                let fits = &rows[..rows.len().min(room)];
                assert!(
                    fits.iter().any(|row| row.hits.iter().any(|(_, _, hit)| *hit == open)),
                    "{place:?} room {room}: {:?}",
                    fits.iter().map(|row| text::plain(&row.line)).collect::<Vec<_>>()
                );
                assert!(!fits.iter().any(|row| row.hits.iter().any(|(_, _, hit)| matches!(hit, Hit::Alert(_, AlertTap::Answer(_))))));
            }
        }
        // And in a drawn frame of a short conversation, the way in is on screen whenever the band
        // is (a conversation too short for it draws none; the top bar's count opens Inbox then).
        let mut drawn = 0;
        for height in [10, 12, 14, 16, 18] {
            let shown = render(&ui, 60, height);
            if shown.contains("▾ fold") {
                drawn += 1;
                assert!(ui.frame.borrow().hits.iter().any(|(_, hit)| *hit == open), "height {height}:\n{shown}");
            }
        }
        assert!(drawn > 0, "no height drew the band");
        // With room, the whole form shows in place.
        let mut ui = in_conversation(questions_world(4, 4));
        ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
        let shown = render(&ui, 120, 80);
        assert!(!shown.contains("Its form does not fit here") && shown.contains("Plot 3"), "{shown}");
    }

    /// The band keeps a card between frames only while nothing it was drawn from changed.
    #[test]
    fn a_kept_band_card_follows_its_call_its_mark_and_its_age() {
        let mut ui = in_conversation(bash_world());
        ui.click(Hit::Alert(SEAT.into(), AlertTap::Toggle));
        assert!(render(&ui, 80, 40).contains("make clean"));
        serve(&mut ui.world, vec![call("Bash", serde_json::json!({"command": "make tulips"}))]);
        let shown = render(&ui, 80, 40);
        assert!(shown.contains("make tulips") && !shown.contains("make clean"), "{shown}");
        ui.prompt_marks.insert("attention/one".into(), 1);
        assert!(options(&render(&ui, 80, 40)).iter().any(|(marked, _, label)| *marked && label == "No"));
        if let Load::Ready(items) = &mut ui.world.attention {
            items[0].age = "7m".into();
        }
        assert!(render(&ui, 80, 40).contains("WAITING 7M"));
    }

    /// Inbox follows the conversation of the prompt it shows, where the call it asks about is.
    #[test]
    fn inbox_follows_the_seat_of_the_prompt_it_shows() {
        let ui = in_inbox(bash_world());
        assert!(ui.live_conversations().contains(&SEAT.to_owned()));
    }

    /// A prompt st stops listing stays under Recently closed, offering no answer, in Inbox or in place.
    #[test]
    fn a_closed_prompt_offers_no_answer() {
        let world = bash_world();
        let mut ui = in_inbox(world.clone());
        ui.live = true;
        assert!(options(&render(&ui, 80, 40)).len() == 2);
        let mut next = world;
        next.attention = Load::Ready(vec![]);
        ui.set_world(next);
        let index = ui.ids().iter().position(|id| id == "attention/one").unwrap();
        ui.select(index);
        let shown = render(&ui, 80, 40);
        assert!(shown.contains("closed, x clears it"), "{shown}");
        assert!(options(&shown).is_empty(), "{shown}");
        assert!(!shown.contains("to answer with the keys"), "{shown}");
        assert!(ui.alert_band(SEAT, 80, 20, Place::Band).is_empty());
    }

    /// The bar names Esc only while a form here has the keys, and Esc still folds a form whose
    /// answers went while it had them (st stops offering them once the prompt waits too long).
    #[test]
    fn esc_folds_the_band_it_names_even_after_the_answers_go() {
        let world = bash_world();
        let mut ui = in_conversation(world.clone());
        ui.live = true;
        ui.toggle_alerts(SEAT);
        let bar = |ui: &Ui| ui.alert_band(SEAT, 80, 30, Place::Band)[0].line.to_string();
        assert!(bar(&ui).contains("▾ fold") && !bar(&ui).contains("Esc"), "{}", bar(&ui));
        ui.toggle_alerts(SEAT);
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(bar(&ui).contains("▾ fold · Esc"), "{}", bar(&ui));
        let mut next = world;
        if let Load::Ready(items) = &mut next.attention {
            for item in items.iter_mut() {
                if let AttentionKind::Prompt { answers, .. } = &mut item.kind {
                    answers.clear();
                }
            }
        }
        ui.set_world(next);
        assert!(options(&render(&ui, 80, 40)).is_empty());
        press(&mut ui, KeyCode::Esc, KeyModifiers::NONE);
        assert!(ui.prompt_keys.is_none());
        assert_eq!(ui.alert_band(SEAT, 80, 30, Place::Band).len(), 1, "folded to its line");
        assert!(ui.effects.is_empty());
    }

    #[test]
    fn over_a_terminal_the_band_names_no_key_and_answers_by_click() {
        let mut ui = in_conversation(bash_world());
        let flat = |rows: &[BandRow]| {
            rows.iter()
                .map(|row| row.line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let folded = flat(&ui.alert_band(SEAT, 80, 30, Place::Terminal));
        assert!(folded.contains("▸ show") && !folded.contains("Ctrl+N"), "{folded}");
        // Even with the keys given to its form in the conversation, every key here is the
        // terminal's: the band names none and answers only a click.
        press(&mut ui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        let rows = ui.alert_band(SEAT, 80, 30, Place::Terminal);
        let shown = flat(&rows);
        for key in ["Esc", "Enter", "Ctrl+N", "↑/↓", "to choose", "Ctrl+H"] {
            assert!(!shown.contains(key), "{key} named over the terminal:\n{shown}");
        }
        assert!(shown.contains("click an option"), "{shown}");
        assert!(
            rows.iter().flat_map(|row| &row.hits).any(|(_, _, hit)| matches!(
                hit,
                Hit::Alert(_, AlertTap::Answer(answer)) if answer == "allow"
            )),
            "{shown}"
        );
        // The conversation's band still names them.
        let band = flat(&ui.alert_band(SEAT, 80, 30, Place::Band));
        assert!(band.contains("Esc to fold"), "{band}");
    }

    #[test]
    fn updates_and_messages_are_not_alerts() {
        let mut update = prompt("attention/update", &[], "");
        update.kind = AttentionKind::Update {
            from: "atlas".into(),
            body: "done".into(),
            about: String::new(),
            subjects: vec![],
        };
        let mut world = demo::test_world();
        world.attention = Load::Ready(vec![update]);
        let ui = Ui::new(world);
        assert!(ui.alert_band(SEAT, 80, 20, Place::Band).is_empty());
    }

    #[test]
    fn the_collapsed_line_fits_a_narrow_pane() {
        let ui = in_conversation(bash_world());
        for width in [12usize, 24, 40] {
            assert_eq!(ui.alert_band(SEAT, width, 10, Place::Band).len(), 1);
        }
    }

    fn with_alerts(items: Vec<Attention>) -> Ui {
        let mut world = demo::test_world();
        world.attention = Load::Ready(items);
        Ui::new(world)
    }

    /// The cards, as a person sees them after expanding the band.
    impl Ui {
        fn expanded_band(&self, agent: &str, width: usize) -> Vec<BandRow> {
            self.alert_card(agent, width, 60, Place::Band)
        }
    }

    fn texts(rows: &[BandRow]) -> String {
        rows.iter()
            .map(|row| {
                row.line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn taps(rows: &[BandRow]) -> Vec<Hit> {
        rows.iter()
            .flat_map(|row| row.hits.iter().map(|(_, _, hit)| hit.clone()))
            .collect()
    }

    #[test]
    fn open_selects_the_alert_in_inbox() {
        let mut ui = with_alerts(vec![prompt("attention/a", &[], ""), prompt("attention/b", &[], "")]);
        ui.click(Hit::Alert("attention/b".into(), AlertTap::Open));
        assert_eq!(ui.tab(), 0);
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/b"));
    }

    /// A native choice with stable answer IDs and the existing recommendation field.
    fn request(recommends: Option<(&str, &str)>) -> Attention {
        let mut value = serde_json::json!({
            "version": 1, "type": "choice", "question": "How far does the release go?",
            "why_person": "You own the rollout.",
            "recommendation": recommends.map(|(answer, reason)| {
                serde_json::json!({"answer": answer, "reason": reason})
            }),
            "answers": [
                {"id": "stage", "label": "Staging only", "consequence": "Nobody sees it."},
                {"id": "canary", "label": "Canary", "consequence": "One host."},
                {"id": "hold", "label": "Hold", "consequence": "Nothing."},
                {"id": "all", "label": "Ship to all", "consequence": "Everyone."}
            ]
        });
        if recommends.is_some_and(|(answer, _)| answer == "change") {
            value["type"] = serde_json::json!("decision");
            value["answers"] = serde_json::json!([
                {"id": "all", "label": "Ship to all", "consequence": "Everyone.", "outcome": "accept"},
                {"id": "hold", "label": "Hold", "consequence": "Nothing.", "outcome": "decline"},
                {"id": "change", "label": "Change it", "consequence": "Revised.", "outcome": "request_changes"}
            ]);
        }
        let request: fractal_client::StructuredRequest = serde_json::from_value(value).unwrap();
        Attention {
            deadline: None,
            requested_at: None,
            id: "attention/release".into(),
            tier: Tier::Stopped,
            title: "How far does the release go?".into(),
            waiting: None,
            age: "1m".into(),
            mission: None,
            agent: Some("agent/example/atlas".into()),
            kind: AttentionKind::Request {
                from: "Atlas".into(),
                from_id: "agent/example/atlas".into(),
                question: request.question.clone(),
                structured: Some(Box::new(request)),
                images: Vec::new(),
            },
            actions: vec!["work.done".into()],
            related: vec![],
            raised_by: None,
            blocked: None,
            conversations: vec!["agent/example/atlas".into()],
        }
    }

    fn answer_taps(rows: &[BandRow]) -> Vec<String> {
        taps(rows)
            .into_iter()
            .filter_map(|hit| match hit {
                Hit::Alert(_, AlertTap::Answer(id)) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// Every ask has a deadline: its card says how long is left and what then applies, worded
    /// as the web and phone word it.
    #[test]
    fn an_ask_card_says_how_long_is_left_and_what_then_applies() {
        let now = "2026-10-26T08:00:00Z";
        assert_eq!(
            crate::ui::adapt::ask_deadline("2026-10-26T11:00:00Z", Some("Hold"), now),
            "answer within 3h, or “Hold” applies"
        );
        assert_eq!(
            crate::ui::adapt::ask_deadline("2026-10-28T08:00:00Z", None, now),
            "answer within 2d, or it times out"
        );
        assert_eq!(
            crate::ui::adapt::ask_deadline("2026-10-26T07:00:00Z", None, now),
            "answer within 0s, or it times out"
        );
        let mut item = request(None);
        item.deadline = Some("answer within 3h, or “Hold” applies".into());
        let mut ui = with_alerts(vec![item]);
        ui.live = true;
        let shown = texts(&ui.expanded_band("agent/example/atlas", 120));
        assert!(
            shown.contains("WAITING 1M · ANSWER WITHIN 3H, OR “HOLD” APPLIES"),
            "{shown}"
        );
    }

    /// The asker's recommended answer leads its card even when it is fourth, so the three-button
    /// cap never hides it: starred, green, explained with its reason, and sent by its own id.
    /// Without a recommendation the card is as before; a recommended answer that needs words is
    /// explained but still written in Inbox.
    #[test]
    fn a_recommended_answer_leads_with_its_reason_and_keeps_its_id() {
        let reason = "Canary was clean for a day and the flag can turn it off.";
        let mut ui = with_alerts(vec![request(Some(("all", reason)))]);
        ui.live = true;
        let rows = ui.expanded_band("agent/example/atlas", 80);
        let shown = texts(&rows);
        println!("--- recommends the fourth answer (80 columns) ---\n{shown}");
        assert_eq!(answer_taps(&rows), ["all", "stage", "canary"], "{shown}");
        assert!(shown.contains(&format!("★ recommended: {reason}")), "{shown}");
        let buttons = shown
            .lines()
            .find(|line| line.contains(" ★ Ship to all "))
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(
            buttons.find("★ Ship to all") < buttons.find("Staging only"),
            "{buttons}"
        );
        let star = rows
            .iter()
            .flat_map(|row| &row.line.spans)
            .find(|span| span.content == " ★ Ship to all ")
            .unwrap();
        assert_eq!(star.style.bg, Some(theme::GREEN));
        ui.click(Hit::Alert("attention/release".into(), AlertTap::Answer("all".into())));
        assert!(
            matches!(
                ui.effects.as_slice(),
                [Effect::Attention { action, answer: Some(answer), .. }]
                    if action == "work.done" && answer == "all"
            ),
            "{:?}",
            ui.effects
        );

        let ui = with_alerts(vec![request(None)]);
        let rows = ui.expanded_band("agent/example/atlas", 80);
        let shown = texts(&rows);
        println!("--- no recommendation (80 columns) ---\n{shown}");
        assert_eq!(answer_taps(&rows), ["stage", "canary", "hold"], "{shown}");
        assert!(!shown.contains('★') && !shown.contains("recommended"), "{shown}");

        let ui = with_alerts(vec![request(Some(("change", "The name is wrong.")))]);
        let rows = ui.expanded_band("agent/example/atlas", 80);
        let shown = texts(&rows);
        println!("--- recommends an answer that needs words (80 columns) ---\n{shown}");
        assert_eq!(answer_taps(&rows), ["all", "hold"], "{shown}");
        assert!(shown.contains("★ recommended: The name is wrong."), "{shown}");
        assert!(
            shown.contains("★ Change it needs your words: open in Inbox"),
            "{shown}"
        );

        let ui = with_alerts(vec![request(Some(("unknown", "Stale recommendation.")))]);
        let rows = ui.expanded_band("agent/example/atlas", 80);
        assert_eq!(answer_taps(&rows), ["stage", "canary", "hold"]);
        assert!(!texts(&rows).contains("recommended"));
    }
}
