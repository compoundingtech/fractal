//! Recent content destinations share the navigation history, not a second MRU list.
use super::{Ui, glass_store::StoredPlace, theme};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, BorderType, Clear, Widget},
};

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
        // Typing deliberately switches to the explicit Enter/Esc interaction.
        self.commit_on_release = false;
    }
}

impl Ui {
    pub(super) fn recent_open(&self) -> bool {
        self.recent.popup.is_some()
    }

    /// Refresh the open snapshot only when the shared tab-status observations change.
    /// This is not typing: keep both the selected destination and release-to-commit mode.
    pub(in crate::ui) fn refresh_recent_labels(&mut self) {
        let Some(mut popup) = self.recent.popup.take() else {
            return;
        };
        let selected_entry = popup.matches.get(popup.selected).copied();
        let mut old_index = 0;
        let mut new_index = 0;
        let mut surviving_selection = None;
        popup.entries.retain_mut(|entry| {
            let keep = self.place_exists(&entry.place);
            if keep {
                if selected_entry == Some(old_index) {
                    surviving_selection = Some(new_index);
                }
                entry.label = self.place_label(&entry.place);
                entry.search = entry.label.to_lowercase();
                new_index += 1;
            }
            old_index += 1;
            keep
        });
        popup.refresh_matches();
        popup.selected = surviving_selection
            .and_then(|entry| popup.matches.iter().position(|index| *index == entry))
            .unwrap_or(0);
        self.recent.popup = Some(popup);
    }
    /// Explicit popup with the previous destination selected, also on legacy Ctrl+6/Ctrl+^.
    pub(super) fn open_recent(&mut self) {
        self.start_recent(false, false);
    }

    fn start_recent(&mut self, commit_on_release: bool, backwards: bool) {
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
            commit_on_release,
        };
        popup.cycle(backwards);
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

