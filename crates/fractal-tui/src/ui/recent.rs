//! Recent content destinations share the navigation history, not a second MRU list.
use super::{Ui, glass_store::StoredPlace, theme};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode};
use ratatui::{buffer::Buffer, layout::Rect, widgets::{Block, BorderType, Clear, Widget}};

#[derive(Default)]
pub(super) struct RecentState {
    control_release_seen: bool,
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
    commit_on_release: bool,
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

    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = self.entries.iter().enumerate()
            .filter(|(_, entry)| query.split_whitespace().all(|word| entry.search.contains(word)))
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        // Typing deliberately switches to the explicit Enter/Esc interaction.
        self.commit_on_release = false;
    }
}

impl Ui {
    pub(super) fn recent_open(&self) -> bool {
        self.recent.popup.is_some()
    }

    /// Explicit popup with the previous destination selected, also on legacy Ctrl+6/Ctrl+^.
    pub(super) fn open_recent(&mut self) {
        self.start_recent(false, false);
    }

    fn start_recent(&mut self, commit_on_release: bool, backwards: bool) {
        let entries: Vec<_> = self.recent_places().into_iter().map(|place| {
            let label = self.place_label(&place);
            let search = label.to_lowercase();
            Entry { place, label, search }
        }).collect();
        let mut popup = Popup {
            matches: (0..entries.len()).collect(),
            entries,
            selected: 0,
            query: String::new(),
            commit_on_release,
        };
        popup.cycle(backwards);
        self.recent.popup = Some(popup);
    }

    fn commit_recent(&mut self) {
        let Some(popup) = self.recent.popup.take() else { return };
        if let Some(&index) = popup.matches.get(popup.selected) {
            self.restore_place(&popup.entries[index].place);
        }
    }

    /// Run before dropping release events, including while an attached program has focus.
    /// Observation alone never captures a program's key. Only an active hold switcher
    /// consumes its Control release. Negotiated enhancement is not evidence of releases.
    pub(super) fn observe_recent_key(&mut self, key: KeyEvent) -> bool {
        // crossterm 0.29 represents kitty modifier releases as Modifier(LeftControl /
        // RightControl) + Release, not Char('c') or a released Tab carrying CONTROL.
        let control_release = key.kind == KeyEventKind::Release
            && matches!(key.code, KeyCode::Modifier(
                ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl
            ));
        if !control_release {
            return false;
        }
        self.recent.control_release_seen = true;
        if self.recent.popup.as_ref().is_some_and(|popup| popup.commit_on_release) {
            self.commit_recent();
            return true;
        }
        false
    }

    /// Dispatch after the parent's attached-program passthrough gate (unless already open).
    /// Returns true exactly when the recent surface owns this event.
    pub(super) fn recent_key(&mut self, key: KeyEvent) -> bool {
        if self.observe_recent_key(key) {
            return true;
        }
        if key.kind == KeyEventKind::Release {
            return self.recent_open();
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let shortcut_modifiers = !key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER);
        let cycle = control && shortcut_modifiers
            && matches!(key.code, KeyCode::Tab | KeyCode::BackTab);
        let backwards = key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab;
        if self.recent.popup.is_none() {
            if control && shortcut_modifiers && matches!(key.code, KeyCode::Char('6' | '^')) {
                self.open_recent();
                return true;
            }
            if cycle && self.keyboard_enhanced {
                self.start_recent(self.recent.control_release_seen, backwards);
                return true;
            }
            return false;
        }
        if cycle {
            self.recent.popup.as_mut().unwrap().cycle(backwards);
            return true;
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
            KeyCode::Char(ch) if !key.modifiers.intersects(
                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER
            ) => {
                let popup = self.recent.popup.as_mut().unwrap();
                popup.query.push(ch);
                popup.filter();
            }
            _ => {}
        }
        true
    }

