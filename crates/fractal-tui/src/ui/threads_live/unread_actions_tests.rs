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
    ui.input_event(Event::FocusGained);
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
    ui.threads.presented(HOST, &[(ID.into(), BTreeMap::new())].into(),
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
    let shown = [(ID.into(), [("message/visible".into(), super::super::BodyGeometry::default())].into())].into();
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
fn unread_action_menu_marks_only_presented_replies_read_before_auto_read_dwell() {
    let mut ui = fixture(true, &[]);
    assert!(present_at(&mut ui, std::time::Instant::now()).contains("Meaningful reply content"));
    // A reply can load after the last frame without having been presented to the reader.
    ui.threads.histories.get_mut(ID).unwrap().head.push(event(
        "message/unpresented", "Loaded but not presented",
    ));
    assert!(ui.threads.writes().is_empty(), "one frame cannot satisfy the auto-read dwell");
    ui.open_palette(None, Open::Here);
    ui.paste("Mark read".into());
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let writes = ui.threads.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].id, ID);
    assert_eq!(writes[0].host, HOST);
    let Mutation::MarkRead(read) = &writes[0].mutation else {
        panic!("palette must mark naturally unread replies read")
    };
    assert_eq!(read.seen_message_ids, ["message/visible"]);
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

fn inbox_detail_survives_dwell_clear_and_marked_refetch(compose_before_dwell: bool) {
    const OTHER: &str = "thread/other-marked";
    const DRAFT: &str = "Unsent reply stays here";
    let mark = smallclaims::store::operation_id_for_key("unread/inbox-context");
    let mut ui = fixture(false, &[(mark.as_str(), "message/visible")]);
    ui.threads.set_access(ThreadAccess {
        mark_read: true, reply: true, ..ThreadAccess::default()
    });
    let mut other = ui.threads.summaries[ID].clone();
    other.value.id = OTHER.into();
    other.value.title = "Other marked thread".into();
    let generation = ui.threads.marked_due(&Pane::Home(None)).unwrap();
    ui.threads.marked_loaded(generation, Ok(vec![ui.threads.summaries[ID].clone(), other.clone()]));
    ui.open_home();
    let index = ui.listing_for(0, 40).ids.iter().position(|id| id == ID).unwrap();
    ui.click(super::super::doc::Hit::Row(index));
    assert_eq!(ui.focused_pane(), Some(Pane::List(0)), "exercise Inbox, not an independent thread tab");
    let host = Pane::Home(Some(ID.into())).key();
    if compose_before_dwell {
        ui.click(super::super::doc::Hit::Thread(host.clone(), thread::Tap::Compose(ID.into())));
        ui.paste(DRAFT.into());
    }
    let now = std::time::Instant::now();
    assert!(present_at(&mut ui, now).contains("Meaningful reply content"));
    assert!(ui.threads.writes().is_empty(), "opening alone does not clear the reminder");
    present_at(&mut ui, now + std::time::Duration::from_millis(500));
    let write = ui.threads.writes().pop().expect("500 ms dwell clears the reminder");
    let Mutation::ClearUnread(clear) = &write.mutation else { panic!("expected clear") };
    assert_eq!(clear.observed_marks, [mark]);
    if !compose_before_dwell {
        ui.click(super::super::doc::Hit::Thread(host.clone(), thread::Tap::Compose(ID.into())));
        ui.paste(DRAFT.into());
    }
    let mut accepted = ui.threads.summaries[ID].clone();
    accepted.value.unread_reminder = None;
    ui.threads.complete(Receipt { write, confirmation: Confirmation::Applied(Some(accepted)) },
        &mut ui.conversation_state.drafts);
    assert_eq!(ui.selected_id().as_deref(), Some(ID), "loading marked rows must not change the keyboard target");
    assert_eq!(ui.main_pane(), Pane::Home(Some(ID.into())));
    let pane = ui.main_pane();
    let generation = ui.threads.marked_due(&pane).expect("clear invalidates marked list");
    ui.threads.marked_loaded(generation, Ok(vec![other]));
    assert!(!ui.listing_for(0, 40).ids.iter().any(|id| id == ID));
    assert!(ui.listing_for(0, 40).ids.iter().any(|id| id == OTHER));
    let rendered = present_at(&mut ui, now + std::time::Duration::from_millis(501));
    assert_eq!(ui.main_pane(), Pane::Home(Some(ID.into())), "refetch must not switch the detail by index");
    assert!(rendered.contains("Meaningful reply content"));
    assert!(rendered.contains(DRAFT));
    assert!(ui.threads.summary(ID).unwrap().unread_reminder.is_none());
    assert!(ui.threads.focused.as_ref().is_some_and(|(place, input)|
        place == &host && matches!(input, Input::Reply(id) if id == ID)));
    assert_eq!(ui.conversation_state.drafts.get(&Input::Reply(ID.into()).key()).map(String::as_str), Some(DRAFT));
    let index = ui.listing_for(0, 40).ids.iter().position(|id| id == OTHER).unwrap();
    ui.click(super::super::doc::Hit::Row(index));
    assert_eq!(ui.main_pane(), Pane::Home(Some(OTHER.into())));
    assert!(!present_at(&mut ui, now + std::time::Duration::from_millis(502)).contains("Meaningful reply content"));
}

