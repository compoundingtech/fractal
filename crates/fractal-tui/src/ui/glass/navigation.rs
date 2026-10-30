//! Device-local content destinations recorded automatically as the view changes.
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
        })
    }

    pub(in crate::ui) fn record_place_change(&mut self, before: Option<StoredPlace>) {
        let Some(after) = self.current_place() else {
            return;
        };
        if before.as_ref() == Some(&after) {
            return;
        }
        let glasses = &mut self.glasses;
        // A higher-level open may surround show_in; record that destination only once.
        if glasses.history.get(glasses.history_cursor) == Some(&after) {
            return;
        }
        glasses
            .history
            .truncate(glasses.history_cursor.saturating_add(1));
        if let Some(before) = before {
            if glasses.history.last() != Some(&before) {
                glasses.history.push(before);
            }
        }
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
        let group = groups.iter().position(|group| group.id == place.group_id)?;
        let tab = match &place.tab_id {
            Some(id) => Some(groups[group].tabs.iter().position(|tab| &tab.id == id)?),
            None if place.pane == Pane::List(0).key() => None,
            None => return None,
        };
        Some((space, group, tab))
    }

    pub(in crate::ui) fn place_exists(&self, place: &StoredPlace) -> bool {
        self.place_location(place).is_some()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn remote_layout_reorders_keep_surviving_history_identities() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Right);
        let before = ui.current_place().unwrap();
        let glass = ui.glasses.glass_mut();
        let mut layout = glass.layout.clone();
        layout.split_first(0, Side::Right, Group::of(Tab::pane("document:doc/c")));
        glass.take(GlassBody { name: glass.name.clone(), layout: to_wire(&layout) }, None);
        assert!(ui.place_exists(&before));
        assert_eq!(ui.current_place().unwrap().tab_id, before.tab_id);
        assert_eq!(ui.focused_pane(), Some(Pane::Document("doc/b".into())));
    }
}
