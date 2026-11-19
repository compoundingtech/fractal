//! Semantic body copy shares the painted document with drag selection.
use super::*;
use std::ops::Range;

impl FramePane {
    fn body_text(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.lines.len());
        if range.start >= end {
            return String::new();
        }
        Selection {
            pane: self.key.clone(),
            anchor: (range.start, 0),
            head: (end - 1, u16::MAX),
        }
        .text_with_link_targets(&self.lines)
    }

    fn entry_range(&self, line: usize) -> Option<Range<usize>> {
        let index = self.entries.iter().rposition(|(_, start)| *start <= line)?;
        Some(self.entries[index].1..self.entries.get(index + 1).map_or(self.lines.len(), |(_, start)| *start))
    }
    pub(super) fn pointed_body(&self, line: usize) -> String {
        let range = self.copy_blocks.iter()
            .filter(|range| range.contains(&line))
            .min_by_key(|range| range.end - range.start)
            .cloned()
            .or_else(|| self.entry_range(line))
            .unwrap_or(0..self.lines.len());
        self.body_text(range)
    }
}

impl Ui {
    fn focused_body_text(&self) -> Option<String> {
        let key = Some(self.main_pane().key());
        let frame = self.frame.borrow();
        let pane = frame.panes.iter().rev().find(|pane| {
            key.as_ref().is_some_and(|key| &pane.key == key)
        }).or_else(|| frame.panes.iter().rev().find(|pane| {
            key.as_ref().and_then(|key| key.strip_prefix("agent:"))
                .is_some_and(|agent| pane.key == format!("chat:{agent}"))
        }))?;
        let range = pane.key.strip_prefix("chat:")
            .and_then(|agent| self.content.focused.get(agent))
            .and_then(|id| pane.entries.iter().find(|(entry, _)| entry == id))
            .and_then(|(_, line)| pane.entry_range(*line))
            .or_else(|| pane.copy_blocks.iter().max_by_key(|range| range.end - range.start).cloned())
            .unwrap_or(0..pane.lines.len());
        let text = pane.body_text(range);
        (!text.trim().is_empty()).then_some(text)
    }

    fn copy_body_key(&mut self, key: KeyEvent) -> bool {
        if key.code != KeyCode::Char('y') || !key.modifiers.is_empty()
            || (matches!(self.tab(), 0 | 2) && self.attention_focus().is_some()
                && (self.current_kind() == Some("request")
                    || self.attention_focus().is_some_and(|id| self.closed.contains(&id))
                    || self.current_item().is_some_and(|item|
                        item.actions.iter().any(|action| action == "custom.reply"))))
        {
            return false;
        }
        let Some(text) = self.focused_body_text() else { return false };
        self.copy_text(&text);
        true
    }

    pub(super) fn copy_text(&mut self, text: &str) {
        if text.trim().is_empty() {
            self.flash("No text to copy");
            return;
        }
        let result = send_clipboard(text, &mut io::stdout());
        self.flash(match result {
            Ok(()) => "Clipboard request sent (OSC 52) · terminal permission required; tmux needs set-clipboard on".to_owned(),
            Err(error) => format!("Clipboard request failed: {error}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Ui {
        let ui = Ui::new(demo::test_world());
        let key = ui.main_pane().key();
        let mut frame = ui.frame.borrow_mut();
        frame.area = Rect::new(0, 0, 40, 10);
        frame.panes.push(FramePane {
            key, rect: Rect::new(0, 1, 40, 4), top: 0, total: 4,
            lines: Rc::new(vec![
                Line::from("frame").decoration(),
                Line::from("first body"),
                Line::from("second body"),
                Line::from("frame").decoration(),
            ]),
            copy_blocks: vec![1..3], entries: Vec::new(), images: Vec::new(), layer: 0,
        });
        drop(frame);
        ui
    }

    #[test]
    fn focused_copy_and_point_menu_share_the_semantic_body() {
        let mut ui = fixture();
        let text = ui.focused_body_text().unwrap();
        assert_eq!(text, "first body\nsecond body");
        let context::PointMenu::Content((_, items)) = ui.point_menu(3, 2).unwrap() else { panic!("content menu") };
        assert!(items.iter().any(|(label, action)| label == "Copy text" && *action == glass::MenuAction::CopyText(text.clone())));
        ui.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(ui.flash.as_ref().unwrap().0.starts_with("Clipboard request sent"));
        ui.run_menu_action(glass::MenuAction::CopyText(text));
        assert!(ui.flash.as_ref().unwrap().0.contains("permission"));
    }

    #[test]
    fn y_types_in_an_existing_input_and_does_not_copy() {
        let mut ui = fixture();
        ui.editing = true;
        ui.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(ui.conversation_state.drafts.values().any(|draft| draft == "y"));
        assert!(ui.flash.is_none());
    }

    #[test]
    fn y_keeps_active_request_commands_even_without_rendered_keycaps() {
        let mut ui = Ui::new(demo::test_world());
        ui.open_home();
        let id = ui.world.attention.items().iter()
            .find(|item| matches!(item.kind, view::AttentionKind::Request { .. }))
            .expect("demo request").id.clone();
        let index = ui.listing(60).ids.iter().position(|item| item == &id).unwrap();
        ui.select(index);
        ui.frame.borrow_mut().hits.clear();
        ui.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(ui.confirm, Some('y'));
        ui.confirm = None;
        ui.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(ui.confirm, Some('n'));
        ui.confirm = None;
        ui.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(ui.editing || ui.answering.is_some());
        ui.editing = false;
        assert!(!ui.copy_body_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)));
        assert!(ui.flash.is_none());
    }

    #[test]
    fn pointed_nested_block_and_focused_outer_card_have_distinct_scopes() {
        let ui = fixture();
        ui.frame.borrow_mut().panes[0].copy_blocks.push(2..3);
        assert_eq!(ui.frame.borrow().panes[0].pointed_body(2), "second body");
        assert_eq!(ui.focused_body_text().as_deref(), Some("first body\nsecond body"));
    }

    #[test]
    fn full_width_card_action_waits_for_click_not_drag() {
        let mut ui = fixture();
        ui.frame.borrow_mut().hits.push((Rect::new(0, 2, 40, 1), Hit::Help));
        for (kind, x) in [(MouseEventKind::Down(MouseButton::Left), 0), (MouseEventKind::Drag(MouseButton::Left), 4), (MouseEventKind::Up(MouseButton::Left), 4)] {
            ui.mouse(MouseEvent { kind, column: x, row: 2, modifiers: KeyModifiers::NONE });
        }
        assert_eq!(ui.selected_text().as_deref(), Some("first"));
        assert!(!ui.help);
        assert!(ui.pending_content_click.is_none());
        for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
            ui.mouse(MouseEvent { kind, column: 0, row: 2, modifiers: KeyModifiers::NONE });
        }
        assert!(ui.help, "a click without dragging still activates the card action");
    }

    #[test]
    fn clipboard_write_failure_is_propagated() {
        struct Failed;
        impl Write for Failed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::Error::other("closed")) }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        assert!(send_clipboard("body", &mut Failed).is_err());
    }
}