#[test]
fn unread_inbox_detail_survives_dwell_clear_and_marked_refetch() {
    inbox_detail_survives_dwell_clear_and_marked_refetch(true);
}

#[test]
fn unread_inbox_detail_retains_draft_started_while_clear_is_in_flight() {
    inbox_detail_survives_dwell_clear_and_marked_refetch(false);
}

#[test]
fn unread_read_host_matches_rendered_detail_not_list_only_tabs() {
    for pane in [Pane::List(0), Pane::List(1), Pane::List(2),
        Pane::Home(Some(ID.into())), Pane::Thread(ID.into())] {
        let mut ui = fixture(true, &[]);
        ui.open_in_glass(pane.clone(), Open::Tab);
        if pane == Pane::List(0) {
            ui.threads.select_inbox_detail(Some(ID.into()));
        }
        let drawn = ui.main_pane();
        let now = std::time::Instant::now();
        let rendered = present_at(&mut ui, now);
        assert_eq!(ui.threads.reading_host.as_deref(), Some(drawn.key().as_str()));
        present_at(&mut ui, now + std::time::Duration::from_millis(499));
        assert!(ui.threads.writes().is_empty(), "{pane:?}: no early read");
        present_at(&mut ui, now + std::time::Duration::from_millis(500));
        let writes = ui.threads.writes();
        if matches!(pane, Pane::List(1 | 2)) {
            assert!(!rendered.contains("Meaningful reply content"));
            assert!(writes.is_empty(), "{pane:?}: a list does not embed its hovered agent/mission");
        } else {
            assert!(rendered.contains("Meaningful reply content"));
            assert_eq!(writes.len(), 1, "{pane:?}: the drawn detail supplies read evidence");
            assert!(matches!(&writes[0].mutation, Mutation::MarkRead(read)
                if read.seen_message_ids == ["message/visible"]));
        }
    }
}

#[test]
fn unread_inbox_detail_releases_on_collapse_disable_and_read_refusal() {
    for release in ["collapse", "disable", "read-refusal"] {
        let mut ui = fixture(false, &[]);
        ui.open_home();
        ui.threads.select_inbox_detail(Some(ID.into()));
        assert_eq!(ui.main_pane(), Pane::Home(Some(ID.into())));
        match release {
            "collapse" => ui.click(super::super::doc::Hit::Thread(
                Pane::Home(Some(ID.into())).key(), thread::Tap::Toggle(ID.into()))),
            "disable" => ui.threads.enable(false),
            "read-refusal" => ui.threads.failure(
                ThreadInterest::Timeline(ID.into()), Some(fractal_client::ErrorCode::Forbidden),
                "Read permission revoked".into(), true),
            _ => unreachable!(),
        }
        assert!(ui.threads.inbox_detail().is_none(), "{release}: release the identity pin");
        assert_ne!(ui.main_pane(), Pane::Home(Some(ID.into())), "{release}: stop mounting the pinned detail");
    }
}

