//! A triage pass is independent of navigation history and always opens in the focused split.

use super::{
    Ui,
    pane::Pane,
    view::{Attention, AttentionKind, Load},
};
use chrono::{DateTime, FixedOffset};
use std::cmp::Ordering;

/// The last visited attention item, not the pane that happens to have keyboard focus.
#[derive(Default)]
pub(super) struct Cursor {
    last: Option<Position>,
}

struct Position {
    class: u8,
    requested_at: Option<DateTime<FixedOffset>>,
    id: String,
}

/// Use question identity, not the availability of a client-side answer button. An exact
/// pending call can identify a native question on servers predating the question flag.
fn class(item: &Attention) -> u8 {
    match &item.kind {
        AttentionKind::Request { .. } => 0,
        AttentionKind::Prompt {
            question,
            call,
            calls,
            ..
        } if *question
            || call.as_deref().is_some_and(|id| {
                calls.as_ref().is_ok_and(|calls| {
                    calls.iter().any(|call| {
                        call.id == id
                            && matches!(
                                call.tool.as_str(),
                                "AskUserQuestion" | "request_user_input"
                            )
                    })
                })
            }) =>
        {
            0
        }
        AttentionKind::Review { .. }
        | AttentionKind::Feedback { .. }
        | AttentionKind::Launch { .. }
        | AttentionKind::Revision { .. } => 1,
        AttentionKind::Fault { .. } => 2,
        _ => 3,
    }
}

