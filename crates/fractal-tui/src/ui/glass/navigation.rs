//! Device-local content destinations recorded automatically as the view changes.
use super::*;
use glass_store::StoredPlace;

impl Ui {
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
        if let Some(before) = before
            && glasses.history.last() != Some(&before)
        {
            glasses.history.push(before);
        }
        glasses.history.push(after);
        if glasses.history.len() > 50 {
            glasses.history.drain(..glasses.history.len() - 50);
        }
        glasses.history_cursor = glasses.history.len() - 1;
        glasses.save();
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
    fn automatic_history_persists_with_its_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        let mut ui = Ui::new(demo::test_world());
        ui.glasses = Glasses::open(None, Some(path.clone()));
        ui.open_in_glass(Pane::Document("doc/a".into()), Open::Tab);
        ui.open_in_glass(Pane::Document("doc/b".into()), Open::Tab);
        let reopened = Glasses::open(None, Some(path));
        assert_eq!(reopened.history, ui.glasses.history);
        assert_eq!(reopened.history_cursor, ui.glasses.history_cursor);
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
        assert_eq!(ui.current_place().unwrap().tab_id, before.tab_id);
        assert_eq!(ui.focused_pane(), Some(Pane::Document("doc/b".into())));
    }
}
