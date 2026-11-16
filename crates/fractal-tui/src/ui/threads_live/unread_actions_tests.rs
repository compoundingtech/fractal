//! Isolated client-boundary fixtures mounted through the production TUI renderer.
use super::*;
use super::super::{Ui, glass::Open};
use crossterm::event::Event;
use ratatui::{Terminal, backend::TestBackend};

const ID: &str = "thread/unread-test";
const HOST: &str = "thread:thread/unread-test";
const AGENT: &str = "agent/example/atlas/builder";

fn event(id: &str, content: &str) -> ThreadTimelineEvent {
    serde_json::from_value(serde_json::json!({
        "id": id, "actor_id": AGENT, "accepted_at_unix_ms": 1_000,
        "kind": "message", "content": content, "thread_quote_id": null
    })).unwrap()
}

fn fixture(unread: bool, marks: &[(&str, &str)]) -> Ui {
    let mut ui = Ui::new(super::super::demo::test_world());
    ui.open_in_glass(Pane::Thread(ID.into()), Open::Tab);
    ui.threads.enable(true);
    ui.threads.set_access(ThreadAccess { mark_read: true, ..ThreadAccess::default() });
    let reminder = marks.first().map(|(_, anchor)| serde_json::json!({
        "anchor_message_id": anchor,
        "marks": marks.iter().map(|(mark_id, anchor_message_id)| serde_json::json!({
            "mark_id": mark_id, "anchor_message_id": anchor_message_id
        })).collect::<Vec<_>>()
    }));
    ui.threads.put_summary(serde_json::from_value(serde_json::json!({
        "api_version": fractal_client::API_VERSION, "request_id": "request/test",
        "snapshot": { "id": "snapshot/test", "host_id": "host/test", "store_index": 1,
            "projection_version": "1", "created_at": "2026-11-16T00:00:00Z" },
        "value": { "id": ID, "agent_id": AGENT, "title": "Unread test",
            "revision": "claim/current", "title_revision": "claim/title", "state": "open",
            "message_count": 1, "unread": unread, "unread_reminder": reminder }
    })).unwrap());
    ui.threads.histories.insert(ID.into(), History {
        head: vec![event("message/visible", "Meaningful reply content\n\nSecond meaningful line")],
        ..History::default()
    });
    ui
}

fn present_at(ui: &mut Ui, now: std::time::Instant) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui.render(frame)).unwrap();
    let body = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
    ui.read_presented_threads_at(now);
    body
}

fn present(ui: &mut Ui) -> String {
    let now = std::time::Instant::now();
    present_at(ui, now);
    present_at(ui, now + std::time::Duration::from_millis(500))
}

#[test]
fn unread_auto_read_requires_foreground_presented_meaningful_content() {
    let mut ui = fixture(true, &[]);
    assert!(ui.threads.writes().is_empty(), "loading/listing never consumes a message");
    ui.input_event(Event::FocusLost);
    assert!(present(&mut ui).contains("Meaningful reply content"));
    assert!(ui.threads.writes().is_empty(), "unfocused terminal is not a reader");
    ui.input_event(Event::FocusGained);
    present(&mut ui);
    let writes = ui.threads.writes();
    assert_eq!(writes.len(), 1);
    let Mutation::MarkRead(read) = &writes[0].mutation else { panic!("expected thread.read") };
    assert_eq!(read.seen_message_ids, ["message/visible"]);

    let mut empty = fixture(true, &[]);
    empty.threads.histories.get_mut(ID).unwrap().head = vec![event("message/blank", "  \n ")];
    present(&mut empty);
    assert!(empty.threads.writes().is_empty(), "whitespace is not meaningful consumption");
}

#[test]
fn unread_background_tab_and_list_hover_do_not_consume_messages() {
    let mut ui = fixture(true, &[]);
    ui.open_in_glass(Pane::Agent(Some("agent/example/atlas/builder".into())), Open::Tab);
    ui.threads.hover(Some("message/visible"));
    present(&mut ui);
    assert!(ui.threads.writes().is_empty());
}

#[test]
fn unread_shortcut_captures_stable_marks_and_preserves_natural_read_history() {
    let mark = smallclaims::store::operation_id_for_key("unread/old");
    let mut ui = fixture(false, &[(mark.as_str(), "message/older")]);
    // Avoid requesting an around page in this dispatch-only fixture.
    ui.threads.reading_host = Some(HOST.into());
    let reminder = ui.threads.summary(ID).unwrap().unread_reminder.as_ref().unwrap();
    let mut visit = super::super::unread_visit::Visit::default();
    visit.begin(ID, Some(reminder));
    ui.threads.visits.insert(ID.into(), visit);
    present(&mut ui);
    ui.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
    let writes = ui.threads.writes();
    assert_eq!(writes.len(), 1);
    let Mutation::MarkUnread(marked) = &writes[0].mutation else { panic!("expected reminder action") };
    assert_eq!(marked.anchor_message_id, "message/visible");
    assert_eq!(marked.observed_marks, [mark]);
    assert_eq!(ui.threads.summary(ID).unwrap().unread, Some(false));
    assert!(!writes.iter().any(|write| matches!(write.mutation, Mutation::MarkRead(_))));
}