    pub(super) fn draw_recent(&self, buf: &mut Buffer, area: Rect) {
        let Some(popup) = &self.recent.popup else { return };
        if area.width < 8 || area.height < 6 {
            return;
        }
        let width = area.width.saturating_sub(4).min(88);
        let height = area.height.saturating_sub(2).min(22);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        self.cover(rect);
        Clear.render(rect, buf);
        Block::bordered().border_type(BorderType::Rounded)
            .title(" Recent places ")
            .style(theme::text().bg(theme::MANTLE))
            .border_style(theme::fg(theme::SURFACE2))
            .render(rect, buf);
        let x = rect.x + 2;
        let room = rect.width.saturating_sub(4);
        buf.set_stringn(x, rect.y + 1, format!("› {}▏", popup.query), room as usize,
            theme::fg(theme::ACCENT).bg(theme::MANTLE));
        let rows = rect.height.saturating_sub(4) as usize;
        let start = popup.selected.saturating_sub(rows.saturating_sub(1));
        for (row, (index, &entry)) in popup.matches.iter().enumerate().skip(start).take(rows).enumerate() {
            let selected = index == popup.selected;
            let style = theme::text().bg(if selected { theme::ROW_SELECTED } else { theme::MANTLE });
            let row_rect = Rect { x, y: rect.y + 2 + row as u16, width: room, height: 1 };
            buf.set_style(row_rect, style);
            buf.set_stringn(x, row_rect.y, &popup.entries[entry].label, room as usize, style);
        }
        if popup.matches.is_empty() {
            buf.set_stringn(x, rect.y + 2, "No recent places match", room as usize,
                theme::soft().bg(theme::MANTLE));
        }
        let hint = if popup.commit_on_release {
            "Ctrl+Tab: cycle · release Ctrl: open · Esc: cancel"
        } else {
            "↑↓: choose · Enter: open · Esc: cancel · type: filter"
        };
        buf.set_stringn(x, rect.bottom() - 2, hint, room as usize,
            theme::soft().bg(theme::MANTLE));
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
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL
            | if backwards { KeyModifiers::SHIFT } else { KeyModifiers::NONE })
    }

    fn release(modifier: ModifierKeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(KeyCode::Modifier(modifier), KeyModifiers::NONE, KeyEventKind::Release)
    }

    #[test]
    fn popup_preselects_previous_filters_draws_and_commits_real_destination() {
        let mut ui = places();
        let previous = ui.recent_places()[1].pane.clone();
        assert!(ui.recent_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::CONTROL)));
        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(popup.entries[popup.matches[popup.selected]].place.pane, previous);
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
        assert_eq!(ui.focused_pane(), Some(Pane::Documents(Some("recent-alpha".into()))));
    }

    #[test]
    fn fallback_does_not_commit_on_first_observed_release_and_escape_cancels() {
        let mut ui = places();
        ui.keyboard_enhanced = true;
        let before = ui.focused_pane();
        assert!(ui.recent_key(ctrl_tab(false)));
        assert!(!ui.recent.popup.as_ref().unwrap().commit_on_release);
        assert!(!ui.observe_recent_key(release(ModifierKeyCode::LeftControl)));
        assert!(ui.recent_open());
        assert_eq!(ui.focused_pane(), before);
        ui.recent_key(key(KeyCode::Esc));
        assert!(!ui.recent_open());
        assert_eq!(ui.focused_pane(), before);
    }

    #[test]
    fn observed_control_release_enables_forward_backward_hold_cycle_and_commit() {
        for modifier in [ModifierKeyCode::LeftControl, ModifierKeyCode::RightControl] {
            let mut ui = places();
            ui.keyboard_enhanced = true;
            assert!(!ui.observe_recent_key(release(modifier)));
            let before = ui.focused_pane();
            let previous = ui.recent_places()[1].pane.clone();
            assert!(ui.recent_key(ctrl_tab(false)));
            assert!(ui.recent.popup.as_ref().unwrap().commit_on_release);
            ui.recent_key(ctrl_tab(false));
            ui.recent_key(ctrl_tab(true));
            assert_eq!(ui.focused_pane(), before, "cycling is only a preview");
            assert!(!ui.observe_recent_key(KeyEvent::new_with_kind(
                KeyCode::Tab, KeyModifiers::CONTROL, KeyEventKind::Release
            )));
            assert!(ui.recent_open(), "Tab release is not Control release");
            assert!(ui.observe_recent_key(release(modifier)));
            assert!(!ui.recent_open());
            assert_eq!(ui.focused_pane().unwrap().key(), previous);
        }
    }

    #[test]
    fn typing_in_hold_mode_requires_explicit_commit_and_empty_filter_is_safe() {
        let mut ui = places();
        ui.keyboard_enhanced = true;
        ui.observe_recent_key(release(ModifierKeyCode::LeftControl));
        ui.recent_key(ctrl_tab(false));
        let before = ui.focused_pane();
        for ch in "not-a-recent-place".chars() {
            ui.recent_key(key(KeyCode::Char(ch)));
        }
        ui.observe_recent_key(release(ModifierKeyCode::LeftControl));
        assert!(ui.recent_open());
        assert!(ui.recent.popup.as_ref().unwrap().matches.is_empty());
        ui.recent_key(key(KeyCode::Down));
        ui.recent_key(key(KeyCode::Enter));
        assert!(!ui.recent_open());
        assert_eq!(ui.focused_pane(), before);
    }

    #[test]
    fn legacy_popup_never_becomes_hold_mode_and_plain_keys_are_not_captured() {
        let mut ui = places();
        ui.observe_recent_key(release(ModifierKeyCode::LeftControl));
        assert!(!ui.recent_key(key(KeyCode::Char('x'))));
        assert!(!ui.recent_key(ctrl_tab(false)), "legacy terminals do not distinguish Ctrl+Tab");
        assert!(ui.recent_key(KeyEvent::new(KeyCode::Char('^'), KeyModifiers::CONTROL)));
        assert!(!ui.recent.popup.as_ref().unwrap().commit_on_release);
        ui.observe_recent_key(release(ModifierKeyCode::RightControl));
        assert!(ui.recent_open());
        ui.recent_key(key(KeyCode::Esc));
    }

    #[test]
    fn backward_first_cycle_wraps_and_other_modifier_releases_do_not_enable_hold() {
        let mut ui = places();
        ui.keyboard_enhanced = true;
        assert!(!ui.observe_recent_key(release(ModifierKeyCode::LeftShift)));
        let oldest = ui.recent_places().last().unwrap().pane.clone();
        ui.recent_key(ctrl_tab(true));
        let popup = ui.recent.popup.as_ref().unwrap();
        assert!(!popup.commit_on_release);
        assert_eq!(popup.entries[popup.matches[popup.selected]].place.pane, oldest);
        ui.recent_key(key(KeyCode::Enter));
        assert_eq!(ui.focused_pane().unwrap().key(), oldest);
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
            assert!(panes.insert(&entry.place.pane), "a pane occurs only once across spaces");
            spaces.insert(&entry.place.space);
        }
        assert!(spaces.len() > 1);
        for ch in "no-match".chars() {
            ui.recent_key(key(KeyCode::Char(ch)));
        }
        for _ in 0.."no-match".len() {
            ui.recent_key(key(KeyCode::Backspace));
        }
        assert_eq!(ui.recent.popup.as_ref().unwrap().matches.len(), history.len());
    }

    #[test]
    fn previous_recent_tracks_back_visits_instead_of_the_forward_history_tail() {
        let mut ui = places();
        ui.open_in_glass(Pane::Documents(Some("recent-delta".into())), Open::Tab);
        ui.navigate_history(false);
        ui.navigate_history(false);
        ui.open_recent();
        ui.recent_key(key(KeyCode::Enter));
        assert_eq!(ui.focused_pane(), Some(Pane::Documents(Some("recent-gamma".into()))));
        ui.open_recent();
        ui.recent_key(key(KeyCode::Enter));
        assert_eq!(ui.focused_pane(), Some(Pane::Documents(Some("recent-beta".into()))));
    }

    #[test]
    fn ui_dispatch_keeps_terminal_tab_passthrough_but_opens_explicit_popup() {
        use crate::ui::{Effect, doc::Hit};
        let mut ui = places();
        ui.keyboard_enhanced = true;
        let id = ui.world.agents.items()[0].id.clone();
        ui.open_in_glass(Pane::Agent(Some(id)), Open::Tab);
        ui.click(Hit::Attach);
        ui.effects.clear();
        let cycle = ctrl_tab(false);
        ui.key(cycle);
        assert!(!ui.recent_open());
        assert!(matches!(ui.effects.last(), Some(Effect::TerminalKey(sent)) if *sent == cycle));
        ui.key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::CONTROL));
        assert!(ui.recent_open());
        ui.key(key(KeyCode::Esc));
        assert!(ui.terminal_focused());
        ui.key(KeyEvent::new(KeyCode::Char('^'), KeyModifiers::CONTROL));
        ui.key(key(KeyCode::Enter));
        assert!(!ui.recent_open());
        assert!(!ui.terminal_focused());
    }
}
