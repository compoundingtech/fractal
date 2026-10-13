//! Alerts beside the conversation or the terminal of the agent they belong to: one line above the
//! message box that opens into a read-only list (title and the first lines of what each says, three
//! at a time, scrolling), and a badge over a terminal. Nothing in the list answers anything: a click
//! on an alert, the badge or Ctrl+N opens the alerts box, which is Home floated over the glass with
//! that alert selected, where the whole of it is read and answered.
//!
//! An alert is anything that blocks or waits on an answer: an ask, a human gate, a launch or
//! revision approval, a native harness prompt or a harness login. It clears itself when it is
//! answered anywhere, so what shows is exactly what st lists and nothing it remembers.

use super::doc::AlertTap;
use super::*;

/// One row above the box: its line, and the buttons on it by column.
pub(super) struct BandRow {
    pub(super) line: Line<'static>,
    pub(super) hits: Vec<(u16, u16, Hit)>,
}

/// How many alerts the opened list shows at once, and how many lines of what an alert says.
const ALERTS_SHOWN: usize = 3;
const SUMMARY_LINES: usize = 2;

impl Ui {
    /// The open alerts that show in `agent`'s conversation, in the order Now lists them.
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

    /// How many alerts wait on the person, whichever agent they are about.
    pub(super) fn open_alert_count(&self) -> usize {
        self.world
            .attention
            .items()
            .iter()
            .filter(|item| {
                item.is_alert() && !self.snoozed.contains(&item.id) && !self.closed.contains(&item.id)
            })
            .count()
    }