#[test]
fn unread_three_marks_clear_only_the_captured_marks_at_their_own_anchors() {
    let one = smallclaims::store::operation_id_for_key("unread/one");
    let two = smallclaims::store::operation_id_for_key("unread/two");
    let three = smallclaims::store::operation_id_for_key("unread/three");
    let mut ui = fixture(false, &[
        (&one, "message/visible"), (&two, "message/second"), (&three, "message/third")
    ]);
    present(&mut ui);
    let writes = ui.threads.writes();
    assert_eq!(writes.len(), 1);
    let Mutation::ClearUnread(clear) = &writes[0].mutation else { panic!("expected reminder clear") };
    assert_eq!(clear.observed_marks, [one]);
    assert_eq!(ui.threads.summary(ID).unwrap().unread_reminder.as_ref().unwrap().marks.len(), 3,
        "no optimistic mutation of authoritative reminder state");
}

#[test]
fn unread_mark_is_sticky_until_a_later_visit() {
    let mark = smallclaims::store::operation_id_for_key("unread/sticky");
    let mut ui = fixture(false, &[]);
    present(&mut ui);
    ui.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
    let write = ui.threads.writes().pop().unwrap();
    let mut accepted = ui.threads.summaries[ID].clone();
    accepted.value.unread_reminder = Some(fractal_client::ThreadUnreadReminder {
        marks: vec![fractal_client::ThreadUnreadMark {
            mark_id: mark.clone(), anchor_message_id: "message/visible".into()
        }], anchor_message_id: "message/visible".into()
    });
    ui.threads.complete(Receipt { write, confirmation: Confirmation::Applied(Some(accepted)) }, &mut HashMap::new());
    present(&mut ui);
    assert!(ui.threads.writes().is_empty(), "marking unread remains sticky in this visit");
    ui.threads.leave_view();
    present(&mut ui);
    let write = ui.threads.writes().pop().unwrap();
    let Mutation::ClearUnread(clear) = write.mutation else { panic!("later visit clears reminder") };
    assert_eq!(clear.observed_marks, [mark]);
}

#[test]
fn unread_around_refusal_is_neutral_and_withholds_body_and_badge() {
    let mark = smallclaims::store::operation_id_for_key("unread/unresolved");
    let mut ui = fixture(true, &[(mark.as_str(), "message/not-yet-known")]);
    present(&mut ui);
    let seek = ui.threads.take_seeks().pop().unwrap();
    assert!(ui.threads.seek_loaded(ID, &seek.key, Err(SeekFailure::Unavailable)).is_none());
    let rendered = present(&mut ui);
    assert!(rendered.contains(thread::ANCHOR_UNAVAILABLE));
    assert!(!rendered.contains("Meaningful reply content"));
    assert!(!rendered.contains("unread"));
    assert!(ui.threads.writes().is_empty());
}

#[test]
fn unread_pending_seek_never_exposes_a_body_or_activates_a_badge() {
    let mark = smallclaims::store::operation_id_for_key("unread/pending");
    let mut ui = fixture(true, &[(mark.as_str(), "message/pending")]);
    present(&mut ui);
    let seek = ui.threads.take_seeks().pop().unwrap();
    let page = serde_json::from_value(serde_json::json!({
        "items": [event("message/visible", "Withheld body")], "next_cursor": null,
        "previous_cursor": null, "anchor": { "message_id": "message/pending", "state": "pending" }
    })).unwrap();
    assert!(ui.threads.seek_loaded(ID, &seek.key, Ok(page)).is_none());
    let rendered = present(&mut ui);
    assert!(rendered.contains(thread::ANCHOR_PENDING));
    assert!(!rendered.contains("Withheld body"));
    assert!(!rendered.contains("unread"));
    assert!(ui.threads.writes().is_empty());
}

#[test]
fn unread_read_only_grant_hides_and_refuses_both_mark_shortcuts() {
    let mut ui = fixture(true, &[]);
    ui.threads.set_access(ThreadAccess::default());
    let rendered = present(&mut ui);
    assert!(!rendered.contains("Mark read"));
    assert!(!rendered.contains("Mark unread"));
    ui.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
    ui.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert!(ui.threads.writes().is_empty());
}

