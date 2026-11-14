//! One thread panel for Inbox, agent, mission and tab hosts. The host supplies the
//! authoritative thread and records after checking the negotiated capability.

use fractal_conversation_ui::doc::Target;
use ratatui::{
    style::Style,
    text::{Line, Span},
};

use super::{composer, doc::DocExt, text, theme};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    Start(String),
    CombineSavedDrafts(String),
    MessageHeader(String),
    Earlier(String),
    Toggle(String),
    Compose(String),
    Send(String),
    Retry(String),
    LeaveComposer(String),
    Reply { thread: String, message: String },
    Rename(String),
    Close(String),
    Reopen(String),
    OpenTab(String),
    MarkVisibleRead(String),
}

pub type Doc = fractal_conversation_ui::doc::Doc<Tap>;

/// Shown in a thread tab instead of a composer when this connection may not change threads.
pub const READ_ONLY: &str = "Read-only on this device: replies and changes need write access.";

pub struct Summary<'a> {
    pub id: &'a str,
    pub title: &'a str,
    /// The agent the thread talks to, as the person knows it (its display name).
    pub agent: &'a str,
    pub closed: bool,
    pub message_count: usize,
    pub unread: bool,
}

pub enum Record<'a> {
    Message {
        id: &'a str,
        sender: &'a str,
        age: &'a str,
        text: &'a str,
        reply_to: Option<&'a str>,
        from_person: bool,
    },
    Renamed {
        actor: &'a str,
        previous_title: &'a str,
        title: &'a str,
        age: &'a str,
    },
    Closed {
        actor: &'a str,
        age: &'a str,
    },
    Reopened {
        actor: &'a str,
        age: &'a str,
    },
}

#[derive(Default)]
pub struct State<'a> {
    /// The requested disclosure. An open thread draws `▾` even while its history is paused
    /// or loading, so the chevron never claims a collapse the user did not ask for.
    pub open: bool,
    /// The open thread has no history to draw yet; only the heading and notice render.
    pub awaiting_history: bool,
    /// One status line drawn directly under this thread's own heading.
    pub notice: Option<&'a str>,
    /// The thread the keyboard acts on in a list of threads; its heading gets the gutter.
    pub selected: bool,
    pub standalone: bool,
    pub draft: &'a str,
    pub cursor: Option<usize>,
    pub reply_to: Option<&'a str>,
    pub hovered_message: Option<&'a str>,
    pub has_earlier: bool,
    pub loading_earlier: bool,
    pub can_mark_read: bool,
    pub can_reply: bool,
    pub pending_reply: bool,
    pub hotkeys: bool,
    /// The host types any bare letter into a message box (an agent's conversation, a thread
    /// tab), so its thread actions take Alt and their keycaps read `alt+z`, not `z`.
    pub alt_keys: bool,
    pub can_rename: bool,
    pub can_close: bool,
    pub can_reopen: bool,
}

/// Adapt a real daemon window to the native component. Older metadata-only daemons omit the
/// message facts needed to render this component, so callers keep their feature gate closed.
pub fn client_panel(
    summary: &fractal_client::ThreadSummary,
    events: &[fractal_client::ThreadTimelineEvent],
    state: &State<'_>,
    person: &str,
    actor_name: impl Fn(&str) -> Option<String>,
    now: &str,
    width: usize,
) -> Option<Doc> {
    use fractal_client::{ThreadState, ThreadTimelineChange};

    let agent = actor_name(&summary.agent_id).unwrap_or_else(|| summary.agent_id.clone());
    let summary = Summary {
        id: &summary.id,
        title: &summary.title,
        agent: &agent,
        closed: summary.state == ThreadState::Closed,
        message_count: summary.message_count?.try_into().ok()?,
        unread: summary.unread?,
    };
    let known_messages: std::collections::BTreeMap<_, _> = events
        .iter()
        .filter_map(|event| match &event.change {
            ThreadTimelineChange::Message { content, .. } => {
                Some((event.id.as_str(), content.as_str()))
            }
            _ => None,
        })
        .collect();
    let presentation: Vec<_> = events
        .iter()
        .map(|event| {
            let name = if event.actor_id == person {
                "You".to_owned()
            } else {
                actor_name(&event.actor_id).unwrap_or_else(|| event.actor_id.clone())
            };
            let age = i64::try_from(event.accepted_at_unix_ms)
                .ok()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|time| crate::age_label(&time.to_rfc3339(), now))
                .unwrap_or_default();
            (name, age)
        })
        .collect();
    // The daemon sends newest first. Reverse its authoritative order, never re-sort by clocks.
    let records: Vec<_> = events
        .iter()
        .zip(&presentation)
        .rev()
        .map(|(event, (name, age))| match &event.change {
            ThreadTimelineChange::Message {
                content,
                thread_quote_id,
            } => Record::Message {
                id: &event.id,
                sender: name,
                age,
                text: content,
                reply_to: thread_quote_id
                    .as_deref()
                    .map(|id| known_messages.get(id).copied().unwrap_or(id)),
                from_person: event.actor_id == person,
            },
            ThreadTimelineChange::Renamed {
                previous_title,
                title,
            } => Record::Renamed {
                actor: name,
                previous_title,
                title,
                age,
            },
            ThreadTimelineChange::Closed => Record::Closed { actor: name, age },
            ThreadTimelineChange::Reopened => Record::Reopened { actor: name, age },
        })
        .collect();
    Some(panel(&summary, &records, state, width))
}

