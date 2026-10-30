//! Device-local content history shared by back/forward and the recent-place switcher.
use super::*;
use glass_store::StoredPlace;

impl Ui {
    pub(in crate::ui) fn current_place(&self) -> Option<StoredPlace> {
        let glass = self.glasses.glass();
        let groups = glass.layout.groups();
        let group = groups.get(glass.focus)?;
        let pane = group.tabs.get(group.current).map_or_else(|| Pane::List(0).key(), |tab| tab.pane.clone());
        Some(StoredPlace { space: glass.id.clone(), group: glass.focus, tab: group.current, pane })
    }

    pub(in crate::ui) fn record_place_change(&mut self, before: Option<StoredPlace>) {
        if self.glasses.restoring { return; }
        let Some(after) = self.current_place() else { return };
        if before.as_ref() == Some(&after) { return; }
        let glasses = &mut self.glasses;
        // A higher-level open may surround show_in; record that destination only once.
        if glasses.history.get(glasses.history_cursor) == Some(&after) { return; }
        glasses.history.truncate(glasses.history_cursor.saturating_add(1));
        if let Some(before) = before {
            if glasses.history.last() != Some(&before) { glasses.history.push(before); }
        }
        glasses.history.push(after);
        if glasses.history.len() > 50 {
            glasses.history.drain(..glasses.history.len() - 50);
        }
        glasses.history_cursor = glasses.history.len() - 1;
        glasses.save();
    }

    fn place_exists(&self, place: &StoredPlace) -> bool {
        if !self.glasses.all.iter().any(|glass| glass.id == place.space &&
            glass.layout.groups().get(place.group).is_some_and(|group| place.tab < group.tabs.len() || (group.tabs.is_empty() && place.tab == 0 && place.pane == Pane::List(0).key()))) {
            return false;
        }
        match Pane::parse(&place.pane) {
            Some(Pane::Agent(Some(id))) => self.world.agents.ready().is_none() || self.world.agents.items().iter().any(|agent| agent.id == id),
            Some(Pane::Mission(Some(id)) | Pane::Declaration(Some(id))) => self.world.missions.ready().is_none() || self.world.missions.items().iter().any(|mission| mission.id == id),
            Some(Pane::Home(Some(id))) => self.world.attention.ready().is_none() || self.world.attention.items().iter().any(|item| item.id == id),
            Some(_) => true,
            None => false,
        }
    }

    pub(in crate::ui) fn restore_place(&mut self, place: &StoredPlace) -> bool {
        if !self.place_exists(place) { return false; }
        let before = self.current_place();
        let was_restoring = self.glasses.restoring;
        self.glasses.restoring = true;
        let index = self.glasses.all.iter().position(|glass| glass.id == place.space).unwrap();
        self.glasses.shown = index;
        self.glasses.home = false;
        self.glasses.palette = None;
        self.glasses.sidebar.focused = false;
        self.glasses.zoomed = false;
        let glass = self.glasses.glass_mut();
        glass.focus = place.group;
        let group = glass.layout.group_mut(place.group).unwrap();
        group.current = place.tab;
        let changed = group.tabs.get(place.tab).is_some_and(|tab| tab.pane != place.pane);
        if let Some(tab) = group.tabs.get_mut(place.tab) {
            tab.pane.clone_from(&place.pane);
        }
        let id = glass.id.clone();
        if changed { self.glass_changed(&id); }
        self.show_focused();
        self.attach_when = None;
        self.glasses.restoring = was_restoring;
        self.record_place_change(before);
        self.glasses.save();
        true
    }

    pub(in crate::ui) fn navigate_history(&mut self, forward: bool) {
        let mut cursor = self.glasses.history_cursor;
        while let Some(next) = if forward { cursor.checked_add(1).filter(|next| *next < self.glasses.history.len()) } else { cursor.checked_sub(1) } {
            cursor = next;
            let place = self.glasses.history[cursor].clone();
            self.glasses.restoring = true;
            let restored = self.restore_place(&place);
            self.glasses.restoring = false;
            if restored {
                self.glasses.history_cursor = cursor;
                self.glasses.save();
                self.flash(if forward { "Forward" } else { "Back" });
                return;
            }
        }
        self.flash(if forward { "No next place" } else { "No previous place" });
    }