fn compare(
    left: (u8, Option<&DateTime<FixedOffset>>, &str),
    right: (u8, Option<&DateTime<FixedOffset>>, &str),
) -> Ordering {
    left.0
        .cmp(&right.0)
        // Unknown timestamps follow known ones; display-age strings never determine order.
        .then_with(|| match (left.1, right.1) {
            (Some(left), Some(right)) => left.cmp(right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| left.2.cmp(right.2))
}

impl Ui {
    /// Visit the next (or previous) outstanding Inbox item. The boundary is a distinct stop:
    /// leave the last card visible, announce all clear, then wrap on the following command.
    pub(super) fn triage_next(&mut self, forward: bool) {
        let items = match &self.world.attention {
            Load::Ready(items) => items,
            Load::Loading => {
                self.flash("loading attention…");
                return;
            }
            Load::Failed(_) => {
                self.flash("attention unavailable");
                return;
            }
        };
        let mut items: Vec<_> = items
            .iter()
            .filter(|item| !self.closed.contains(&item.id) && !self.snoozed.contains(&item.id))
            .map(|item| (class(item), item))
            .collect();
        items.sort_by(|(left_class, left), (right_class, right)| {
            compare(
                (*left_class, left.requested_at.as_ref(), &left.id),
                (*right_class, right.requested_at.as_ref(), &right.id),
            )
        });
        let next = match &self.triage.last {
            None => {
                if forward {
                    (!items.is_empty()).then_some(0)
                } else {
                    items.len().checked_sub(1)
                }
            }
            Some(last) => {
                if let Some(index) = items.iter().position(|(_, item)| item.id == last.id) {
                    if forward {
                        index.checked_add(1).filter(|index| *index < items.len())
                    } else {
                        index.checked_sub(1)
                    }
                } else {
                    // Resolution, closure or snoozing must not restart the pass. Keep moving
                    // beyond the removed item's last position in the current priority order.
                    let beyond = |(class, item): &(u8, &Attention)| {
                        let order = compare(
                            (*class, item.requested_at.as_ref(), &item.id),
                            (last.class, last.requested_at.as_ref(), &last.id),
                        );
                        if forward {
                            order.is_gt()
                        } else {
                            order.is_lt()
                        }
                    };
                    if forward {
                        items.iter().position(beyond)
                    } else {
                        items.iter().rposition(beyond)
                    }
                }
            }
        };
        let Some(index) = next else {
            self.triage.last = None;
            self.flash("all clear");
            return;
        };
        let (class, item) = items[index];
        let reason = match class {
            0 => "agent question",
            1 => "decision",
            2 => "failure",
            _ => item.kind.word(),
        };
        let message = format!("{}/{} · {reason}", index + 1, items.len());
        let id = item.id.clone();
        self.triage.last = Some(Position {
            class,
            requested_at: item.requested_at,
            id: id.clone(),
        });
        self.replace_focused_pane(Pane::Home(Some(id)));
        self.flash(message);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        demo,
        glass::Open,
        view::{Load, PendingCall},
    };
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn item(id: &str, kind: AttentionKind, at: &str) -> Attention {
        let mut item = demo::world().attention.items()[0].clone();
        item.id = format!("attention/{id}");
        item.title = id.to_owned();
        item.kind = kind;
        item.requested_at = Some(DateTime::parse_from_rfc3339(at).unwrap());
        // Identical rounded display ages must not turn chronological order into ID order.
        item.age = "1h".into();
        item
    }

    fn question() -> AttentionKind {
        AttentionKind::Request {
            from: "Builder".into(),
            from_id: "agent/example/builder".into(),
            question: "Which option?".into(),
            structured: None,
            images: Vec::new(),
        }
    }

    fn decision() -> AttentionKind {
        AttentionKind::Revision {
            reason: "Change the plan?".into(),
            changes: Vec::new(),
        }
    }

    fn failure() -> AttentionKind {
        AttentionKind::Fault {
            what: "Build failed".into(),
            because: "Compiler error".into(),
            fix: None,
            source: "builder".into(),
        }
    }

    fn update() -> AttentionKind {
        AttentionKind::Update {
            from: "Builder".into(),
            body: "Finished".into(),
            about: "message/example".into(),
            subjects: Vec::new(),
        }
    }

    fn ui(items: Vec<Attention>) -> Ui {
        let mut world = demo::world();
        world.attention = Load::Ready(items);
        let mut ui = Ui::new(world);
        ui.glasses.sidebar.shown = false;
        ui
    }

    fn focused(ui: &Ui, id: &str, message: &str) {
        assert_eq!(
            ui.focused_pane(),
            Some(Pane::Home(Some(format!("attention/{id}"))))
        );
        assert_eq!(
            ui.flash.as_ref().map(|(message, _)| message.as_str()),
            Some(message)
        );
    }

    #[test]
    fn priority_then_exact_oldest_first_with_inbox_updates_last() {
        let mut ui = ui(vec![
            item("update", update(), "2026-10-01T00:00:00Z"),
            item("failure", failure(), "2026-10-02T00:00:00Z"),
            item("decision", decision(), "2026-10-03T00:00:00Z"),
            item("a-newer", question(), "2026-10-04T10:02:00Z"),
            item("z-older", question(), "2026-10-04T11:01:00+01:00"),
        ]);
        for (id, message) in [
            ("z-older", "1/5 · agent question"),
            ("a-newer", "2/5 · agent question"),
            ("decision", "3/5 · decision"),
            ("failure", "4/5 · failure"),
            ("update", "5/5 · update"),
        ] {
            ui.triage_next(true);
            focused(&ui, id, message);
        }
        ui.triage_next(true);
        focused(&ui, "update", "all clear");
        ui.triage_next(true);
        focused(&ui, "z-older", "1/5 · agent question");
    }

    #[test]
    fn backward_pass_stops_before_wrapping_and_can_change_direction() {
        let mut ui = ui(vec![
            item("first", question(), "2026-10-01T00:00:00Z"),
            item("last", failure(), "2026-10-01T00:00:00Z"),
        ]);
        ui.triage_next(false);
        focused(&ui, "last", "2/2 · failure");
        ui.triage_next(false);
        focused(&ui, "first", "1/2 · agent question");
        ui.triage_next(true);
        focused(&ui, "last", "2/2 · failure");
        ui.triage_next(false);
        ui.triage_next(false);
        focused(&ui, "first", "all clear");
        ui.triage_next(false);
        focused(&ui, "last", "2/2 · failure");
    }

    #[test]
    fn closed_and_snoozed_items_are_excluded_and_empty_is_all_clear() {
        let mut ui = ui(vec![
            item("closed", question(), "2026-10-01T00:00:00Z"),
            item("snoozed", decision(), "2026-10-01T00:00:00Z"),
            item("open", failure(), "2026-10-01T00:00:00Z"),
        ]);
        ui.closed.insert("attention/closed".into());
        ui.snoozed.insert("attention/snoozed".into());
        ui.triage_next(true);
        focused(&ui, "open", "1/1 · failure");
        ui.closed.insert("attention/open".into());
        ui.triage_next(true);
        focused(&ui, "open", "all clear");
        ui.triage_next(false);
        focused(&ui, "open", "all clear");
    }

    #[test]
    fn resolving_current_item_continues_past_it_and_other_navigation_does_not_reset_cursor() {
        let mut ui = ui(vec![
            item("first", question(), "2026-10-01T00:00:00Z"),
            item("second", question(), "2026-10-02T00:00:00Z"),
            item("third", failure(), "2026-10-01T00:00:00Z"),
        ]);
        ui.triage_next(true);
        ui.open_in_glass(Pane::List(1), Open::Tab);
        ui.triage_next(true);
        focused(&ui, "second", "2/3 · agent question");
        if let Load::Ready(items) = &mut ui.world.attention {
            items.retain(|item| item.id != "attention/second");
        }
        ui.triage_next(true);
        focused(&ui, "third", "2/2 · failure");
    }

    #[test]
    fn replaces_focused_tab_even_when_target_is_already_in_another_split() {
        let mut ui = ui(vec![item("next", question(), "2026-10-01T00:00:00Z")]);
        ui.open_in_glass(Pane::List(1), Open::Tab);
        ui.open_in_glass(Pane::Home(Some("attention/next".into())), Open::Right);
        ui.focus_group(0);
        let split = ui.focused_split();
        ui.triage_next(true);
        assert_eq!(ui.focused_split(), split);
        focused(&ui, "next", "1/1 · agent question");
        // Tab zero was replaced, not a new tab appended after the old list.
        ui.show_in(0, 0);
        focused(&ui, "next", "1/1 · agent question");
        ui.focus_group(1);
        focused(&ui, "next", "1/1 · agent question");
        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let shown: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(shown.contains("1/1 · agent question"), "{shown}");
    }

    #[test]
    fn native_question_identity_outlives_answer_eligibility() {
        let mut native = item("native", question(), "2026-10-01T00:00:00Z");
        native.kind = AttentionKind::Prompt {
            seat: "Builder".into(),
            seat_id: "agent/example/builder".into(),
            text: "Choose".into(),
            question: true,
            answers: Vec::new(),
            episode: "episode/question".into(),
            call: None,
            calls: Err("expired".into()),
            allow: false,
        };
        let mut ui = ui(vec![
            item("decision", decision(), "2026-10-01T00:00:00Z"),
            native,
        ]);
        ui.triage_next(true);
        focused(&ui, "native", "1/2 · agent question");
    }

    #[test]
    fn only_the_exact_named_pending_question_call_establishes_question_priority() {
        let mut native = item("permission", question(), "2026-10-01T00:00:00Z");
        native.kind = AttentionKind::Prompt {
            seat: "Builder".into(),
            seat_id: "agent/example/builder".into(),
            text: "Allow?".into(),
            question: false,
            answers: vec!["allow".into()],
            episode: "episode/permission".into(),
            call: Some("permission".into()),
            calls: Ok(vec![PendingCall {
                id: "question".into(),
                tool: "AskUserQuestion".into(),
                input: serde_json::json!({}),
                complete: true,
            }]),
            allow: true,
        };
        let mut ui = ui(vec![
            native,
            item("decision", decision(), "2026-10-01T00:00:00Z"),
        ]);
        ui.triage_next(true);
        focused(&ui, "decision", "1/2 · decision");
        if let Load::Ready(items) = &mut ui.world.attention
            && let AttentionKind::Prompt { call, .. } = &mut items[0].kind
        {
            *call = Some("question".into());
        }
        // Starting a new pass picks up the changed native question classification.
        ui.triage = Cursor::default();
        ui.triage_next(true);
        focused(&ui, "permission", "1/2 · agent question");
    }

    #[test]
    fn triage_keymap_separates_legacy_enhanced_and_attached_input() {
        use super::super::{Effect, doc::Hit};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for enhanced in [false, true] {
            let mut ui = ui(vec![
                item("first", question(), "2026-10-01T00:00:00Z"),
                item("last", update(), "2026-10-02T00:00:00Z"),
            ]);
            ui.keyboard_enhanced = enhanced;
            ui.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
            focused(&ui, "first", "1/2 · agent question");
            ui.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
            focused(&ui, "last", "2/2 · update");
            if enhanced {
                ui.key(KeyEvent::new(
                    KeyCode::Char('L'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ));
                focused(&ui, "first", "1/2 · agent question");
            }
            let agent = ui.world.agents.items()[0].id.clone();
            ui.open_in_glass(Pane::Agent(Some(agent)), Open::Tab);
            ui.click(Hit::Attach);
            ui.effects.clear();
            let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
            ui.key(key);
            assert!(matches!(ui.effects.last(), Some(Effect::TerminalKey(sent)) if *sent == key));
            ui.key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
            ui.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
            if enhanced {
                assert_eq!(
                    ui.flash.as_ref().map(|(message, _)| message.as_str()),
                    Some("all clear")
                );
                ui.key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
                ui.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
            }
            assert!(matches!(ui.focused_pane(), Some(Pane::Home(_))));
        }
    }

    #[test]
    fn replaced_content_keeps_its_tab_identity_for_back_and_forward() {
        let mut ui = ui(vec![item("first", question(), "2026-10-01T00:00:00Z")]);
        let prior = Pane::Document("doc/triage-before".into());
        ui.open_in_glass(prior.clone(), Open::Tab);
        let tab_id = ui.current_place().unwrap().tab_id;
        ui.triage_next(true);
        assert_eq!(ui.current_place().unwrap().tab_id, tab_id);
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(prior));
        ui.navigate_history(true);
        assert_eq!(
            ui.focused_pane(),
            Some(Pane::Home(Some("attention/first".into())))
        );
    }
    fn unavailable_attention_keeps_the_current_place(
        attention: Load<Vec<Attention>>,
        message: &str,
    ) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut ui = ui(vec![
            item("first", question(), "2026-10-01T00:00:00Z"),
            item("last", update(), "2026-10-02T00:00:00Z"),
        ]);
        ui.triage_next(true);
        let before = ui.current_place();
        let ready = std::mem::replace(&mut ui.world.attention, attention);
        ui.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(ui.current_place(), before);
        assert_eq!(
            ui.flash.as_ref().map(|(message, _)| message.as_str()),
            Some(message)
        );
        ui.world.attention = ready;
        ui.triage_next(true);
        focused(&ui, "last", "2/2 · update");
    }

    #[test]
    fn loading_attention_is_not_reported_as_all_clear() {
        unavailable_attention_keeps_the_current_place(Load::Loading, "loading attention…");
    }

    #[test]
    fn failed_attention_is_not_reported_as_all_clear() {
        unavailable_attention_keeps_the_current_place(
            Load::Failed("initial fetch failed".into()),
            "attention unavailable",
        );
    }
}