/// `selected` marks the thread the keyboard acts on the way the Inbox marks its selected
/// row: a `▌` gutter (visible without color) on a full-width selected-row background.
pub(super) fn heading(
    title: &str,
    metadata: &str,
    open: bool,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let title = inline(title);
    let gutter = if selected && width > 2 { "▌" } else { "" };
    let marker = if width == 1 {
        if open { "▾" } else { "▸" }
    } else if open {
        "▾ "
    } else {
        "▸ "
    };
    let room = width.saturating_sub(text::width(gutter) + text::width(marker));
    let title_min = text::width(&title).min(12);
    let metadata = if text::width(metadata) <= room.saturating_sub(title_min) {
        metadata.to_owned()
    } else {
        String::new()
    };
    let title = text::truncate(&title, room.saturating_sub(text::width(&metadata)));
    let row = |style: Style| {
        if gutter.is_empty() {
            style
        } else {
            style.bg(theme::ROW_SELECTED)
        }
    };
    let mut spans = Vec::with_capacity(5);
    if !gutter.is_empty() {
        spans.push(Span::styled(gutter, row(theme::fg(theme::ACCENT))));
    }
    let used = text::width(gutter)
        + text::width(marker)
        + text::width(&title)
        + text::width(&metadata);
    spans.extend([
        Span::styled(marker, row(theme::strong(theme::SAPPHIRE))),
        Span::styled(title, row(theme::strong(theme::SAPPHIRE))),
        Span::styled(metadata, row(theme::dim())),
    ]);
    if !gutter.is_empty() {
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used)),
            row(Style::default()),
        ));
    }
    Line::from(spans)
}