#[test]
fn unread_auto_read_requires_500ms_and_scroll_past_resets_the_clock() {
    let mut ui = fixture(true, &[]);
    let now = std::time::Instant::now();
    present_at(&mut ui, now);
    assert!(ui.threads.writes().is_empty());
    present_at(&mut ui, now + std::time::Duration::from_millis(499));
    assert!(ui.threads.writes().is_empty());
    // No qualifying body at this presentation breaks continuous visibility.
    ui.threads.presented(HOST, &[(ID.into(), HashSet::new())].into(),
        now + std::time::Duration::from_millis(499));
    present_at(&mut ui, now + std::time::Duration::from_millis(500));
    assert!(ui.threads.writes().is_empty());
    present_at(&mut ui, now + std::time::Duration::from_millis(999));
    assert!(ui.threads.writes().is_empty());
    present_at(&mut ui, now + std::time::Duration::from_millis(1000));
    assert_eq!(ui.threads.writes().len(), 1);
}

#[test]
fn unread_unknown_entry_and_placement_changes_do_not_recapture_marks() {
    let mark = smallclaims::store::operation_id_for_key("unread/late-summary");
    let mut ui = fixture(false, &[(mark.as_str(), "message/visible")]);
    let summary = ui.threads.summaries.remove(ID).unwrap();
    let now = std::time::Instant::now();
    let shown = [(ID.into(), ["message/visible".into()].into())].into();
    ui.threads.presented("home:attention/item", &shown, now);
    ui.threads.put_summary(summary);
    ui.threads.presented("chat:agent/example/atlas/builder", &shown, now);
    ui.threads.presented(HOST, &shown, now + std::time::Duration::from_millis(500));
    ui.threads.presented(HOST, &shown, now + std::time::Duration::from_millis(1000));
    assert!(ui.threads.writes().is_empty(), "unknown-at-entry remains sticky across placements");
    ui.threads.presented("mission:other", &BTreeMap::new(), now + std::time::Duration::from_millis(1001));
    ui.threads.presented(HOST, &shown, now + std::time::Duration::from_millis(1002));
    ui.threads.presented(HOST, &shown, now + std::time::Duration::from_millis(1502));
    let write = ui.threads.writes().pop().unwrap();
    let Mutation::ClearUnread(clear) = write.mutation else { panic!("later visit clears") };
    assert_eq!(clear.observed_marks, [mark]);
}

#[test]
fn unread_action_menu_keeps_pre_palette_visible_anchor_evidence() {
    let mut ui = fixture(false, &[]);
    present(&mut ui);
    ui.open_palette(None, Open::Here);
    ui.paste("Mark unread".into());
    // Painting the menu may cover the body, but explicit action evidence was captured on opening.
    present(&mut ui);
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let write = ui.threads.writes().pop().expect("menu dispatches Mark unread");
    let Mutation::MarkUnread(mark) = write.mutation else { panic!("expected reminder action") };
    assert_eq!(mark.anchor_message_id, "message/visible");
}

#[test]
fn unread_marked_row_is_a_private_reminder_and_opening_it_seeks_the_anchor() {
    let mark = smallclaims::store::operation_id_for_key("unread/return");
    let mut ui = fixture(false, &[(mark.as_str(), "message/anchor")]);
    let generation = ui.threads.marked_due(&Pane::Home(None)).unwrap();
    ui.threads.marked_loaded(generation, Ok(vec![ui.threads.summaries[ID].clone()]));
    let mut list = super::super::screens::Listing {
        items: Vec::new(), ids: Vec::new(), state: super::super::screens::ListState::Ready, legend: Vec::new(),
    };
    ui.threads.marked_unread(&mut list);
    assert_eq!(list.ids, [ID]);
    let marker = list.items.iter().find_map(|item| match item {
        super::super::screens::Item::Row { right, .. } => Some(right.iter().map(|span| span.content.as_ref()).collect::<String>()),
        _ => None,
    }).unwrap();
    assert!(marker.contains("reminder"));
    assert!(!marker.contains("unread"));
    assert_eq!(ui.threads.summary(ID).unwrap().unread, Some(false));
    ui.open_in_glass(Pane::Home(Some(ID.into())), Open::Tab);
    present(&mut ui);
    let seek = ui.threads.take_seeks().pop().expect("marked row requests around");
    assert_eq!(seek.anchor, "message/anchor");
    let page = serde_json::from_value(serde_json::json!({
        "items": [event("message/anchor", "Return anchor body\n\nTrailing body line")],
        "next_cursor": null, "previous_cursor": null,
        "anchor": { "message_id": "message/anchor", "state": "current" }
    })).unwrap();
    ui.threads.seek_loaded(ID, &seek.key, Ok(page));
    let rendered = present(&mut ui);
    assert!(rendered.contains("Return anchor body"));
    let write = ui.threads.writes().pop().expect("viewed return anchor clears captured mark");
    let Mutation::ClearUnread(clear) = write.mutation else { panic!("expected clear") };
    assert_eq!(clear.observed_marks, [mark]);
}