#[test]
fn unread_stable_reading_rule_is_shared_by_all_thread_placements() {
    for host in [HOST, "home:attention/item", "chat:agent/example/atlas/builder", "mission:mission/test"] {
        let mut ui = fixture(true, &[]);
        let now = std::time::Instant::now();
        let shown = [(ID.into(), [("message/visible".into(), super::super::BodyGeometry::default())].into())].into();
        ui.threads.presented(host, &shown, now);
        assert!(ui.threads.writes().is_empty(), "{host}: initial frame is not a read");
        ui.threads.presented(host, &shown, now + std::time::Duration::from_millis(499));
        assert!(ui.threads.writes().is_empty(), "{host}: dwell is incomplete");
        ui.threads.presented(host, &shown, now + std::time::Duration::from_millis(500));
        let write = ui.threads.writes().pop().expect("each placement uses the same dwell rule");
        let Mutation::MarkRead(read) = write.mutation else { panic!("expected natural read") };
        assert_eq!(read.seen_message_ids, ["message/visible"]);
    }
}

#[test]
fn unread_unknown_foreground_never_automatically_consumes_content() {
    let mut ui = fixture(true, &[]);
    ui.terminal_foreground = None;
    present(&mut ui);
    assert!(ui.threads.writes().is_empty());
    ui.input_event(Event::FocusGained);
    present(&mut ui);
    assert_eq!(ui.threads.writes().len(), 1);
}

#[test]
fn unread_fully_visible_single_line_qualifies_after_the_dwell() {
    let mut ui = fixture(true, &[]);
    ui.threads.histories.get_mut(ID).unwrap().head =
        vec![event("message/visible", "One meaningful line")];
    present(&mut ui);
    assert_eq!(ui.threads.writes().len(), 1);
}

#[test]
fn unread_body_change_with_same_message_id_restarts_the_dwell() {
    let mut ui = fixture(true, &[]);
    let now = std::time::Instant::now();
    present_at(&mut ui, now);
    ui.threads.histories.get_mut(ID).unwrap().head =
        vec![event("message/visible", "Changed content\n\nSecond meaningful line")];
    present_at(&mut ui, now + std::time::Duration::from_millis(499));
    present_at(&mut ui, now + std::time::Duration::from_millis(500));
    assert!(ui.threads.writes().is_empty());
    present_at(&mut ui, now + std::time::Duration::from_millis(999));
    assert_eq!(ui.threads.writes().len(), 1);
}

#[test]
fn unread_explicit_read_clears_all_observed_marks_without_reading_unseen_history() {
    for refused in [false, true] {
        let one = smallclaims::store::operation_id_for_key("unread/explicit-one");
        let two = smallclaims::store::operation_id_for_key("unread/explicit-two");
        let mut ui = fixture(true, &[
            (one.as_str(), "message/offscreen-one"),
            (two.as_str(), "message/offscreen-two"),
        ]);
        if refused {
            ui.threads.seek_refused.insert(ID.into());
        } else {
            ui.threads.seek_pending.insert(ID.into(), "seek/pending".into());
        }
        assert!(ui.threads.mark_visible_read(HOST, ID.into(), &HashSet::new()));
        let writes = ui.threads.writes();
        assert_eq!(writes.len(), 1);
        let Mutation::ClearUnread(clear) = &writes[0].mutation else {
            panic!("explicit clear must not mark unseen history read")
        };
        assert_eq!(clear.observed_marks, [one, two]);
    }
}

#[test]
fn unread_geometry_change_with_same_message_id_restarts_the_dwell() {
    let mut ui = fixture(true, &[]);
    let now = std::time::Instant::now();
    let mut proof = super::super::BodyGeometry::default();
    let shown = |proof| [(ID.into(), [("message/visible".into(), proof)].into())].into();
    ui.threads.presented(HOST, &shown(proof), now);
    proof.scroll_top = 1;
    ui.threads.presented(HOST, &shown(proof), now + std::time::Duration::from_millis(499));
    ui.threads.presented(HOST, &shown(proof), now + std::time::Duration::from_millis(500));
    assert!(ui.threads.writes().is_empty());
    ui.threads.presented(HOST, &shown(proof), now + std::time::Duration::from_millis(999));
    assert_eq!(ui.threads.writes().len(), 1);
}