/// A collapsed embed takes exactly one line. The header remains the toggle when open.
/// Activity appears only when the thread has an accepted record to show.
pub fn panel(
    summary: &Summary<'_>,
    records: &[Record<'_>],
    state: &State<'_>,
    width: usize,
) -> Doc {
    let mut doc = Doc::new();
    if width == 0 {
        return doc;
    }
    if !state.standalone {
        let metadata = format!(
            " · {} message{}{}{}",
            summary.message_count,
            if summary.message_count == 1 { "" } else { "s" },
            if summary.closed { " · closed" } else { "" },
            if summary.unread { " · unread" } else { "" },
        );
        doc.line(heading(
            summary.title,
            &metadata,
            state.open,
            state.selected,
            width,
        ));
        target(&mut doc, 0, 0, width, Tap::Toggle(summary.id.to_owned()));
        if !state.open {
            return doc;
        }
    } else {
        // A tab has no disclosure heading, so it names the thread, its agent and its state.
        let title = inline(summary.title);
        let metadata = format!(
            " · to {} · {}",
            inline(summary.agent),
            if summary.closed { "closed" } else { "open" },
        );
        let metadata = if text::width(&metadata) + text::width(&title).min(12) <= width {
            metadata
        } else {
            String::new()
        };
        doc.line(Line::from(vec![
            Span::styled(
                text::truncate(&title, width.saturating_sub(text::width(&metadata))),
                theme::strong(theme::SAPPHIRE),
            ),
            Span::styled(metadata, theme::dim()),
        ]));
        doc.blank();
    }
    if let Some(notice) = state.notice {
        doc.wrap(&[text::run(notice, theme::dim())], width);
    }
    if state.awaiting_history {
        return doc;
    }

    if state.loading_earlier {
        doc.line(Line::from(Span::styled(
            "Loading earlier messages…",
            theme::dim(),
        )));
    } else if state.has_earlier {
        doc.buttons(&[(
            "↑",
            "Load earlier messages",
            Tap::Earlier(summary.id.into()),
            theme::OVERLAY1,
        )]);
        doc.blank();
    }
    for record in records {
        match record {
            Record::Message {
                id,
                sender,
                age,
                text: body,
                reply_to,
                from_person,
            } => {
                let line = doc.lines.len();
                let id = id.to_string();
                doc.entries.push((id.clone(), line));
                let age = text::truncate(&format!(" · {}", inline(age)), width);
                let sender =
                    text::truncate(&inline(sender), width.saturating_sub(text::width(&age)));
                let mut header = vec![
                    Span::styled(
                        sender,
                        theme::strong(if *from_person {
                            theme::BLUE
                        } else {
                            theme::SAPPHIRE
                        }),
                    ),
                    Span::styled(age, theme::dim()),
                ];
                let column = header.iter().map(Span::width).sum::<usize>();
                let reply = "  ↩ Reply";
                if state.can_reply
                    && state.hovered_message == Some(id.as_str())
                    && column + text::width(reply) <= width
                {
                    header.push(Span::styled(reply, theme::fg(theme::SAPPHIRE)));
                    target(
                        &mut doc,
                        line,
                        column + 2,
                        text::width(reply) - 2,
                        Tap::Reply {
                            thread: summary.id.to_owned(),
                            message: id.clone(),
                        },
                    );
                }
                doc.line(Line::from(header));
                if let Some(quote) = reply_to {
                    doc.line(Line::from(Span::styled(
                        text::truncate(&format!("↳ {}", inline(quote)), width),
                        theme::dim(),
                    )));
                }
                let start = doc.lines.len();
                doc.lines(text::markdown(body, width, theme::text()));
                doc.messages.push((id, start..doc.lines.len()));
                doc.blank();
            }
            Record::Renamed {
                actor,
                previous_title,
                title,
                age,
            } => event(
                &mut doc,
                actor,
                &format!(
                    "renamed “{}” to “{}”",
                    inline(previous_title),
                    inline(title)
                ),
                age,
                width,
            ),
            Record::Closed { actor, age } => {
                event(&mut doc, actor, "closed the thread", age, width)
            }
            Record::Reopened { actor, age } => {
                event(&mut doc, actor, "reopened the thread", age, width)
            }
        }
    }
    let has_messages = records
        .iter()
        .any(|record| matches!(record, Record::Message { .. }));
    if !has_messages && !state.has_earlier && !state.loading_earlier {
        doc.line(Line::from(Span::styled("No messages yet.", theme::dim())));
        doc.blank();
    }
    if state.can_reply {
        let focused = state.cursor.is_some();
        doc.line(if state.pending_reply && !focused {
            composer::pending_divider(width, "Send reply")
        } else {
            composer::divider(width, focused)
        });
        if let Some(quote) = state.reply_to {
            doc.line(Line::from(Span::styled(
                text::truncate(&format!("Reply to {}", inline(quote)), width),
                theme::dim(),
            )));
        }
        let start = doc.lines.len();
        doc.lines(composer::text(
            width,
            focused,
            state.draft,
            state.cursor,
            "Message thread · type or click".to_owned(),
        ));
        if !state.pending_reply {
            for line in start..doc.lines.len() {
                target(
                    &mut doc,
                    line,
                    0,
                    width,
                    Tap::Compose(summary.id.to_owned()),
                );
            }
        }
        if state.pending_reply && focused {
            doc.line(composer::pending("Send reply"));
        } else if focused && !state.pending_reply {
            doc.buttons_fit(
                &[
                    (
                        "enter",
                        "Send",
                        Tap::Send(summary.id.to_owned()),
                        theme::ACCENT,
                    ),
                    (
                        "esc",
                        "Leave composer",
                        Tap::LeaveComposer(summary.id.to_owned()),
                        theme::OVERLAY1,
                    ),
                ],
                width,
            );
        }
        doc.blank();
    } else if state.standalone && !(state.can_rename || state.can_close || state.can_reopen) {
        doc.wrap(&[text::run(READ_ONLY, theme::dim())], width);
        doc.blank();
    }

    let (label, lifecycle) = if summary.closed {
        ("Reopen thread", Tap::Reopen(summary.id.to_owned()))
    } else {
        ("Close thread", Tap::Close(summary.id.to_owned()))
    };
    let keycap = |bare: &'static str, chord: &'static str| match (state.hotkeys, state.alt_keys) {
        (false, _) => "",
        (true, false) => bare,
        (true, true) => chord,
    };
    let mut actions = Vec::new();
    if summary.closed && state.can_reopen || !summary.closed && state.can_close {
        actions.push((
            keycap("z", "alt+z"),
            label,
            lifecycle,
            theme::OVERLAY1,
        ));
    }
    if state.can_rename {
        actions.push((
            keycap("n", "alt+n"),
            "Rename",
            Tap::Rename(summary.id.to_owned()),
            theme::SAPPHIRE,
        ));
    }
    if !state.standalone {
        actions.push((
            keycap("p", "alt+p"),
            "Open in tab",
            Tap::OpenTab(summary.id.to_owned()),
            theme::SAPPHIRE,
        ));
    }
    if summary.unread && state.can_mark_read {
        actions.push((
            keycap("v", "alt+v"),
            "Mark visible replies read",
            Tap::MarkVisibleRead(summary.id.to_owned()),
            theme::OVERLAY1,
        ));
    }
    doc.buttons_fit(&actions, width);
    doc
}

fn target(doc: &mut Doc, line: usize, column: usize, width: usize, tap: Tap) {
    doc.targets.push(Target {
        line,
        column: column as u16,
        width: width as u16,
        hit: tap,
    });
}

fn inline(value: &str) -> String {
    text::sanitize(value).replace('\n', " ")
}

fn event(doc: &mut Doc, actor: &str, action: &str, age: &str, width: usize) {
    doc.wrap(
        &[text::run(
            format!("· {} {action} · {}", inline(actor), inline(age)),
            theme::dim(),
        )],
        width,
    );
    doc.blank();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editable<'a>() -> State<'a> {
        State {
            hotkeys: true,
            can_reply: true,
            can_rename: true,
            can_close: true,
            can_reopen: true,
            ..State::default()
        }
    }

    #[test]
    fn canonical_window_renders_received_replies_in_server_order_under_clock_skew() {
        let summary: fractal_client::ThreadSummary = serde_json::from_value(serde_json::json!({
            "id":"thread/1", "agent_id":"agent/atlas", "title":"Worker test", "revision":"closed",
            "title_revision":"created", "state":"closed", "message_count":2, "unread":true,
        }))
        .unwrap();
        let events =
            serde_json::from_value::<Vec<fractal_client::ThreadTimelineEvent>>(serde_json::json!([
                {"id":"message/2","actor_id":"agent/atlas","accepted_at_unix_ms":60_000,
                 "kind":"message","content":"CI passed.","thread_quote_id":"message/1"},
                {"id":"event/closed","actor_id":"person/operator","accepted_at_unix_ms":120_000,
                 "kind":"thread.closed"},
                {"id":"message/1","actor_id":"person/operator","accepted_at_unix_ms":180_000,
                 "kind":"message","content":"Can you check CI?","thread_quote_id":null}
            ]))
            .unwrap();
        let doc = client_panel(
            &summary,
            &events,
            &State {
                open: true,
                ..editable()
            },
            "person/operator",
            |id| (id == "agent/atlas").then(|| "Atlas Builder".to_owned()),
            "1970-01-01T00:04:00Z",
            80,
        )
        .unwrap();
        let lines: Vec<_> = doc.lines.iter().map(Line::to_string).collect();
        assert_eq!(lines[1], "You · 1m ago");
        assert_eq!(lines[4], "· You closed the thread · 2m ago");
        assert_eq!(lines[6], "Atlas Builder · 3m ago");
        assert_eq!(lines[7], "↳ Can you check CI?");
        assert_eq!(lines[8], "CI passed.");
        assert!(lines.last().unwrap().contains("Reopen thread"));
    }

    #[test]
    fn folded_header_is_the_only_line_and_the_whole_row_toggles() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: true,
            message_count: 3,
            unread: true,
        };
        let doc = panel(&summary, &[], &State::default(), 80);
        assert_eq!(
            doc.lines[0].to_string(),
            "▸ Worker test · 3 messages · closed · unread"
        );
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.targets[0].width, 80);
        assert_eq!(doc.targets[0].hit, Tap::Toggle(summary.id.to_owned()));
        let narrow = panel(&summary, &[], &State::default(), 20);
        assert_eq!(narrow.lines[0].to_string(), "▸ Worker test");
    }

    #[test]
    fn hovering_only_adds_reply_to_the_message_header() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: false,
            message_count: 1,
            unread: false,
        };
        let records = [Record::Message {
            id: "message/1",
            sender: "Atlas Builder",
            age: "3m ago",
            text: "CI passed.",
            reply_to: None,
            from_person: false,
        }];
        let mut state = State {
            open: true,
            ..editable()
        };
        let rest = panel(&summary, &records, &state, 80);
        state.hovered_message = Some("message/1");
        let hovered = panel(&summary, &records, &state, 80);
        assert_eq!(rest.lines.len(), hovered.lines.len());
        assert_eq!(rest.lines[1].to_string(), "Atlas Builder · 3m ago");
        assert_eq!(
            hovered.lines[1].to_string(),
            "Atlas Builder · 3m ago  ↩ Reply"
        );
        assert_eq!(hovered.targets[1].line, 1);
        assert_eq!(hovered.messages[0].1, 2..3);
    }

    #[test]
    fn focused_composer_and_lifecycle_actions_have_separate_rows() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: false,
            message_count: 0,
            unread: false,
        };
        let doc = panel(
            &summary,
            &[],
            &State {
                open: true,
                cursor: Some(0),
                ..editable()
            },
            80,
        );
        assert_eq!(doc.lines[1].to_string(), "No messages yet.");
        assert_eq!(doc.lines[3].spans[0].style, theme::fg(theme::ACCENT));
        let lines: Vec<_> = doc.lines.iter().map(Line::to_string).collect();
        let controls = lines
            .iter()
            .position(|line| line.contains("Leave composer"))
            .unwrap();
        assert!(lines[controls].contains("Send"));
        assert!(lines[controls + 1].is_empty());
        assert!(lines[controls + 2].contains("Close thread"));
        assert!(lines[controls + 2].contains("Rename"));
        assert!(lines[controls + 2].contains("Open in tab"));
    }

    #[test]
    fn closed_thread_keeps_history_composer_and_quote_without_reopening() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: true,
            message_count: 1,
            unread: false,
        };
        let records = [
            Record::Closed {
                actor: "You",
                age: "3m ago",
            },
            Record::Message {
                id: "message/2",
                sender: "Atlas Builder",
                age: "2m ago",
                text: "CI passed.",
                reply_to: None,
                from_person: false,
            },
        ];
        let doc = panel(
            &summary,
            &records,
            &State {
                open: true,
                hovered_message: Some("message/2"),
                ..editable()
            },
            80,
        );
        assert_eq!(doc.lines[1].to_string(), "· You closed the thread · 3m ago");
        assert_eq!(doc.lines[4].to_string(), "CI passed.");
        assert!(
            doc.targets
                .iter()
                .any(|target| target.hit == Tap::Reopen(summary.id.to_owned()))
        );
        assert!(
            doc.targets
                .iter()
                .any(|target| matches!(target.hit, Tap::Compose(_)))
        );
        assert!(
            doc.targets
                .iter()
                .any(|target| matches!(target.hit, Tap::Reply { .. }))
        );
    }

    #[test]
    fn requested_open_thread_without_history_keeps_open_chevron_and_notice_under_heading() {
        let summary = Summary {
            id: "thread/1",
            title: "Sensor follow-up",
            agent: "Atlas Builder",
            closed: false,
            message_count: 0,
            unread: false,
        };
        let doc = panel(
            &summary,
            &[],
            &State {
                open: true,
                awaiting_history: true,
                notice: Some("Paused · collapse a thread to resume live updates"),
                ..editable()
            },
            80,
        );
        let lines: Vec<_> = doc.lines.iter().map(Line::to_string).collect();
        assert_eq!(
            lines,
            [
                "▾ Sensor follow-up · 0 messages",
                "Paused · collapse a thread to resume live updates",
            ],
            "the chevron follows the requested open state and the notice follows its own heading"
        );
        assert_eq!(doc.targets[0].hit, Tap::Toggle(summary.id.to_owned()));
    }

    #[test]
    fn selected_heading_has_a_glyph_gutter_and_full_row_background() {
        let line = heading("Worker test", " · 1 message", true, true, 40);
        let shown = line.to_string();
        assert!(
            shown.starts_with("▌▾ Worker test · 1 message"),
            "the gutter is a glyph, so it survives NO_COLOR: {shown:?}"
        );
        assert_eq!(text::width(&shown), 40, "the selected background spans the row");
        assert!(line.spans.iter().all(|span| span.style.bg == Some(theme::ROW_SELECTED)));
        assert_eq!(line.spans[0].style.fg, Some(theme::ACCENT));
        let plain = heading("Worker test", " · 1 message", true, false, 40);
        assert_eq!(plain.to_string(), "▾ Worker test · 1 message");
        assert!(plain.spans.iter().all(|span| span.style.bg.is_none()));
    }

    #[test]
    fn empty_standalone_thread_names_itself_and_says_why_it_cannot_be_answered() {
        let summary = Summary {
            id: "thread/1",
            title: "Sensor follow-up",
            agent: "Atlas Builder",
            closed: false,
            message_count: 0,
            unread: false,
        };
        let read_only = panel(
            &summary,
            &[],
            &State {
                open: true,
                standalone: true,
                ..State::default()
            },
            80,
        );
        let lines: Vec<_> = read_only.lines.iter().map(Line::to_string).collect();
        assert_eq!(
            lines[..5],
            [
                "Sensor follow-up · to Atlas Builder · open",
                "",
                "No messages yet.",
                "",
                READ_ONLY,
            ]
        );
        let writable = panel(
            &summary,
            &[],
            &State {
                open: true,
                standalone: true,
                ..editable()
            },
            80,
        );
        let lines: Vec<_> = writable.lines.iter().map(Line::to_string).collect();
        assert!(!lines.iter().any(|line| line == READ_ONLY));
        assert!(lines.iter().any(|line| line == "No messages yet."));
        let closed = panel(
            &Summary {
                closed: true,
                ..summary
            },
            &[],
            &State {
                open: true,
                standalone: true,
                ..State::default()
            },
            80,
        );
        assert_eq!(
            closed.lines[0].to_string(),
            "Sensor follow-up · to Atlas Builder · closed"
        );
    }

    #[test]
    fn thread_action_keycaps_are_lowercase_like_the_footer_and_help() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: false,
            message_count: 0,
            unread: true,
        };
        let doc = panel(
            &summary,
            &[],
            &State {
                open: true,
                can_mark_read: true,
                ..editable()
            },
            120,
        );
        let actions = doc.lines.last().unwrap().to_string();
        for keycap in [" z ", " n ", " p ", " v "] {
            assert!(actions.contains(keycap), "{keycap:?} in {actions:?}");
        }
        for keycap in [" Z ", " N ", " P ", " V "] {
            assert!(!actions.contains(keycap), "{keycap:?} in {actions:?}");
        }
    }

    #[test]
    fn thread_action_keycaps_take_alt_where_letters_type() {
        let summary = Summary {
            id: "thread/1",
            title: "Worker test",
            agent: "Atlas Builder",
            closed: false,
            message_count: 0,
            unread: true,
        };
        let doc = panel(
            &summary,
            &[],
            &State {
                open: true,
                can_mark_read: true,
                alt_keys: true,
                ..editable()
            },
            120,
        );
        let actions = doc.lines.last().unwrap().to_string();
        for keycap in [" alt+z ", " alt+n ", " alt+p ", " alt+v "] {
            assert!(actions.contains(keycap), "{keycap:?} in {actions:?}");
        }
        for keycap in [" z ", " n ", " p ", " v "] {
            assert!(!actions.contains(keycap), "{keycap:?} in {actions:?}");
        }
    }
}