    pub(in crate::ui) fn navigation_key(&mut self, key: KeyEvent) -> bool {
        let Some(forward) = history_chord(key, self.keyboard_enhanced) else { return false };
        self.navigate_history(forward);
        true
    }

}

fn history_chord(key: KeyEvent, enhanced: bool) -> Option<bool> {
    if key.kind != KeyEventKind::Press || key.modifiers != KeyModifiers::CONTROL { return None; }
    match key.code {
        KeyCode::Char('[') if enhanced => Some(false),
        KeyCode::Char(']') if enhanced => Some(true),
        // Legacy 0x1d is unambiguous, unlike 0x1b (Escape/Ctrl+[).
        KeyCode::Char('5') if !enhanced => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_chords_are_disjoint_from_escape_attach_and_plain_tabs() {
        for enhanced in [false, true] {
            for code in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char('['), KeyCode::Char(']')] {
                assert_eq!(history_chord(KeyEvent::new(code, KeyModifiers::NONE), enhanced), None);
            }
            assert_eq!(history_chord(KeyEvent::new(KeyCode::Esc, KeyModifiers::CONTROL), enhanced), None);
            assert_eq!(history_chord(KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL), enhanced), enhanced.then_some(false));
            assert_eq!(history_chord(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL), enhanced), enhanced.then_some(true));
            assert_eq!(history_chord(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL), enhanced), (!enhanced).then_some(true));
        }
    }

    #[test]
    fn history_restores_content_and_location_and_ignores_focus_only() {
        let mut ui = Ui::new(demo::test_world());
        let a = Pane::Agent(Some(ui.world.agents.items()[0].id.clone()));
        let b = Pane::Agent(Some(ui.world.agents.items()[1].id.clone()));
        ui.open_in_glass(a.clone(), Open::Tab);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(b.clone(), Open::Right);
        let count = ui.glasses.history.len();
        ui.focus_group(first.group);
        assert_eq!(ui.glasses.history.len(), count);
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(a));
        assert_eq!(ui.current_place(), Some(first));
        ui.navigate_history(true);
        assert_eq!(ui.focused_pane(), Some(b));
    }

    #[test]
    fn history_skips_removed_subjects_and_caps_at_fifty() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::List(0), Open::Tab);
        ui.open_in_glass(Pane::Agent(Some("agent/removed".into())), Open::Tab);
        ui.open_in_glass(Pane::List(2), Open::Tab);
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(Pane::List(0)));
        for n in 0..60 {
            ui.open_in_glass(Pane::Document(format!("doc/history-{n}")), Open::Tab);
        }
        assert_eq!(ui.glasses.history.len(), 50);
    }

    #[test]
    fn history_persists_cursor_and_loads_old_workspaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        let mut ui = Ui::new(demo::test_world());
        ui.glasses = Glasses::open(None, Some(path.clone()));
        ui.open_in_glass(Pane::List(1), Open::Tab);
        ui.open_in_glass(Pane::List(2), Open::Tab);
        ui.navigate_history(false);
        let reopened = Glasses::open(None, Some(path.clone()));
        assert_eq!(reopened.history, ui.glasses.history);
        assert_eq!(reopened.history_cursor, ui.glasses.history_cursor);
        let mut old = serde_json::to_value(glass_store::load(&path)).unwrap();
        old["version"] = 2.into();
        old.as_object_mut().unwrap().remove("history");
        old.as_object_mut().unwrap().remove("history_cursor");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let reopened = Glasses::open(None, Some(path));
        assert!(reopened.history.is_empty());
        assert_eq!(reopened.glass().layout, ui.glasses.glass().layout);
    }

    #[test]
    fn enhanced_and_legacy_history_keys_pass_through_attached_terminals() {
        for enhanced in [false, true] {
            let mut ui = Ui::new(demo::test_world());
            ui.keyboard_enhanced = enhanced;
            let id = ui.world.agents.items()[0].id.clone();
            ui.open_in_glass(Pane::Agent(Some(id)), Open::Tab);
            ui.click(Hit::Attach);
            assert!(ui.terminal_focused());
            ui.effects.clear();
            for code in [KeyCode::Char('['), KeyCode::Char(']'), KeyCode::Char('5')] {
                let key = KeyEvent::new(code, KeyModifiers::CONTROL);
                ui.key(key);
                assert!(matches!(ui.effects.last(), Some(Effect::TerminalKey(sent)) if *sent == key));
            }
        }
    }
}
