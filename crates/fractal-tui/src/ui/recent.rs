//! Recent content destinations share the navigation history, not a second MRU list.
use super::{Ui, glass_store::StoredPlace, theme};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, BorderType, Clear, Widget},
};

#[derive(Default)]
pub(super) struct RecentState {
    popup: Option<Popup>,
}

struct Entry {
    place: StoredPlace,
    label: String,
    search: String,
}

struct Popup {
    entries: Vec<Entry>,
    matches: Vec<usize>,
    selected: usize,
    query: String,
}

impl Popup {
    fn cycle(&mut self, backwards: bool) {
        let count = self.matches.len();
        if count != 0 {
            self.selected = if backwards {
                (self.selected + count - 1) % count
            } else {
                (self.selected + 1) % count
            };
        }
    }

    fn refresh_matches(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query
                    .split_whitespace()
                    .all(|word| entry.search.contains(word))
            })
            .map(|(index, _)| index)
            .collect();
    }

    fn filter(&mut self) {
        self.refresh_matches();
        self.selected = 0;
    }
}

impl Ui {
    pub(super) fn recent_open(&self) -> bool {
        self.recent.popup.is_some()
    }

    /// Explicit popup with the previous destination selected, also on legacy Ctrl+6/Ctrl+^.
    pub(super) fn open_recent(&mut self) {
        let entries: Vec<_> = self
            .recent_places()
            .into_iter()
            .map(|place| {
                let label = self.place_label(&place);
                let search = label.to_lowercase();
                Entry {
                    place,
                    label,
                    search,
                }
            })
            .collect();
        let mut popup = Popup {
            matches: (0..entries.len()).collect(),
            entries,
            selected: 0,
            query: String::new(),
        };
        popup.cycle(false);
        self.recent.popup = Some(popup);
    }

    fn commit_recent(&mut self) {
        let Some(popup) = self.recent.popup.take() else {
            return;
        };
        if let Some(&index) = popup.matches.get(popup.selected) {
            self.restore_place(&popup.entries[index].place);
        }
    }

