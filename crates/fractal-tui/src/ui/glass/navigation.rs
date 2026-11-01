//! Device-local content history shared by back/forward and the recent-place switcher.
use super::*;
use glass_store::StoredPlace;

impl Ui {
    pub(in crate::ui) fn prune_place_history(&mut self) {
        let mut history = std::mem::take(&mut self.glasses.history);
        let old_cursor = self.glasses.history_cursor;
        let mut retained: usize = 0;
        let mut index = 0;
        history.retain(|place| {
            let keep = self.place_exists(place);
            if keep && index <= old_cursor {
                retained += 1;
            }
            index += 1;
            keep
        });
        self.glasses.history = history;
        self.glasses.history_cursor = retained.saturating_sub(1);
        self.glasses.save();
    }
    pub(in crate::ui) fn current_place(&self) -> Option<StoredPlace> {
        let glass = self.glasses.all.get(self.glasses.shown)?;
        let groups = glass.layout.groups();
        let group = groups.get(glass.focus)?;
        let pane = group
            .tabs
            .get(group.current)
            .map_or_else(|| Pane::List(0).key(), |tab| tab.pane.clone());
        let selection = (pane == Pane::List(0).key())
            .then(|| self.selected_id())
            .flatten();
        Some(StoredPlace {
            space: glass.id.clone(),
            group: glass.focus,
            tab: group.current,
            group_id: group.id.clone(),
            tab_id: group.tabs.get(group.current).map(|tab| tab.id.clone()),
            pane,
            selection,
            visited: 0,
        })
    }

    pub(in crate::ui) fn record_place_change(&mut self, before: Option<StoredPlace>) {
        if self.glasses.restoring {
            return;
        }
        let Some(mut after) = self.current_place() else {
            return;
        };
        if before.as_ref() == Some(&after) {
            return;
        }
        let glasses = &mut self.glasses;
        // A higher-level open may surround show_in; record that destination only once.
        if glasses
            .history
            .get(glasses.history_cursor)
            .is_some_and(|place| same_place(place, &after))
        {
            return;
        }
        let mut visited = glasses
            .history
            .iter()
            .map(|place| place.visited)
            .max()
            .unwrap_or(0);
        glasses
            .history
            .truncate(glasses.history_cursor.saturating_add(1));
        if let Some(mut before) = before
            && !glasses
                .history
                .last()
                .is_some_and(|place| same_place(place, &before))
        {
            visited += 1;
            before.visited = visited;
            glasses.history.push(before);
        }
        after.visited = visited + 1;
        glasses.history.push(after);
        if glasses.history.len() > 50 {
            glasses.history.drain(..glasses.history.len() - 50);
        }
        glasses.history_cursor = glasses.history.len() - 1;
        glasses.save();
    }

    fn place_location(&self, place: &StoredPlace) -> Option<(usize, usize, Option<usize>)> {
        let space = self
            .glasses
            .all
            .iter()
            .position(|glass| glass.id == place.space)?;
        let groups = self.glasses.all[space].layout.groups();
        match &place.tab_id {
            Some(id) => groups.iter().enumerate().find_map(|(group, tabs)| {
                tabs.tabs
                    .iter()
                    .position(|tab| &tab.id == id)
                    .map(|tab| (space, group, Some(tab)))
            }),
            None if place.pane == Pane::List(0).key() => groups
                .iter()
                .position(|group| group.id == place.group_id)
                .map(|group| (space, group, None)),
            None => None,
        }
    }

    pub(in crate::ui) fn place_exists(&self, place: &StoredPlace) -> bool {
        self.place_location(place).is_some() && self.place_content_exists(place)
    }

    fn place_content_exists(&self, place: &StoredPlace) -> bool {
        if let Some(selection) = &place.selection
            && self.world.attention.ready().is_some()
            && !self
                .world
                .attention
                .items()
                .iter()
                .any(|item| &item.id == selection)
        {
            return false;
        }
        match Pane::parse(&place.pane) {
            // Only agent IDs have an authoritative inventory here. Conversations
            // include saved sessions, and their partial cache cannot prove deletion.
            Some(Pane::Agent(Some(id))) if id.starts_with("agent/") => {
                self.world.agents.ready().is_none()
                    || self.world.agents.items().iter().any(|agent| agent.id == id)
            }
            Some(Pane::Mission(Some(id)) | Pane::Declaration(Some(id))) => {
                self.world.missions.ready().is_none()
                    || self
                        .world
                        .missions
                        .items()
                        .iter()
                        .any(|mission| mission.id == id)
            }
            Some(Pane::Home(Some(id))) => {
                self.world.attention.ready().is_none()
                    || self
                        .world
                        .attention
                        .items()
                        .iter()
                        .any(|item| item.id == id)
            }
            Some(Pane::Document(reference)) => !self.documents.known_missing(&reference),
            Some(_) => true,
            None => false,
        }
    }