#[tokio::test]
async fn unread_v2_peer_requires_upgrade_without_querying_new_thread_endpoints() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let queries = Arc::new(AtomicUsize::new(0));
    let observed = queries.clone();
    let stream_queries = queries.clone();
    let app = axum::Router::new()
        .route("/v1/client/capabilities", axum::routing::get(|| async {
            let mut caps: serde_json::Value = serde_json::from_str(include_str!(
                "../../../../../docs/st3/client-v0/fixtures/capabilities.json"
            )).unwrap();
            caps["api_version"] = serde_json::json!(fractal_client::API_VERSION);
            caps["value"]["capabilities"] = serde_json::json!([
                {"id":"collections","version":1,"state":"granted"},
                {"id":"threads-v1","version":2,"state":"granted"},
                {"id":"thread-summary-feed","version":1,"state":"granted"}
            ]);
            axum::Json(caps)
        }))
        .route("/v1/client/collections/stream", axum::routing::get(
            move |upgrade: axum::extract::ws::WebSocketUpgrade| {
                let stream_queries = stream_queries.clone();
                async move {
                    // The feed negotiates capabilities only after its collection handshake.
                    upgrade.protocols(["fractal.client.collections.v0"])
                        .on_upgrade(move |mut socket| async move {
                            while let Some(Ok(message)) = socket.recv().await {
                                if let axum::extract::ws::Message::Text(text) = message {
                                    let command: serde_json::Value = serde_json::from_str(&text).unwrap();
                                    if matches!(command["collection"].as_str(), Some("threads" | "thread-timeline")) {
                                        stream_queries.fetch_add(1, Ordering::SeqCst);
                                    }
                                }
                            }
                        })
                }
            },
        ))
        .fallback(move |uri: axum::http::Uri| {
            let observed = observed.clone();
            async move {
                if uri.path().starts_with("/v1/client/threads") {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                axum::http::StatusCode::NOT_FOUND
            }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = fractal_client::Client::fabric_loopback(
        format!("http://{}", listener.local_addr().unwrap()), "controlled-transport-receiver");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let (updates, received) = std::sync::mpsc::channel();
    let (_commands, requests) = tokio::sync::mpsc::unbounded_channel();
    let feed = tokio::spawn(fractal_feed::run_members_with_sidebar(
        vec![client], false, true, true, Some("person/fixture".into()), updates, requests));
    let (available, access, upgrade_required) = tokio::task::spawn_blocking(move || {
        loop {
            if let fractal_feed::Update::ThreadSupport { available, access, upgrade_required } =
                received.recv_timeout(std::time::Duration::from_secs(5)).unwrap()
            {
                break (available, access, upgrade_required);
            }
        }
    }).await.unwrap();
    assert!(!available);
    assert!(upgrade_required);
    let mut ui = fixture(false, &[]);
    ui.threads.enable(available);
    ui.threads.set_access(access);
    assert!(ui.threads.marked_due(&Pane::Home(None)).is_none());
    present(&mut ui);
    assert!(ui.threads.seeks.is_empty());
    assert_eq!(queries.load(Ordering::SeqCst), 0);
    feed.abort();
    server.abort();
}

#[test]
fn unread_action_menu_keeps_explicit_clear_available_with_no_visible_body() {
    let mark = smallclaims::store::operation_id_for_key("unread/menu-offscreen");
    let mut ui = fixture(false, &[(mark.as_str(), "message/offscreen")]);
    ui.threads.seek_refused.insert(ID.into());
    // No automatic foreground signal and no drawn body are needed for an explicit clear.
    ui.terminal_foreground = None;
    present(&mut ui);
    ui.open_palette(None, Open::Here);
    ui.paste("Mark read".into());
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let writes = ui.threads.writes();
    assert_eq!(writes.len(), 1);
    let Mutation::ClearUnread(clear) = &writes[0].mutation else {
        panic!("menu must clear the reminder, not unseen history")
    };
    assert_eq!(clear.observed_marks, [mark]);
}
