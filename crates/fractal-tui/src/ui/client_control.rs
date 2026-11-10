//! Status and local stop latch for the existing device-command protocol.
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Default)]
pub(super) struct Status {
    pub run_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoke_requested: bool,
    pub focus_lost: bool,
}

impl Status {
    pub fn active(&self) -> bool {
        !self.revoke_requested && self.run_id.is_some()
            && self.expires_at.is_some_and(|expiry| expiry > Utc::now())
    }

    pub fn indicator(&self) -> Option<String> {
        if self.revoke_requested {
            Some("Client control stopped · enrollment revocation pending".into())
        } else if self.active() {
            Some(format!("Test run {} · ctrl+alt+x revoke enrollment", self.run_id.as_deref()?))
        } else {
            None
        }
    }

    /// Stop locally before any network operation. This binding also wins over embedded PTYs.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if self.active() && key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('x')
            && key.modifiers == KeyModifiers::CONTROL.union(KeyModifiers::ALT)
        {
            self.revoke_requested = true;
            true
        } else {
            false
        }
    }
}

impl super::Ui {
    pub(crate) fn control_apply(&mut self, control: &crate::client_control::Control, command: &serde_json::Value) -> bool {
        control.allows(command, self.control_screen()) && self.control_navigate(command)
    }

    pub(super) fn control_foreground(&self) -> bool {
        // Both calls only query the controlling terminal and current process group.
        !self.client_control.focus_lost
            && unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) == libc::getpgrp() }
    }

    pub(super) fn control_screen(&self) -> &'static str {
        if self.help || self.popover.is_some() || self.glasses.palette_open() { return ""; }
        if self.home_open() { return "NowRoot"; }
        match self.focused_pane() {
            Some(super::pane::Pane::List(0)) => "NowRoot",
            Some(super::pane::Pane::Home(Some(_))) => "Attention",
            _ => "",
        }
    }

    pub(super) fn control_navigate(&mut self, command: &serde_json::Value) -> bool {
        if self.help || self.popover.is_some() || self.glasses.palette_open() { return false; }
        if command["command_kind"] == "phone.inspect" { return true; }
        match command["args"]["screen"].as_str() {
            Some("NowRoot") => { self.open_home(); true }
            Some("Attention") => {
                let Some(id) = command["args"]["params"]["id"].as_str() else { return false; };
                if !self.world.attention.items().iter().any(|item| item.id == id
                    && matches!(item.kind, super::view::AttentionKind::Request { .. })) { return false; }
                self.open_in_glass(super::pane::Pane::Home(Some(id.into())), super::glass::Open::Tab);
                true
            }
            _ => false,
        }
    }

    /// Called only after the terminal draw; ask content comes from the shipped card model.
    pub(crate) fn control_visible(&self) -> fractal_schema::client_control::ClientVisibleState {
        use fractal_schema::client_control::{ClientVisibleItem, ClientVisibleState};
        let screen = self.control_screen();
        let pane = self.focused_pane();
        let route = pane.as_ref().map_or_else(String::new, |pane| pane.key());
        let drawn = self.frame.borrow().panes.iter().any(|pane|
            pane.key == route && pane.rect.width > 0 && pane.rect.height > 0);
        let opened_item = if drawn && screen == "Attention" {
            let id = match &pane { Some(super::pane::Pane::Home(Some(id))) => Some(id), _ => None };
            id.and_then(|id| self.world.attention.items().iter().find(|item| item.id == *id))
                .and_then(|item| match &item.kind {
                    super::view::AttentionKind::Request { question, structured, .. } =>
                        Some(ClientVisibleItem { id: item.id.clone(), title: item.title.clone(),
                            body: structured.as_ref().map_or(question, |request| &request.question).clone() }),
                    _ => None,
                })
        } else { None };
        let mut visible_item_ids = if screen == "NowRoot" {
            self.frame.borrow().context_rows.iter()
                .filter(|(_, id, _)| self.world.attention.items().iter().any(|item| item.id == *id))
                .map(|(_, id, _)| id.clone()).collect::<Vec<_>>()
        } else { opened_item.iter().map(|item| item.id.clone()).collect() };
        visible_item_ids.sort();
        visible_item_ids.dedup();
        ClientVisibleState { route, screen: screen.into(), visible_item_ids, opened_item }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active() -> Status {
        Status { run_id: Some("run/fixture-1".into()),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(1)),
            revoke_requested: false, focus_lost: false }
    }

    #[test]
    fn indicator_names_run_and_revoke_binding() {
        let status = active();
        let indicator = status.indicator().unwrap();
        assert!(indicator.contains("run/fixture-1"));
        assert!(indicator.contains("ctrl+alt+x"));
        assert!(Status::default().indicator().is_none());
        let mut expired = active();
        expired.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert!(expired.indicator().is_none());
    }

    #[test]
    fn revoke_key_stops_before_network_and_never_repeats() {
        let mut status = active();
        assert!(!status.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert!(status.key(key));
        assert!(!status.active());
        assert!(status.revoke_requested);
        assert!(!status.key(key));
        assert!(status.indicator().unwrap().contains("revocation pending"));
    }

    #[test]
    fn status_line_and_global_revoke_work_even_offline_and_editing() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let mut ui = super::super::Ui::new(super::super::demo::test_world());
        ui.world.link = super::super::view::Link::Offline("offline".into());
        ui.client_control = active();
        ui.editing = true;
        let area = Rect::new(0, 0, 100, 1);
        let mut buffer = Buffer::empty(area);
        ui.footer(&mut buffer, area);
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Test run run/fixture-1"));
        assert!(text.contains("ctrl+alt+x"));
        ui.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert!(ui.client_control.revoke_requested);
        assert!(ui.effects.is_empty(), "the revoke chord must not type into or submit a draft");
    }

    #[test]
    fn ascii_seed_renders_exactly_one_line_and_navigation_sends_no_answer_key() {
        use super::super::{Effect, Ui, demo, screens, view::{AttentionKind, Load}};
        use ratatui::{Terminal, backend::TestBackend};
        for (title, question) in [
            ("client control fixture request".to_owned(), "Seeded request body for terminal verification".to_owned()),
            ("T".repeat(60), "Q".repeat(60)),
        ] {
            let mut world = demo::test_world();
            let Load::Ready(items) = &mut world.attention else { panic!("demo attention must be loaded"); };
            let item = items.first_mut().unwrap();
            item.title = title.clone();
            item.kind = AttentionKind::Request { from: "Fixture producer".into(),
                from_id: "agent/fixture".into(), question: question.clone(), structured: None, images: Vec::new() };
            let id = item.id.clone();
            let mut ui = Ui::new(world);
            assert!(ui.control_navigate(&serde_json::json!({
                "command_kind":"phone.navigate","args":{"screen":"Attention","params":{"id":id}}
            })));
            assert!(ui.effects.iter().all(|effect| !matches!(effect,
                Effect::TerminalKey(_) | Effect::Attention { .. } | Effect::Send { .. }
                | Effect::Reply { .. } | Effect::Discuss { .. })),
                "navigation must never synthesize an answer action or terminal key");
            let drafts = ui.drafts_for(&id, 100);
            let doc = screens::home_detail(&ui.world, Some(&id), 100, &drafts);
            let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
            terminal.draw(|frame| {
                let area = frame.area();
                ui.pane(frame.buffer_mut(), &format!("home:{id}"), area, doc.clone(), false);
            }).unwrap();
            let lines: Vec<String> = terminal.backend().buffer().content.chunks(100)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .map(|row| row.trim().trim_matches(['│', '┃']).trim().to_owned()).collect();
            for expected in [&title, &question] {
                assert!(expected.is_ascii() && expected.len() <= 60);
                assert_eq!(lines.iter().filter(|line| *line == expected).count(), 1,
                    "seed text must occupy one exact terminal line: {expected:?}");
            }
        }
    }
}