    pub(in crate::ui) fn restore_place(&mut self, place: &StoredPlace) -> bool {
        let Some((space, split, tab)) = self.place_location(place) else {
            return false;
        };
        if !self.place_content_exists(place) {
            return false;
        }
        let before = self.current_place();
        let was_restoring = self.glasses.restoring;
        self.glasses.restoring = true;
        self.glasses.shown = space;
        self.glasses.home = false;
        self.glasses.palette = None;
        self.glasses.sidebar.focused = false;
        self.glasses.zoomed = false;
        let glass = self.glasses.glass_mut();
        glass.focus = split;
        let group = glass.layout.group_mut(split).unwrap();
        // An empty workspace's Inbox has no physical tab yet. Restoring it must not
        // overwrite the first tab subsequently opened in that group.
        let index = tab
            .or_else(|| group.tabs.iter().position(|tab| tab.pane == place.pane))
            .unwrap_or_else(|| {
                group.tabs.push(Tab::pane(&place.pane));
                group.tabs.len() - 1
            });
        group.current = index;
        let changed = group.tabs[index].pane != place.pane || tab.is_none();
        group.tabs[index].pane.clone_from(&place.pane);
        let id = glass.id.clone();
        if changed {
            self.glass_changed(&id);
        }
        self.show_focused();
        if let Some(selection) = &place.selection {
            self.focus_pane(&Pane::Home(Some(selection.clone())));
        }
        self.attach_when = None;
        self.glasses.restoring = was_restoring;
        self.record_place_change(before);
        self.glasses.save();
        true
    }

    pub(in crate::ui) fn navigate_history(&mut self, forward: bool) {
        let mut cursor = self.glasses.history_cursor;
        while let Some(next) = if forward {
            cursor
                .checked_add(1)
                .filter(|next| *next < self.glasses.history.len())
        } else {
            cursor.checked_sub(1)
        } {
            cursor = next;
            let place = self.glasses.history[cursor].clone();
            self.glasses.restoring = true;
            let restored = self.restore_place(&place);
            self.glasses.restoring = false;
            if restored {
                self.glasses.history_cursor = cursor;
                self.glasses.history[cursor].visited = self
                    .glasses
                    .history
                    .iter()
                    .map(|place| place.visited)
                    .max()
                    .unwrap_or(0)
                    + 1;
                self.glasses.save();
                self.flash(if forward { "Forward" } else { "Back" });
                return;
            }
        }
        self.flash(if forward {
            "No next place"
        } else {
            "No previous place"
        });
    }

    pub(in crate::ui) fn navigation_key(&mut self, key: KeyEvent) -> bool {
        if self.glasses.palette.is_some() {
            return false;
        }
        let Some(forward) = history_chord(key, self.keyboard_enhanced) else {
            return false;
        };
        self.navigate_history(forward);
        true
    }

    pub(in crate::ui) fn recent_places(&self) -> Vec<StoredPlace> {
        let mut places = Vec::new();
        let mut seen = HashSet::new();
        let mut history: Vec<_> = self.glasses.history.iter().enumerate().collect();
        history.sort_by_key(|(index, place)| std::cmp::Reverse((place.visited, *index)));
        for place in self
            .current_place()
            .into_iter()
            .chain(history.into_iter().map(|(_, place)| place.clone()))
        {
            let identity = place.selection.as_ref().map_or_else(
                || place.pane.clone(),
                |id| Pane::Home(Some(id.clone())).key(),
            );
            if seen.insert(identity) && self.place_exists(&place) {
                places.push(place);
            }
        }
        places
    }

    pub(in crate::ui) fn place_label(&self, place: &StoredPlace) -> String {
        let space = self
            .glasses
            .all
            .iter()
            .find(|glass| glass.id == place.space)
            .map_or("", |glass| glass.name.as_str());
        let pane = if let Some(selection) = &place.selection {
            Pane::Home(Some(selection.clone()))
        } else {
            let Some(pane) = Pane::parse(&place.pane) else {
                return space.into();
            };
            pane
        };
        let tab = Tab {
            id: String::new(),
            title: None,
            pane: place.pane.clone(),
        };
        let badge = self.tab_badge(&tab, Some(&pane));
        let attention = if badge.ask {
            "?"
        } else if badge.needs_person {
            "◆"
        } else {
            ""
        };
        format!(
            "{} {}{} · {}",
            badge.activity,
            self.pane_title(&pane),
            attention,
            space
        )
    }
}