    /// Dispatch after the parent's attached-program passthrough gate (unless already open).
    /// Returns true exactly when the recent surface owns this event.
    pub(super) fn recent_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return self.recent_open();
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let shortcut_modifiers = !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER);
        if self.recent.popup.is_none() {
            if control && shortcut_modifiers && matches!(key.code, KeyCode::Char('6' | '^')) {
                self.open_recent();
                return true;
            }
            return false;
        }
        match key.code {
            KeyCode::Esc => self.recent.popup = None,
            KeyCode::Enter => self.commit_recent(),
            KeyCode::Up | KeyCode::BackTab => self.recent.popup.as_mut().unwrap().cycle(true),
            KeyCode::Down | KeyCode::Tab => self.recent.popup.as_mut().unwrap().cycle(false),
            KeyCode::Backspace => {
                let popup = self.recent.popup.as_mut().unwrap();
                popup.query.pop();
                popup.filter();
            }
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                let popup = self.recent.popup.as_mut().unwrap();
                popup.query.push(ch);
                popup.filter();
            }
            _ => {}
        }
        true
    }

    /// A paste while the popup is open filters it, one line typed at once, and never reaches
    /// whatever is behind it. Returns true exactly when the popup took the paste.
    pub(super) fn recent_paste(&mut self, text: &str) -> bool {
        let Some(popup) = self.recent.popup.as_mut() else {
            return false;
        };
        let line = text.lines().next().unwrap_or_default();
        popup.query.extend(line.chars().filter(|ch| !ch.is_control()));
        popup.filter();
        true
    }

    pub(super) fn draw_recent(&self, buf: &mut Buffer, area: Rect) {
        let Some(popup) = &self.recent.popup else {
            return;
        };
        if area.width < 8 || area.height < 6 {
            return;
        }
        let width = area.width.saturating_sub(4).min(88);
        // Border, query, footer and at least the selected destination: the outer margin yields first,
        // so the smallest accepted window still shows what Enter opens.
        let height = area.height.saturating_sub(2).min(22).max(5);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        self.cover(rect);
        Clear.render(rect, buf);
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(" Recent places ")
            .style(theme::text().bg(theme::MANTLE))
            .border_style(theme::fg(theme::SURFACE2))
            .render(rect, buf);
        let x = rect.x + 2;
        let room = rect.width.saturating_sub(4);
        buf.set_stringn(
            x,
            rect.y + 1,
            format!("› {}▏", popup.query),
            room as usize,
            theme::fg(theme::ACCENT).bg(theme::MANTLE),
        );
        let rows = rect.height.saturating_sub(4) as usize;
        let start = popup.selected.saturating_sub(rows.saturating_sub(1));
        for (row, (index, &entry)) in popup
            .matches
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .enumerate()
        {
            let background = if index == popup.selected {
                theme::ROW_SELECTED
            } else {
                theme::MANTLE
            };
            let style = theme::text().bg(background);
            let y = rect.y + 2 + row as u16;
            buf.set_style(Rect::new(x, y, room, 1), style);
            buf.set_stringn(x, y, &popup.entries[entry].label, room as usize, style);
        }
        if popup.matches.is_empty() {
            buf.set_stringn(
                x,
                rect.y + 2,
                "No recent places match",
                room as usize,
                theme::soft().bg(theme::MANTLE),
            );
        }
        let hint = "↑↓: choose · Enter: open · Esc: cancel · type: filter";
        buf.set_stringn(
            x,
            rect.bottom() - 2,
            hint,
            room as usize,
            theme::soft().bg(theme::MANTLE),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{demo, glass::Open, pane::Pane};

    fn places() -> Ui {
        let mut ui = Ui::new(demo::world());
        ui.open_in_glass(Pane::Documents(Some("recent-alpha".into())), Open::Tab);
        ui.open_in_glass(Pane::Documents(Some("recent-beta".into())), Open::Tab);
        ui.open_in_glass(Pane::Documents(Some("recent-gamma".into())), Open::Tab);
        ui
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_tab(backwards: bool) -> KeyEvent {
        KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::CONTROL
                | if backwards {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                },
        )
    }

    #[test]
    fn popup_preselects_previous_filters_draws_and_commits_real_destination() {
        let mut ui = places();
        let previous = ui.recent_places()[1].pane.clone();
        assert!(ui.recent_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::CONTROL)));
        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(
            popup.entries[popup.matches[popup.selected]].place.pane,
            previous
        );
        for ch in "recent-alpha".chars() {
            assert!(ui.recent_key(key(KeyCode::Char(ch))));
        }
        assert_eq!(ui.recent.popup.as_ref().unwrap().matches.len(), 1);
        let area = Rect::new(0, 0, 100, 24);
        let mut buf = Buffer::empty(area);
        ui.draw_recent(&mut buf, area);
        let rendered: String = buf.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("Recent places"));
        assert!(rendered.contains("recent-alpha"));
        ui.recent_key(key(KeyCode::Enter));
        assert!(!ui.recent_open());
        assert_eq!(
            ui.focused_pane(),
            Some(Pane::Documents(Some("recent-alpha".into())))
        );
    }

    #[test]
    fn popup_uses_shared_history_and_labels_across_spaces_without_duplicate_panes() {
        let mut ui = places();
        ui.open_in_glass(Pane::Documents(Some("other-space".into())), Open::Glass);
        ui.open_in_glass(Pane::Documents(Some("recent-alpha".into())), Open::Tab);
        let history = ui.recent_places();
        ui.open_recent();
        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(popup.entries.len(), history.len());
        let mut panes = std::collections::HashSet::new();
        let mut spaces = std::collections::HashSet::new();
        for (entry, place) in popup.entries.iter().zip(&history) {
            assert_eq!(entry.place.pane, place.pane);
            assert_eq!(entry.place.space, place.space);
            assert_eq!(entry.label, ui.place_label(place));
            assert!(
                panes.insert(&entry.place.pane),
                "a pane occurs only once across spaces"
            );
            spaces.insert(&entry.place.space);
        }
        assert!(spaces.len() > 1);
        for ch in "no-match".chars() {
            ui.recent_key(key(KeyCode::Char(ch)));
        }
        for _ in 0.."no-match".len() {
            ui.recent_key(key(KeyCode::Backspace));
        }
        assert_eq!(
            ui.recent.popup.as_ref().unwrap().matches.len(),
            history.len()
        );
    }

    #[test]
    fn previous_recent_tracks_back_visits_instead_of_the_forward_history_tail() {
        let mut ui = places();
        ui.open_in_glass(Pane::Documents(Some("recent-delta".into())), Open::Tab);
        ui.navigate_history(false);
        ui.navigate_history(false);
        ui.open_recent();
        ui.recent_key(key(KeyCode::Enter));
        assert_eq!(
            ui.focused_pane(),
            Some(Pane::Documents(Some("recent-gamma".into())))
        );
        ui.open_recent();
        ui.recent_key(key(KeyCode::Enter));
        assert_eq!(
            ui.focused_pane(),
            Some(Pane::Documents(Some("recent-beta".into())))
        );
    }

    #[test]
    fn attached_program_keeps_control_caret_and_tab_while_leader_r_opens_recent() {
        use crate::ui::{Effect, doc::Hit};
        for enhanced in [false, true] {
            let mut ui = places();
            ui.keyboard_enhanced = enhanced;
            let id = ui.world.agents.items()[0].id.clone();
            ui.open_in_glass(Pane::Agent(Some(id)), Open::Tab);
            ui.click(Hit::Attach);
            ui.effects.clear();
            for chord in [
                ctrl_tab(false),
                KeyEvent::new(KeyCode::Char('6'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('^'), KeyModifiers::CONTROL),
            ] {
                for kind in [
                    KeyEventKind::Press,
                    KeyEventKind::Repeat,
                    KeyEventKind::Release,
                ] {
                    let sent = KeyEvent { kind, ..chord };
                    ui.key(sent);
                    assert!(!ui.recent_open());
                    assert!(
                        matches!(ui.effects.last(), Some(Effect::TerminalKey(key)) if *key == sent)
                    );
                }
            }
            ui.key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
            ui.key(key(KeyCode::Char('r')));
            assert!(ui.recent_open());
            ui.key(key(KeyCode::Esc));
            assert!(ui.terminal_focused());
            ui.key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
            ui.key(key(KeyCode::Char('r')));
            ui.key(key(KeyCode::Enter));
            assert!(!ui.recent_open());
            assert!(!ui.terminal_focused());
        }
    }

    #[test]
    fn minimum_window_keeps_the_selected_destination_visible_after_resize() {
        use ratatui::{Terminal, backend::TestBackend};
        let shows_selected = |terminal: &Terminal<TestBackend>, label: &str| {
            let buffer = terminal.backend().buffer();
            let room = buffer.area.width.saturating_sub(4).min(88).saturating_sub(4) as usize;
            let shown: String = label.chars().take(room).collect();
            (0..buffer.area.height).any(|y| {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.find(&shown).is_some_and(|at| {
                    let x = line[..at].chars().count() as u16;
                    buffer[(x, y)].bg == theme::ROW_SELECTED
                })
            })
        };
        let mut ui = places();
        ui.open_recent();
        ui.key(key(KeyCode::Down));
        let popup = ui.recent.popup.as_ref().unwrap();
        assert!(popup.selected > 0, "the selection is not the first row");
        let entry = &popup.entries[popup.matches[popup.selected]];
        let (label, target) = (entry.label.clone(), Pane::parse(&entry.place.pane).unwrap());
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        assert!(shows_selected(&terminal, &label));
        terminal.backend_mut().resize(20, 6);
        for _ in 0..2 {
            terminal.draw(|frame| ui.render(frame)).unwrap();
        }
        assert_eq!(terminal.backend().buffer().area, Rect::new(0, 0, 20, 6));
        assert!(
            shows_selected(&terminal, &label),
            "the selected destination stays visible at 20x6"
        );
        ui.key(key(KeyCode::Enter));
        assert!(!ui.recent_open());
        assert_eq!(ui.focused_pane(), Some(target));

        // Opening straight into the smallest accepted window shows the preselected destination.
        ui.open_recent();
        let popup = ui.recent.popup.as_ref().unwrap();
        let entry = &popup.entries[popup.matches[popup.selected]];
        let (label, target) = (entry.label.clone(), Pane::parse(&entry.place.pane).unwrap());
        let mut terminal = Terminal::new(TestBackend::new(20, 6)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        assert!(shows_selected(&terminal, &label));
        ui.key(key(KeyCode::Enter));
        assert!(!ui.recent_open());
        assert_eq!(ui.focused_pane(), Some(target));
    }

    /// Over a real native terminal connection: leader r takes the program's focus, the popup
    /// keeps the paste and the mouse, and closing it gives the focus back. Any byte that leaked
    /// to the program would arrive ahead of the expected focus report.
    #[test]
    fn recent_popup_owns_paste_mouse_and_focus_over_a_native_attached_program() {
        use crate::ui::{doc::Hit, pty::NativeTerminal};
        use alacritty_terminal::term::TermMode;
        use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
        use pty_core::protocol::{MessageType, PacketReader, encode_packet};
        use ratatui::{Terminal, backend::TestBackend};
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};

        fn input(peer: &mut UnixStream, reader: &mut PacketReader) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                assert!(Instant::now() < deadline, "no terminal input");
                let mut bytes = [0u8; 1024];
                let count = peer.read(&mut bytes).unwrap();
                for packet in reader.feed(&bytes[..count]).unwrap() {
                    if packet.type_ == MessageType::Data {
                        return packet.payload;
                    }
                }
            }
        }
        fn draw(ui: &Ui) {
            let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
            terminal.draw(|frame| ui.render(frame)).unwrap();
        }

        let mut ui = places();
        let id = ui.world.agents.items()[0].id.clone();
        ui.open_in_glass(Pane::Agent(Some(id.clone())), Open::Tab);
        ui.click(Hit::Attach);
        assert!(ui.terminal_focused());
        let (client, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        ui.terminal_view_mut(&id).unwrap().native =
            Some(NativeTerminal::spawn(client, &id, "one".into(), 24, 80));
        peer.write_all(&encode_packet(
            MessageType::Screen,
            b"\x1b[?1004h\x1b[?1003h\x1b[?1006h\x1b[?2004h",
        ))
        .unwrap();
        let wanted = TermMode::FOCUS_IN_OUT | TermMode::MOUSE_MOTION | TermMode::BRACKETED_PASTE;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ui.native_terminal().unwrap().mode().contains(wanted) {
            assert!(Instant::now() < deadline, "screen did not arrive");
            std::thread::sleep(Duration::from_millis(5));
        }
        draw(&ui);
        let body = ui.terminal_body.get().expect("the attached program is drawn");
        let mut reader = PacketReader::new();
        ui.input_event(Event::FocusGained);
        assert_eq!(input(&mut peer, &mut reader), b"\x1b[I");

        ui.input_event(Event::Key(KeyEvent::new(
            KeyCode::Char('\\'),
            KeyModifiers::CONTROL,
        )));
        ui.input_event(Event::Key(key(KeyCode::Char('r'))));
        assert!(ui.recent_open());
        assert_eq!(
            input(&mut peer, &mut reader),
            b"\x1b[O",
            "opening the popup takes the program's focus"
        );

        ui.input_event(Event::Paste("recent-alpha\nsecond line".into()));
        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(popup.query, "recent-alpha", "the paste filters the popup");
        assert_eq!(popup.matches.len(), 1);

        draw(&ui);
        let (column, row) = (body.x + 2, body.y + 2);
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::ScrollDown,
        ] {
            let mouse = MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            };
            ui.input_event(Event::Mouse(mouse));
            ui.mouse(mouse);
        }
        ui.paste("direct".into());
        assert_eq!(ui.recent.popup.as_ref().unwrap().query, "recent-alphadirect");
        // The outer terminal's own focus changes stay with the popup too.
        ui.input_event(Event::FocusLost);
        ui.input_event(Event::FocusGained);
        assert!(ui.recent_open() && ui.terminal_focused());

        ui.input_event(Event::Key(key(KeyCode::Esc)));
        assert!(!ui.recent_open());
        assert_eq!(
            input(&mut peer, &mut reader),
            b"\x1b[I",
            "closing the popup gives the focus back, and nothing reached the program before it"
        );
    }
}