    /// Run before dropping release events, including while an attached program has focus.
    /// Observation alone never captures a program's key. Only an active hold switcher
    /// consumes its Control release. Negotiated enhancement is not evidence of releases.
    pub(super) fn observe_recent_key(&mut self, key: KeyEvent) -> bool {
        // crossterm 0.29 represents kitty modifier releases as Modifier(LeftControl /
        // RightControl) + Release, not Char('c') or a released Tab carrying CONTROL.
        let control_release = key.kind == KeyEventKind::Release
            && matches!(
                key.code,
                KeyCode::Modifier(ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl)
            );
        if !control_release {
            return false;
        }
        self.recent.control_release_seen = true;
        if self
            .recent
            .popup
            .as_ref()
            .is_some_and(|popup| popup.commit_on_release)
        {
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
        let shortcut_modifiers = !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER);
        let cycle =
            control && shortcut_modifiers && matches!(key.code, KeyCode::Tab | KeyCode::BackTab);
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
        popup
            .query
            .extend(line.chars().filter(|ch| !ch.is_control()));
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
        let height = area.height.saturating_sub(2).clamp(5, 22);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
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
            let selected = index == popup.selected;
            let style = theme::text().bg(if selected {
                theme::ROW_SELECTED
            } else {
                theme::MANTLE
            });
            let row_rect = Rect {
                x,
                y: rect.y + 2 + row as u16,
                width: room,
                height: 1,
            };
            buf.set_style(row_rect, style);
            buf.set_stringn(
                x,
                row_rect.y,
                &popup.entries[entry].label,
                room as usize,
                style,
            );
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
        let hint = if popup.commit_on_release {
            "Ctrl+Tab: cycle · release Ctrl: open · Esc: cancel"
        } else {
            "↑↓: choose · Enter: open · Esc: cancel · type: filter"
        };
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
    use crate::ui::{
        demo,
        glass::Open,
        pane::Pane,
        view::{AgentState, AttentionKind, Link, Load},
    };

    fn places() -> Ui {
        let mut ui = Ui::new(demo::world());
        ui.open_in_glass(Pane::Documents(Some("recent-alpha".into())), Open::Tab);
        ui.open_in_glass(Pane::Documents(Some("recent-beta".into())), Open::Tab);
        ui.open_in_glass(Pane::Documents(Some("recent-gamma".into())), Open::Tab);
        ui
    }

    fn live_places() -> Ui {
        let mut world = demo::world();
        world.link = Link::Live;
        let template = world.agents.items()[0].clone();
        world.agents = Load::Ready(
            ["agent/recent-a", "agent/recent-b"]
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    let mut agent = template.clone();
                    agent.id = id.into();
                    agent.name = format!("Live agent {index}");
                    agent.state = AgentState::Idle;
                    agent
                })
                .collect(),
        );
        world.attention = Load::Ready(Vec::new());
        let mut ui = Ui::new(world);
        ui.open_in_glass(Pane::Agent(Some("agent/recent-a".into())), Open::Tab);
        ui.open_in_glass(Pane::Agent(Some("agent/recent-b".into())), Open::Tab);
        ui.open_in_glass(Pane::Documents(Some("recent-notes".into())), Open::Tab);
        ui
    }

    fn selected_place(ui: &Ui) -> &StoredPlace {
        let popup = ui.recent.popup.as_ref().unwrap();
        &popup.entries[popup.matches[popup.selected]].place
    }

    fn add_live_question(ui: &mut Ui) {
        let mut world = ui.world.clone();
        if let Load::Ready(agents) = &mut world.agents {
            for agent in agents {
                agent.state = AgentState::Working;
                agent.name.push_str(" updated");
            }
        }
        let mut alert = demo::world().attention.items()[0].clone();
        alert.id = "attention/recent-question".into();
        alert.agent = Some("agent/recent-b".into());
        alert.conversations = vec!["agent/recent-b".into()];
        alert.kind = AttentionKind::Prompt {
            seat: "Live agent 1".into(),
            seat_id: "agent/recent-b".into(),
            text: "Choose".into(),
            question: true,
            answers: Vec::new(),
            episode: "episode/recent-question".into(),
            call: None,
            calls: Ok(Vec::new()),
            allow: true,
        };
        world.attention = Load::Ready(vec![alert]);
        ui.set_world(world);
    }

    #[test]
    fn world_refresh_updates_open_labels_activity_and_questions_without_ending_hold() {
        let mut ui = live_places();
        ui.keyboard_enhanced = true;
        ui.observe_recent_key(release(ModifierKeyCode::LeftControl));
        ui.recent_key(ctrl_tab(false));
        let selected = selected_place(&ui).clone();
        assert_eq!(selected.pane, "agent:agent/recent-b");
        let before = ui.recent.popup.as_ref().unwrap().entries[1].label.clone();

        add_live_question(&mut ui);

        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(selected_place(&ui), &selected);
        assert_eq!(
            popup.selected, 1,
            "refresh must not reset to the first match"
        );
        assert!(popup.commit_on_release);
        let entry = &popup.entries[popup.matches[popup.selected]];
        assert_eq!(entry.label, ui.place_label(&selected));
        assert_ne!(entry.label, before);
        assert!(entry.label.contains("updated"));
        assert!(entry.label.starts_with("● "), "working activity is live");
        assert!(entry.label.contains('?'), "live question is visible");
        assert_eq!(entry.search, entry.label.to_lowercase());
        let area = Rect::new(0, 0, 100, 24);
        let mut buffer = Buffer::empty(area);
        ui.draw_recent(&mut buffer, area);
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("updated"));
        assert!(rendered.contains('?'));
        assert!(ui.observe_recent_key(release(ModifierKeyCode::LeftControl)));
        assert_eq!(ui.focused_pane().unwrap().key(), selected.pane);
    }

    #[test]
    fn world_refresh_keeps_query_selected_match_and_explicit_commit_mode() {
        let mut ui = live_places();
        ui.keyboard_enhanced = true;
        ui.observe_recent_key(release(ModifierKeyCode::LeftControl));
        ui.recent_key(ctrl_tab(false));
        for ch in "live agent".chars() {
            ui.recent_key(key(KeyCode::Char(ch)));
        }
        ui.recent_key(key(KeyCode::Down));
        let selected = selected_place(&ui).clone();
        assert_eq!(ui.recent.popup.as_ref().unwrap().selected, 1);

        add_live_question(&mut ui);

        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(popup.query, "live agent");
        assert_eq!(popup.matches.len(), 2);
        assert_eq!(popup.selected, 1);
        assert_eq!(selected_place(&ui), &selected);
        assert!(!popup.commit_on_release);
        assert!(!ui.observe_recent_key(release(ModifierKeyCode::LeftControl)));
        assert!(ui.recent_open());
    }

    #[test]
    fn world_refresh_refilters_current_query_and_prunes_disappeared_destinations() {
        let mut ui = live_places();
        ui.open_recent();
        ui.recent_key(key(KeyCode::Char('?')));
        assert!(ui.recent.popup.as_ref().unwrap().matches.is_empty());

        add_live_question(&mut ui);

        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(
            popup.matches.len(),
            2,
            "the agent and Inbox both show the question"
        );
        assert!(
            popup
                .matches
                .iter()
                .any(|index| { popup.entries[*index].place.pane == Pane::List(0).key() })
        );
        assert_eq!(selected_place(&ui).pane, "agent:agent/recent-b");
        let mut world = ui.world.clone();
        if let Load::Ready(items) = &mut world.attention {
            for item in items {
                if let AttentionKind::Prompt { question, .. } = &mut item.kind {
                    *question = false;
                }
            }
        }
        ui.set_world(world);
        let popup = ui.recent.popup.as_ref().unwrap();
        assert_eq!(popup.query, "?");
        assert!(popup.matches.is_empty());
        assert_eq!(popup.selected, 0);
        assert!(!popup.commit_on_release);

        let mut world = ui.world.clone();
        if let Load::Ready(agents) = &mut world.agents {
            agents.retain(|agent| agent.id != "agent/recent-b");
        }
        ui.set_world(world);
        assert!(
            ui.recent
                .popup
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .all(|entry| { entry.place.pane != "agent:agent/recent-b" })
        );
        ui.recent_key(key(KeyCode::Esc));
        ui.set_world(ui.world.clone());
        assert!(!ui.recent_open(), "refreshing a closed popup is a no-op");
    }

    #[test]
    fn ui_dispatch_legacy_control_tab_does_not_switch_a_detached_quiet_pane() {
        let mut ui = places();
        assert!(!ui.keyboard_enhanced);
        assert!(!ui.terminal_focused());
        let before = ui.current_place().unwrap();
        for event in [
            ctrl_tab(false),
            ctrl_tab(true),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL),
        ] {
            ui.key(event);
            assert_eq!(ui.current_place().unwrap(), before);
            assert!(!ui.recent_open());
            assert!(!ui.terminal_focused());
        }
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

    fn release(modifier: ModifierKeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(
            KeyCode::Modifier(modifier),
            KeyModifiers::NONE,
            KeyEventKind::Release,
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
                KeyCode::Tab,
                KeyModifiers::CONTROL,
                KeyEventKind::Release
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
        assert!(
            !ui.recent_key(ctrl_tab(false)),
            "legacy terminals do not distinguish Ctrl+Tab"
        );
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
        assert_eq!(
            popup.entries[popup.matches[popup.selected]].place.pane,
            oldest
        );
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
            let room = buffer
                .area
                .width
                .saturating_sub(4)
                .min(88)
                .saturating_sub(4) as usize;
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
        let body = ui
            .terminal_body
            .get()
            .expect("the attached program is drawn");
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
        assert_eq!(
            ui.recent.popup.as_ref().unwrap().query,
            "recent-alphadirect"
        );
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