fn same_place(left: &StoredPlace, right: &StoredPlace) -> bool {
    left.space == right.space
        && left.group_id == right.group_id
        && left.tab_id == right.tab_id
        && left.pane == right.pane
        && left.selection == right.selection
}

fn history_chord(key: KeyEvent, enhanced: bool) -> Option<bool> {
    if key.kind != KeyEventKind::Press || key.modifiers != KeyModifiers::CONTROL {
        return None;
    }
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
            for code in [
                KeyCode::Esc,
                KeyCode::Enter,
                KeyCode::Char('['),
                KeyCode::Char(']'),
            ] {
                assert_eq!(
                    history_chord(KeyEvent::new(code, KeyModifiers::NONE), enhanced),
                    None
                );
            }
            assert_eq!(
                history_chord(KeyEvent::new(KeyCode::Esc, KeyModifiers::CONTROL), enhanced),
                None
            );
            assert_eq!(
                history_chord(
                    KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL),
                    enhanced
                ),
                enhanced.then_some(false)
            );
            assert_eq!(
                history_chord(
                    KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
                    enhanced
                ),
                enhanced.then_some(true)
            );
            assert_eq!(
                history_chord(
                    KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL),
                    enhanced
                ),
                (!enhanced).then_some(true)
            );
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
    fn saved_session_history_survives_reopen_prune_and_back_forward() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        let mut ui = Ui::new(demo::test_world());
        ui.glasses = Glasses::open(None, Some(path.clone()));
        let session = Pane::Agent(Some("session/saved-omp".into()));
        assert!(
            !ui.world
                .agents
                .items()
                .iter()
                .any(|agent| agent.id == "session/saved-omp")
        );
        ui.open_in_glass(session.clone(), Open::Tab);
        let saved = ui.current_place().unwrap();
        let document = Pane::Document("doc/next".into());
        ui.open_in_glass(document.clone(), Open::Tab);
        let next = ui.current_place().unwrap();
        let history = ui.glasses.history.clone();

        let mut ui = Ui::new(demo::test_world());
        ui.glasses = Glasses::open(None, Some(path));
        ui.prune_place_history();
        assert_eq!(ui.glasses.history, history);
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(session));
        assert_eq!(ui.current_place(), Some(saved));
        ui.navigate_history(true);
        assert_eq!(ui.focused_pane(), Some(document));
        assert_eq!(ui.current_place(), Some(next));
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
                assert!(
                    matches!(ui.effects.last(), Some(Effect::TerminalKey(sent)) if *sent == key)
                );
            }
        }
    }

    #[test]
    fn opening_inbox_cards_records_the_selection_without_changing_the_list() {
        let mut ui = Ui::new(demo::test_world());
        let ids: Vec<_> = ui
            .world
            .attention
            .items()
            .iter()
            .take(2)
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(ids.len(), 2);
        ui.open_in_glass(Pane::Home(Some(ids[0].clone())), Open::Here);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(Pane::Home(Some(ids[1].clone())), Open::Here);
        ui.navigate_history(false);
        assert_eq!(ui.current_place(), Some(first));
        assert_eq!(ui.selected_id().as_ref(), Some(&ids[0]));
        assert_eq!(ui.focused_pane(), Some(Pane::List(0)));
    }

    #[test]
    fn history_crosses_spaces_and_prunes_closed_tabs_and_missing_subjects() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::List(1), Open::Tab);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(Pane::List(2), Open::Glass);
        let second = ui.current_place().unwrap();
        assert_ne!(first.space, second.space);
        ui.navigate_history(false);
        assert_eq!(ui.current_place(), Some(first));
        ui.navigate_history(true);
        assert_eq!(ui.current_place(), Some(second));
        ui.open_in_glass(Pane::Agent(Some("agent/removed".into())), Open::Tab);
        ui.prune_place_history();
        assert!(
            !ui.glasses
                .history
                .iter()
                .any(|place| place.pane.contains("agent/removed"))
        );
        ui.close_tab();
        ui.navigate_history(false);
        assert!(!ui.current_place().unwrap().pane.contains("agent/removed"));
    }

    #[test]
    fn history_skips_documents_observed_missing_but_not_unavailable_hosts() {
        use fractal_ui_model::documents::Unavailable;
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/kept".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/deleted".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/current".into()), Open::Tab);
        ui.documents.got("doc/deleted", Err(Unavailable::NotFound));
        ui.documents
            .got("doc/kept", Err(Unavailable::RemoteUnavailable));
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(Pane::Document("doc/kept".into())));
    }

    #[test]
    fn reordering_tabs_and_inserting_splits_never_overwrites_other_content() {
        let mut ui = Ui::new(demo::test_world());
        let a = Pane::Document("doc/a".into());
        let b = Pane::Document("doc/b".into());
        let c = Pane::Document("doc/c".into());
        ui.open_in_glass(a.clone(), Open::Tab);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(b.clone(), Open::Tab);
        ui.drop_tab((0, 0), Drop::Strip { group: 0, index: 2 });
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(b.clone()));
        assert_eq!(ui.glasses.glass().layout.groups()[0].tabs[1].pane, a.key());
        assert!(ui.restore_place(&first));
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(a, Open::Tab);
        ui.open_in_glass(b.clone(), Open::Right);
        let second = ui.current_place().unwrap();
        ui.focus_group(0);
        ui.open_in_glass(c.clone(), Open::Right);
        assert!(ui.restore_place(&second));
        assert_eq!(ui.focused_pane(), Some(b));
        assert!(ui.glasses.glass().layout.find(&c.key()).is_some());
    }

    #[test]
    fn back_restores_a_moved_tab_in_its_destination_after_its_group_is_removed() {
        let mut ui = Ui::new(demo::test_world());
        let a = Pane::Document("doc/a".into());
        let b = Pane::Document("doc/b".into());
        ui.open_in_glass(a.clone(), Open::Tab);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(b.clone(), Open::Right);
        let destination = ui.current_place().unwrap().group_id;
        ui.drop_tab((0, 0), Drop::Strip { group: 1, index: 1 });
        assert_eq!(ui.glasses.glass().layout.groups().len(), 1);
        ui.prune_place_history();
        assert!(
            ui.glasses
                .history
                .iter()
                .any(|place| same_place(place, &first))
        );
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(b.clone()));
        ui.navigate_history(false);
        assert_eq!(ui.focused_pane(), Some(a));
        let restored = ui.current_place().unwrap();
        assert_eq!(restored.tab_id, first.tab_id);
        assert_eq!(restored.group_id, destination);
        assert_eq!(restored.group, 0);
        assert_eq!(restored.tab, 1);
        ui.navigate_history(true);
        assert_eq!(ui.focused_pane(), Some(b));
    }

    #[test]
    fn remote_layout_reorders_keep_surviving_history_identities() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Right);
        let before = ui.current_place().unwrap();
        let glass = ui.glasses.glass_mut();
        let mut layout = glass.layout.clone();
        layout.split_first(0, Side::Right, Group::of(Tab::pane("document:doc/c")));
        glass.take(
            GlassBody {
                name: glass.name.clone(),
                layout: to_wire(&layout),
            },
            None,
        );
        assert!(ui.restore_place(&before));
        assert_eq!(ui.current_place().unwrap().tab_id, before.tab_id);
        assert_eq!(ui.focused_pane(), Some(Pane::Document("doc/b".into())));
    }

    #[test]
    fn sidebar_drag_open_records_the_content_transition() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Tab);
        let before = ui.current_place().unwrap();
        let id = ui.world.agents.items()[0].id.clone();
        ui.drop_subject(
            &id,
            Drop::Edge {
                group: 0,
                side: Side::Right,
                first: false,
            },
        );
        assert_eq!(ui.focused_pane(), Some(Pane::Agent(Some(id))));
        ui.navigate_history(false);
        assert_eq!(ui.current_place(), Some(before));
    }

    #[test]
    fn closing_the_last_index_space_keeps_the_active_location_valid() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Glass);
        assert_eq!(ui.glasses.shown, 1);
        ui.open_palette(Some(GLASSES), Open::Here);
        let palette = ui.glasses.palette.as_ref().unwrap();
        let close = ui
            .matches(palette)
            .iter()
            .position(|choice| choice.action == Action::CloseGlass)
            .unwrap();
        ui.open_choice(Some(close), Open::Here);
        assert_eq!(ui.glasses.shown, 0);
        assert_eq!(ui.glasses.all.len(), 1);
    }

    #[test]
    fn enter_keeps_sidebar_and_usage_ownership_over_a_detached_terminal() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Terminal("terminal/example-shell".into()), Open::Tab);
        ui.glasses.sidebar.shown = true;
        ui.glasses.sidebar.focused = true;
        ui.glasses.sidebar.section = 4;
        ui.key(KeyCode::Enter.into());
        assert!(ui.terminal.is_none());
        ui.glasses.sidebar.focused = false;
        ui.glasses.home = true;
        ui.glasses.usage = true;
        ui.key(KeyCode::Enter.into());
        assert!(ui.terminal.is_none());
    }

    #[test]
    fn automatic_stops_ignore_focus_only_and_keep_content_identity() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        let first = ui.current_place().unwrap();
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Right);
        let count = ui.glasses.history.len();
        ui.focus_group(first.group);
        assert_eq!(ui.glasses.history.len(), count);
        assert_eq!(ui.current_place(), Some(first));
        for n in 0..60 {
            ui.open_in_glass(Pane::Document(format!("doc/history-{n}")), Open::Tab);
        }
        assert_eq!(ui.glasses.history.len(), 50);
        assert_eq!(ui.glasses.history_cursor, 49);
    }

    #[test]
    fn automatic_history_persists_and_closed_tabs_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        let mut ui = Ui::new(demo::test_world());
        ui.glasses = Glasses::open(None, Some(path.clone()));
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Tab);
        let reopened = Glasses::open(None, Some(path));
        assert_eq!(reopened.history, ui.glasses.history);
        assert_eq!(reopened.history_cursor, ui.glasses.history_cursor);
        let closed = ui.current_place().unwrap().tab_id;
        ui.close_tab();
        assert!(ui.glasses.history.iter().all(|place| place.tab_id != closed));
    }

    #[test]
    fn persisted_history_follows_tabs_moved_to_another_group_strip() {
        for keep_source_group in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("glasses.json");
            let mut ui = Ui::new(demo::test_world());
            ui.glasses = Glasses::open(None, Some(path.clone()));
            ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
            let source = ui.current_place().unwrap();
            if keep_source_group {
                ui.open_in_glass(Pane::Document("doc/stays".into()), Open::Tab);
                ui.show_in(source.group, source.tab);
            }
            // Different content stops belong to the same stable tab, not its latest pane.
            ui.swap_focused_pane(Pane::Document("doc/b".into()));
            ui.swap_focused_pane(Pane::List(0));
            let home = ui.current_place().unwrap();
            ui.open_in_glass(Pane::Document("doc/destination".into()), Open::Right);
            let destination = ui.current_place().unwrap();
            assert_ne!(source.group_id, destination.group_id);
            assert_eq!(ui.glasses.glass().layout.groups().len(), 2);
            let before_move = Glasses::open(None, Some(path.clone()));
            assert_eq!(before_move.history, ui.glasses.history);

            ui.drop_tab(
                (source.group, source.tab),
                Drop::Strip {
                    group: destination.group,
                    index: 0,
                },
            );
            let moved = ui.current_place().unwrap();
            assert_eq!(moved.tab_id, source.tab_id);
            assert_eq!(moved.group_id, destination.group_id);
            assert_eq!(moved.group, usize::from(keep_source_group));
            assert_eq!(moved.tab, 0);
            assert_eq!(
                ui.glasses.glass().layout.groups().len(),
                if keep_source_group { 2 } else { 1 },
            );

            let expected = ui
                .glasses
                .history
                .iter()
                .filter(|place| place.tab_id == source.tab_id)
                .cloned()
                .map(|mut place| {
                    place.group = moved.group;
                    place.group_id.clone_from(&moved.group_id);
                    place.tab = moved.tab;
                    place
                })
                .collect::<Vec<_>>();
            assert!(
                expected
                    .iter()
                    .any(|place| place.pane == Pane::Document("doc/a".into()).key())
            );
            assert!(
                expected
                    .iter()
                    .any(|place| place.pane == Pane::Document("doc/b".into()).key())
            );
            assert!(
                expected
                    .iter()
                    .any(|place| { place.pane == home.pane && place.selection == home.selection })
            );
            let reopened = Glasses::open(None, Some(path));
            let actual = reopened
                .history
                .iter()
                .filter(|place| place.tab_id == source.tab_id)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
            assert_eq!(
                reopened.history[reopened.history_cursor],
                ui.glasses.history[ui.glasses.history_cursor],
            );
        }
    }

    #[test]
    fn legacy_control_five_keeps_palette_section_ownership() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Tab);
        ui.navigate_history(false);
        let before = ui.current_place();
        ui.open_palette(None, Open::Here);
        ui.key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert_eq!(ui.glasses.palette.as_ref().unwrap().section, Some(4));
        assert_eq!(ui.current_place(), before);
        ui.key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert_eq!(ui.glasses.palette.as_ref().unwrap().section, None);
        assert_eq!(ui.current_place(), before);
    }
}
