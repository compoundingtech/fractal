//! Alerts beside the conversation or the terminal of the agent they belong to: one line above the
//! message box and a badge over a terminal, either of which opens the alerts box, which is Home
//! floated over the glass with the agent's first alert selected.
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

    /// The line for `agent` at `width` columns: empty when nothing waits on the person there,
    /// otherwise what waits and that a click opens it. It holds no answer: answering happens in
    /// the box it opens, where a form has the room it needs.
    pub(super) fn alert_band(&self, agent: &str, width: usize) -> Vec<BandRow> {
        let alerts = self.alerts_in(agent);
        let Some(first) = alerts.first() else {
            return Vec::new();
        };
        let count = alerts.len();
        let (glyph, color) = screens::attention_style(&first.kind);
        let lead = format!(" ◆ {count} alert{} · ", if count == 1 { "" } else { "s" });
        let tail = " ▸ open ";
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
                Hit::Alert(agent.to_owned(), AlertTap::Box),
            )],
        }]
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
    fn the_line_says_what_waits_and_has_one_target_that_opens_the_box() {
        let ui = with_alerts(vec![
            prompt("attention/a", &["allow", "deny"]),
            prompt("attention/b", &[]),
        ]);
        let rows = ui.alert_band("agent/example/atlas", 80);
        assert_eq!(rows.len(), 1);
        let shown = texts(&rows);
        assert!(shown.contains("2 alerts") && shown.contains("is waiting for a permission"), "{shown}");
        assert!(shown.contains("▸ open"), "{shown}");
        assert_eq!(
            rows[0].hits.iter().map(|(_, _, hit)| hit.clone()).collect::<Vec<_>>(),
            vec![Hit::Alert("agent/example/atlas".into(), AlertTap::Box)]
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
        ui.click(Hit::Alert("agent/example/atlas".into(), AlertTap::Box));
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
}