    /// What an alert says, in a few words: the question, the reason, the call it waits on.
    fn alert_summary(item: &Attention) -> String {
        let text = match &item.kind {
            AttentionKind::Review { question, because, .. } => format!("{question} {because}"),
            AttentionKind::Feedback { question, .. } => question.clone(),
            AttentionKind::Launch { planner, name, .. } => format!("{planner} proposes {name}"),
            AttentionKind::Revision { reason, .. } => reason.clone(),
            AttentionKind::Fault { what, because, .. } => format!("{what} {because}"),
            AttentionKind::Message { body, .. } => body.clone(),
            AttentionKind::Prompt { text, .. } => text.clone(),
            AttentionKind::Login { text, .. } => text.clone(),
            AttentionKind::Request { question, structured, .. } => structured
                .as_ref()
                .map_or_else(|| question.clone(), |request| request.question.clone()),
            _ => String::new(),
        };
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The lines above the box for `agent` at `width` columns: none when nothing waits there; one
    /// that says what waits until it is opened; opened, a read-only list with each alert's title
    /// and the first lines of what it says, three at a time, with rows to scroll the rest. Every
    /// row is a way in to the alerts box: no answer is offered here, because an answer is made
    /// after reading all of it.
    pub(super) fn alert_band(&self, agent: &str, width: usize) -> Vec<BandRow> {
        let alerts = self.alerts_in(agent);
        let Some(first) = alerts.first() else {
            return Vec::new();
        };
        let count = alerts.len();
        let open = self.alerts_open.contains(agent);
        let toggle = Hit::Alert(agent.to_owned(), AlertTap::Toggle);
        if !open {
            let (glyph, color) = screens::attention_style(&first.kind);
            let lead = format!(" ◆ {count} alert{} · ", if count == 1 { "" } else { "s" });
            let tail = " ▸ show ";
            let room = width
                .saturating_sub(text::width(&lead) + text::width(tail) + 4)
                .max(8);
            return vec![BandRow {
                line: Line::from(vec![
                    Span::styled(lead, theme::strong(theme::PERSON)),
                    Span::styled(format!("{glyph} "), theme::fg(color)),
                    Span::styled(
                        text::truncate(first_line(&first.title), room),
                        theme::text(),
                    ),
                    Span::styled(tail, theme::dim()),
                ]),
                hits: vec![(0, width as u16, toggle)],
            }];
        }
        // Opened: a title bar the width of the pane, which hides it again.
        let left = format!(" ◆ {count} alert{} ", if count == 1 { "" } else { "s" });
        let right = " ▾ hide ";
        let gap = width.saturating_sub(text::width(&left) + text::width(right));
        let bar = |style: Style| style.bg(theme::SURFACE0);
        let mut rows = vec![BandRow {
            line: Line::from(vec![
                Span::styled(left, bar(theme::strong(theme::PERSON))),
                Span::styled(" ".repeat(gap), bar(Style::default())),
                Span::styled(right, bar(theme::dim())),
            ]),
            hits: vec![(0, width as u16, toggle)],
        }];
        let inner = width.saturating_sub(4).max(10);
        let first_shown = self
            .alerts_scroll
            .get(agent)
            .copied()
            .unwrap_or(0)
            .min(count.saturating_sub(ALERTS_SHOWN));
        let scroll_row = |label: String, delta: isize| BandRow {
            line: Line::from(Span::styled(label, theme::fg(theme::OVERLAY1))),
            hits: vec![(
                0,
                width as u16,
                Hit::Alert(agent.to_owned(), AlertTap::Scroll(delta)),
            )],
        };
        if first_shown > 0 {
            rows.push(scroll_row(format!("   ▲ {first_shown} earlier"), -1));
        }
        for item in alerts.iter().skip(first_shown).take(ALERTS_SHOWN) {
            let (glyph, color) = screens::attention_style(&item.kind);
            let opens = Hit::Alert(item.id.clone(), AlertTap::Open);
            let mut item_rows = vec![BandRow {
                line: Line::from(vec![
                    Span::styled(format!(" {glyph} "), theme::fg(color)),
                    Span::styled(
                        text::truncate(first_line(&item.title), inner.saturating_sub(8)),
                        theme::bold(),
                    ),
                    Span::styled(format!("  {} ago", item.age), theme::dim()),
                ]),
                hits: Vec::new(),
            }];
            let summary = Self::alert_summary(item);
            if !summary.is_empty() {
                let mut lines = text::wrap(
                    &[text::run(summary, theme::soft())],
                    inner,
                    &[text::run("   ", theme::dim())],
                    &[text::run("   ", theme::dim())],
                    None,
                );
                let cut = lines.len() > SUMMARY_LINES;
                lines.truncate(SUMMARY_LINES);
                if cut && let Some(last) = lines.last_mut() {
                    last.spans.push(Span::styled(" …", theme::dim()));
                }
                item_rows.extend(lines.into_iter().map(|line| BandRow {
                    line,
                    hits: Vec::new(),
                }));
            }
            // A bar down the left in the alert's colour; every row of it opens the alert.
            for row in &mut item_rows {
                row.line
                    .spans
                    .insert(0, Span::styled("▌", theme::fg(color)));
                row.hits = vec![(0, width as u16, opens.clone())];
            }
            rows.extend(item_rows);
        }
        let below = count.saturating_sub(first_shown + ALERTS_SHOWN);
        if below > 0 {
            rows.push(scroll_row(format!("   ▼ {below} more"), 1));
        }
        rows
    }

    /// Scroll the opened list by `delta` alerts, within what exists.
    pub(super) fn scroll_alerts(&mut self, agent: &str, delta: isize) {
        let count = self.alerts_in(agent).len();
        let last = count.saturating_sub(ALERTS_SHOWN);
        let at = self.alerts_scroll.get(agent).copied().unwrap_or(0);
        let next = at.saturating_add_signed(delta).min(last);
        self.alerts_scroll.insert(agent.to_owned(), next);
    }

    /// Open or close the list of `agent`'s alerts above its box.
    pub(super) fn toggle_alerts(&mut self, agent: &str) {
        if !self.alerts_open.remove(agent) {
            self.alerts_open.insert(agent.to_owned());
        }
    }

    /// A small badge, bottom right of `body`, saying how many alerts wait; a click opens the box.
    /// Nothing is drawn when none wait. It sits over a few cells of a terminal's own output, and
    /// a click on it is the box's, never the program's.
    pub(super) fn draw_alert_badge(&self, buf: &mut Buffer, body: Rect, agent: &str) {
        let count = self.open_alert_count();
        if count == 0 || body.width < 12 || body.height < 3 {
            return;
        }
        let label = format!(" ◆ {count} ");
        let width = text::width(&label) as u16;
        let rect = Rect {
            x: body.x + body.width - width - 1,
            y: body.y + body.height - 1,
            width,
            height: 1,
        };
        buf.set_stringn(
            rect.x,
            rect.y,
            &label,
            width as usize,
            Style::default()
                .fg(theme::CRUST)
                .bg(theme::PERSON)
                .add_modifier(Modifier::BOLD),
        );
        self.alert_badge.set(Some(rect));
        self.hit(rect, Hit::Alert(agent.to_owned(), AlertTap::Box));
    }

    /// Open the alerts box: Home over the glass, with `agent`'s first alert selected (or the
    /// first of all when none is about it). One alert is the card at once; several are the list,
    /// and a click or the arrow keys and Enter go into one.
    pub(super) fn open_alerts(&mut self, agent: &str) {
        let first = self
            .alerts_in(agent)
            .first()
            .map(|item| item.id.clone())
            .or_else(|| {
                self.world
                    .attention
                    .items()
                    .iter()
                    .find(|item| {
                        item.is_alert()
                            && !self.snoozed.contains(&item.id)
                            && !self.closed.contains(&item.id)
                    })
                    .map(|item| item.id.clone())
            });
        match first {
            Some(id) => self.open_alert(&id),
            None => self.flash("Nothing waits on you"),
        }
    }

    /// Open an alert's whole card in Now, selected.
    pub(super) fn open_alert(&mut self, id: &str) {
        if self.glasses.is_some() {
            self.open_home();
        } else {
            self.switch_tab(0);
        }
        if let Some(index) = self
            .listing_for(0, 40)
            .ids
            .iter()
            .position(|item| item == id)
        {
            self.select(index);
        }
    }
}

/// The first non-empty line, without markdown's heading marks.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(id: &str, answers: &[&str]) -> Attention {
        Attention {
            id: id.into(),
            tier: Tier::Stopped,
            title: "agent/example/atlas is waiting for a permission".into(),
            waiting: None,
            age: "1m".into(),
            mission: None,
            agent: Some("agent/example/atlas".into()),
            kind: AttentionKind::Prompt {
                seat: "atlas".into(),
                seat_id: "agent/example/atlas".into(),
                text: "Claude asks to use Bash".into(),
                answers: answers.iter().map(|answer| (*answer).to_owned()).collect(),
                episode: "episode-1".into(),
            },
            actions: if answers.is_empty() {
                vec![]
            } else {
                vec!["prompt.respond".into()]
            },
            related: vec![],
            raised_by: None,
            blocked: None,
            conversations: vec!["agent/example/atlas".into()],
        }
    }

    fn with_alerts(items: Vec<Attention>) -> Ui {
        let mut world = demo::world();
        world.attention = Load::Ready(items);
        Ui::new(world)
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

    #[test]
    fn nothing_waiting_shows_no_line() {
        let ui = with_alerts(vec![]);
        assert!(ui.alert_band("agent/example/atlas", 80).is_empty());
    }

    #[test]
    fn an_alert_shows_only_beside_the_agent_it_belongs_to() {
        let ui = with_alerts(vec![prompt("attention/one", &["allow", "deny"])]);
        assert!(!ui.alert_band("agent/example/atlas", 80).is_empty());
        assert!(ui.alert_band("agent/example/other", 80).is_empty());
    }

    #[test]
    fn the_line_says_what_waits_and_has_one_target_that_opens_the_list() {
        let ui = with_alerts(vec![
            prompt("attention/a", &["allow", "deny"]),
            prompt("attention/b", &[]),
        ]);
        let rows = ui.alert_band("agent/example/atlas", 80);
        assert_eq!(rows.len(), 1);
        let shown = texts(&rows);
        assert!(shown.contains("2 alerts") && shown.contains("is waiting for a permission"), "{shown}");
        assert!(shown.contains("▸ show"), "{shown}");
        assert_eq!(
            rows[0].hits.iter().map(|(_, _, hit)| hit.clone()).collect::<Vec<_>>(),
            vec![Hit::Alert("agent/example/atlas".into(), AlertTap::Toggle)]
        );
        for width in [12usize, 24, 40] {
            assert_eq!(ui.alert_band("agent/example/atlas", width).len(), 1);
        }
    }

    #[test]
    fn updates_and_messages_are_not_alerts() {
        let mut update = prompt("attention/update", &[]);
        update.kind = AttentionKind::Update {
            from: "atlas".into(),
            body: "done".into(),
            about: String::new(),
            subjects: vec![],
        };
        let mut message = prompt("attention/message", &[]);
        message.kind = AttentionKind::Message {
            from: "atlas".into(),
            body: "hi".into(),
        };
        let ui = with_alerts(vec![update, message]);
        assert!(ui.alert_band("agent/example/atlas", 80).is_empty());
        assert_eq!(ui.open_alert_count(), 0);
    }

    #[test]
    fn opening_the_box_selects_the_agents_first_alert_in_now() {
        let mut other = prompt("attention/other", &[]);
        other.agent = Some("agent/example/docs".into());
        other.conversations = vec!["agent/example/docs".into()];
        let mut ui = with_alerts(vec![other, prompt("attention/mine", &[])]);
        ui.open_alerts("agent/example/atlas");
        assert_eq!(ui.tab, 0);
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/mine"));
        // With none about the agent, the first alert of all.
        let mut ui = with_alerts(vec![prompt("attention/mine", &[])]);
        ui.open_alerts("agent/example/nobody");
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/mine"));
        // With none at all, it says so and opens nothing.
        let mut ui = with_alerts(vec![]);
        ui.open_alerts("agent/example/atlas");
        assert!(ui.flash.as_ref().is_some_and(|(text, _)| text.contains("Nothing waits")));
    }

    #[test]
    fn open_selects_the_alert_in_now() {
        let mut ui = with_alerts(vec![prompt("attention/a", &[]), prompt("attention/b", &[])]);
        ui.open_alert("attention/b");
        assert_eq!(ui.tab, 0);
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/b"));
    }

    fn six(ui_agent: &str) -> Vec<Attention> {
        (0..6)
            .map(|index| {
                let mut item = prompt(&format!("attention/{index}"), &["allow", "deny"]);
                item.title = format!("Approve change number {index}");
                item.agent = Some(ui_agent.into());
                item.conversations = vec![ui_agent.into()];
                if let AttentionKind::Prompt { text, .. } = &mut item.kind {
                    *text = format!(
                        "Claude asks to run a long command for item {index} that goes on and on so that it needs more than two lines to say what it does and where, and the rest should be cut with an ellipsis"
                    );
                }
                item
            })
            .collect()
    }

    #[test]
    fn opened_the_list_is_read_only_shows_three_titles_with_two_lines_each() {
        let agent = "agent/example/atlas";
        let mut ui = with_alerts(six(agent));
        ui.click(Hit::Alert(agent.into(), AlertTap::Toggle));
        let rows = ui.alert_band(agent, 80);
        let shown = texts(&rows);
        assert!(shown.contains("6 alerts") && shown.contains("▾ hide"), "{shown}");
        for index in 0..3 {
            assert!(shown.contains(&format!("Approve change number {index}")), "{shown}");
        }
        assert!(!shown.contains("number 3"), "only three at a time: {shown}");
        assert!(shown.contains("▼ 3 more") && !shown.contains('▲'), "{shown}");
        // Read only: nothing here answers anything, and every alert row opens it.
        for word in ["allow", "deny", "Approve ", "accept", "answer"] {
            assert!(!shown.contains(&format!("[{word}")) && !shown.contains(&format!(" {word} ")), "{word}: {shown}");
        }
        assert!(shown.contains('…'), "a long text is cut with an ellipsis: {shown}");
        // Two lines of text and a title per alert, plus the bar and the more-row: at most 12.
        assert!(rows.len() <= 3 * 3 + 2, "{}", rows.len());
        let opens: Vec<Hit> = rows
            .iter()
            .flat_map(|row| row.hits.iter().map(|(_, _, hit)| hit.clone()))
            .filter(|hit| matches!(hit, Hit::Alert(_, AlertTap::Open)))
            .collect();
        assert!(opens.contains(&Hit::Alert("attention/0".into(), AlertTap::Open)));
    }

    #[test]
    fn the_list_scrolls_through_all_six_and_a_click_on_an_alert_opens_it_in_the_box() {
        let agent = "agent/example/atlas";
        let mut ui = with_alerts(six(agent));
        ui.click(Hit::Alert(agent.into(), AlertTap::Toggle));
        ui.click(Hit::Alert(agent.into(), AlertTap::Scroll(1)));
        ui.click(Hit::Alert(agent.into(), AlertTap::Scroll(1)));
        ui.click(Hit::Alert(agent.into(), AlertTap::Scroll(1)));
        ui.click(Hit::Alert(agent.into(), AlertTap::Scroll(1)));
        let shown = texts(&ui.alert_band(agent, 80));
        assert!(shown.contains("number 5") && shown.contains("number 3"), "{shown}");
        assert!(shown.contains("▲ 3 earlier") && !shown.contains('▼'), "stops at the end: {shown}");
        ui.click(Hit::Alert(agent.into(), AlertTap::Scroll(-9)));
        assert!(texts(&ui.alert_band(agent, 80)).contains("number 0"));
        // A click on one opens the box on it.
        ui.click(Hit::Alert("attention/4".into(), AlertTap::Open));
        assert!(ui.home_open() || ui.tab == 0);
        assert_eq!(ui.attention_focus().as_deref(), Some("attention/4"));
    }
}
