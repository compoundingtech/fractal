//! Pointer instructions describe the painted target and use the menu's own resolver.
//!
//! In the TUI, middle-click closes or dismisses an item in any closable list.
//! Destructive actions and answering decisions are excluded.

use super::context::PointMenu;
use super::*;

#[derive(PartialEq, Eq)]
pub(super) enum CloseTarget {
    Tab(usize, usize),
    InboxRow(String),
    InboxCard(String),
}

pub(super) enum MiddleClose<T> {
    Ignored,
    Consumed,
    Released(T),
}

/// A closable list owns its middle press through release, including drags outside it.
pub(super) fn middle_close<T: PartialEq>(
    pressed: &mut Option<T>,
    kind: MouseEventKind,
    target: Option<T>,
) -> MiddleClose<T> {
    match kind {
        MouseEventKind::Down(button) => {
            *pressed = if button == MouseButton::Middle { target } else { None };
            if pressed.is_some() { MiddleClose::Consumed } else { MiddleClose::Ignored }
        }
        MouseEventKind::Drag(MouseButton::Middle) if pressed.is_some() => MiddleClose::Consumed,
        MouseEventKind::Up(MouseButton::Middle) => match pressed.take() {
            Some(pressed) if target.as_ref() == Some(&pressed) => MiddleClose::Released(pressed),
            Some(_) => MiddleClose::Consumed,
            None => MiddleClose::Ignored,
        },
        _ => MiddleClose::Ignored,
    }
}

impl Ui {
    fn inbox_row_at<'a>(&self, frame: &'a FrameInfo, x: u16, y: u16) -> Option<&'a str> {
        frame.context_rows.iter().rev()
            .find(|(rect, _, _)| contains(*rect, x, y))
            .map(|(_, id, _)| id.as_str())
            .filter(|id| self.world.attention.items().iter().any(|item| item.id == *id))
    }

    /// Resolve the painted Inbox target, independently of keyboard focus or selection.
    pub(super) fn close_target(&self, x: u16, y: u16) -> Option<CloseTarget> {
        let frame = self.frame.borrow();
        let (_, hit) = frame.hits.iter().rev().find(|(rect, _)| contains(*rect, x, y))?;
        match hit {
            Hit::GlassTab(group, tab) => Some(CloseTarget::Tab(*group, *tab)),
            Hit::Row(_) => self.inbox_row_at(&frame, x, y)
                .map(|id| CloseTarget::InboxRow(id.to_owned())),
            Hit::Key('x') => frame.panes.iter().rev()
                .find(|pane| contains(pane.rect, x, y))
                .and_then(|pane| pane.key.strip_prefix("home:"))
                .map(|id| CloseTarget::InboxCard(id.to_owned())),
            _ => None,
        }
    }

    pub(super) fn dismiss_inbox(&mut self, id: String) {
        let Some(item) = self.world.attention.items().iter().find(|item| item.id == id) else {
            return;
        };
        if item.actions.iter().any(|action| action == "custom.reply") {
            self.flash("Reply with c using this source's declared fields");
            return;
        }
        if self.closed.remove(&id) {
            self.acted.insert(id.clone());
            if let Load::Ready(items) = &mut self.world.attention {
                items.retain(|item| item.id != id);
            }
            if self.tab() == 0 {
                self.select(self.selected[0]);
            }
            return;
        }
        match item.kind.word() {
            "request" => self.act_attention(id, 'r'),
            "update" => self.read_update_id(id),
            "message" => self.act_attention(id, 'm'),
            "fault" => self.flash(
                "A fault clears when the agent that owns it retries or cancels; open it with g",
            ),
            word => self.flash(format!(
                "A {word} waits on your decision, so x cannot dismiss it: answer it with the keys on its card"
            )),
        }
    }

    pub(super) fn draw_click_hint(
        &self,
        buf: &mut Buffer,
        rect: Rect,
        hit: &Hit,
        point: (u16, u16),
    ) {
        // Status and pending decisions keep their footer. A floating menu may cover it.
        let area = self.frame.borrow().area.intersection(buf.area);
        if area.height < 2
            || area.width < 3
            || self.flash.is_some()
            || self.confirm.is_some()
            || matches!(self.world.link, Link::Offline(_))
        {
            return;
        }
        let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        // Text to read stays quiet: moving over messages leaves the normal keys in the footer
        // (Nathan, 2026-10-07).
        if matches!(hit, Hit::Message | Hit::Subject | Hit::InlineImage(_)) {
            return;
        }
        if rect.intersects(footer)
            || self
                .frame
                .borrow()
                .menu
                .is_some_and(|menu| menu.intersects(footer))
        {
            return;
        }
        let label = text::sanitize(&text::truncate(&self.target_label(buf, rect), 48));
        let mut left = match hit {
            Hit::Answer(_) => "choose answer; Enter sends".into(),
            Hit::Alert(_, AlertTap::Answer(_)) => "answer this alert".into(),
            Hit::Alert(_, AlertTap::Choose(..)) => "choose this answer".into(),
            Hit::Alert(_, AlertTap::Open) => "open this alert in Inbox".into(),
            Hit::Alert(_, AlertTap::Toggle) => "show or hide the alerts".into(),
            Hit::Voice => "start voice input".into(),
            Hit::NewTerminal => "new terminal".into(),
            Hit::ResourceRow(_) => "open resource or toggle group".into(),
            Hit::ResourceFilter => "filter sidebar".into(),
            Hit::ResourceNewFolder => "new sidebar folder".into(),
            Hit::ResourceAdd(_) => "find a resource to add".into(),
            Hit::ResourceFolderMenu(_) => "rename or delete folder".into(),
            Hit::ResourceFolderDrag(_) => "drag to reorder folder".into(),
            Hit::SidebarToggle => "show/hide sidebar · Ctrl+S".into(),
            Hit::SidebarRow(_) => "open item in tab".into(),
            Hit::SidebarSection(_) => format!("show {label}"),
            Hit::Usage => "show/hide Usage".into(),
            Hit::Connection => "connection details".into(),
            Hit::Row(_) => "select row".into(),
            Hit::Message | Hit::Subject | Hit::InlineImage(_) => String::new(),
            Hit::Resize => "drag resize; double-click equalize".into(),
            Hit::SidebarResize => "drag resize sidebar; double-click reset".into(),
            Hit::Key(_) | Hit::Availability(_) | Hit::Enter | Hit::ChatEnter | Hit::Escape | Hit::Menu(_) => label,
            Hit::ToggleTool(_) | Hit::Pane(PaneIntent::Expand(_)) => "expand/collapse".into(),
            Hit::Pane(PaneIntent::Open(_)) | Hit::Open(_) => "open subject".into(),
            Hit::Pane(PaneIntent::Image(_)) => "open image".into(),
            Hit::ContentImage(_) => "load image".into(),
            Hit::ContentOutput(_) => "load full output".into(),
            Hit::Pane(PaneIntent::Send(_)) => "send message".into(),
            Hit::Pane(PaneIntent::LoadOlder) => "load older messages".into(),
            Hit::JumpLatest => "jump to latest [End]".into(),
            Hit::Composer | Hit::ChatComposer => "edit message [c]".into(),
            Hit::Help => "show/hide help [F1]".into(),
            Hit::Peek(id) if self.popover.as_deref() == Some(id) && rect.height > 1 => {
                "card already open".into()
            }
            Hit::Peek(_) => "show card".into(),
            Hit::Actions(_) => "agent actions".into(),
            Hit::Field(_) => "focus field [Tab]".into(),
            Hit::Revoke(_) => "ask to revoke; y confirms".into(),
            Hit::Attach => "attach terminal [Enter on a detached terminal pane]".into(),
            Hit::Detach => "leave terminal [Ctrl+\\ c or Ctrl+T twice]".into(),
            Hit::GlassMenu => "spaces [Ctrl+G]".into(),
            Hit::PaletteSection(1) => "agents [Ctrl+2]".into(),
            Hit::PaletteSection(2) => "missions [Ctrl+3]".into(),
            Hit::PaletteSection(3) => "fleet [palette: fleet]".into(),
            Hit::PaletteSection(_) => format!("open {label}"),
            Hit::Home => "open Inbox [Ctrl+H]".into(),
            Hit::Link(url) if super::documents::openable(url).is_some() => "open document".into(),
            Hit::Link(_) => "copy link".into(),
            Hit::Document(super::documents::Tap::Back) => "back one document [b]".into(),
            Hit::Document(super::documents::Tap::Retry) => "read it again [r]".into(),
            Hit::Document(super::documents::Tap::More) => "more documents [m]".into(),
            Hit::Split(true) => "split right [Ctrl+V]".into(),
            Hit::Split(false) => "split below [Ctrl+X]".into(),
            Hit::GlassTab(..) => {
                "show tab; wheel scrolls tabs; drag to move/split; middle-click closes".into()
            }
            Hit::GlassTabMore(..) => {
                "more tabs: show the next hidden tab [Tab]; wheel scrolls without selecting".into()
            }
            Hit::GlassAdd(_) => "new tab [Ctrl+T]".into(),
            Hit::PaletteChoice(_) => format!("open {label}"),
        };
        // A first click in an unfocused split changes only focus (except its composer).
        if self.popover.is_none()
            && self.context.is_none()
            && !self.home_open()
            && !self.palette_open()
            && !matches!(hit, Hit::Composer | Hit::ChatComposer | Hit::Resize | Hit::SidebarResize)
            && self.click_focuses_split(point.0, point.1)
        {
            left = "focus split; click again to act".into();
        }
        let right = if self.context.is_some() {
            "close menu".into()
        } else {
            match self.point_menu(point.0, point.1) {
                Some(PointMenu::Content((title, _))) => {
                    format!("{} menu", text::truncate(&text::sanitize(&title), 24))
                }
                Some(PointMenu::Glass) => "tab/split menu".into(),
                None => "no menu".into(),
            }
        };
        let middle = if match hit {
            Hit::Row(_) => self.inbox_row_at(&self.frame.borrow(), point.0, point.1).is_some(),
            Hit::Key('x') => self.frame.borrow().panes.iter().any(|pane| {
                contains(pane.rect, point.0, point.1) && pane.key.starts_with("home:")
            }),
            _ => false,
        } {
            " · Middle: dismiss"
        } else {
            ""
        };
        let build = self
            .build
            .then(|| format!(" {} ", crate::version::short(crate::version::now())));
        let reserved = build
            .as_deref()
            .map_or(0, |build| text::width(build) as u16 + 1);
        let width = area.width.saturating_sub(reserved + 2) as usize;
        // Keep both operations visible on narrow terminals, even when the labels are long.
        let allowance = width.saturating_sub(text::width("Click:  · Right: ") + text::width(middle)) / 2;
        let hint = format!(
            "Click: {} · Right: {}{middle}",
            text::truncate(&left, allowance),
            text::truncate(&right, allowance)
        );
        for x in footer.x..footer.right() {
            buf[(x, footer.y)]
                .set_symbol(" ")
                .set_style(theme::dim().bg(theme::CRUST));
        }
        buf.set_stringn(
            area.x + 1,
            footer.y,
            hint,
            width,
            theme::dim().bg(theme::CRUST),
        );
        if let Some(build) = build
            && reserved < area.width
        {
            buf.set_stringn(
                area.right() - reserved,
                footer.y,
                build,
                reserved as usize,
                theme::dim().bg(theme::CRUST),
            );
        }
    }

    fn target_label(&self, buf: &Buffer, rect: Rect) -> String {
        let rect = rect.intersection(buf.area);
        let mut label = String::new();
        let mut skip = 0;
        for x in rect.x..rect.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = buf[(x, rect.y)].symbol();
            label.push_str(symbol);
            skip = text::width(symbol).saturating_sub(1);
        }
        label.trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_button_disarms_a_middle_press() {
        let mut pressed = None;
        assert!(matches!(middle_close(&mut pressed, MouseEventKind::Down(MouseButton::Middle), Some(1)), MiddleClose::Consumed));
        assert!(matches!(middle_close(&mut pressed, MouseEventKind::Down(MouseButton::Left), Some(1)), MiddleClose::Ignored));
        assert!(matches!(middle_close(&mut pressed, MouseEventKind::Up(MouseButton::Middle), Some(1)), MiddleClose::Ignored));
    }

    fn draw(ui: &Ui) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(160, 48)).unwrap();
        for _ in 0..2 {
            terminal.draw(|frame| ui.render(frame)).unwrap();
        }
        terminal.backend().buffer().clone()
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn pointer(ui: &mut Ui, kind: MouseEventKind, x: u16, y: u16) {
        ui.input_event(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }

    fn target(ui: &Ui, matches: impl Fn(&Hit) -> bool) -> Rect {
        ui.frame
            .borrow()
            .hits
            .iter()
            .rev()
            .find(|(_, hit)| matches(hit))
            .unwrap()
            .0
    }

    #[test]
    fn pointer_footer_names_both_operations_and_keeps_status_and_keyboard_hints() {
        let mut ui = Ui::new(demo::test_world());
        ui.build = true;
        let before = draw(&ui);
        assert!(line(&before, 47).contains("Keys:"));
        let rect = target(&ui, |hit| matches!(hit, Hit::Connection));
        let focus = ui.focus();
        pointer(&mut ui, MouseEventKind::Moved, rect.x, rect.y);
        let buf = draw(&ui);
        let footer = line(&buf, 47);
        assert!(
            footer.contains("Click: connection details · Right: actions menu"),
            "{footer}"
        );
        assert!(
            !footer.contains("ctrl+w") && !footer.contains("↑↓"),
            "{footer}"
        );
        assert!(footer.contains(crate::version::short(crate::version::now()).as_str()));
        assert_eq!(ui.focus(), focus);
        assert!(ui.effects.is_empty());
        assert_eq!(
            buf[(rect.x, rect.y)].bg,
            theme::hover_background(before[(rect.x, rect.y)].bg)
        );
        ui.flash("A useful status");
        assert!(line(&draw(&ui), 47).contains("A useful status"));
        ui.flash = None;
        ui.world.link = Link::Offline("Connection interrupted".into());
        assert!(line(&draw(&ui), 47).contains("Connection interrupted"));
        ui.world.link = Link::Live;
        ui.confirm = Some('x');
        assert!(!line(&draw(&ui), 47).contains("Click:"));
        ui.confirm = None;
        ui.input_event(Event::FocusLost);
        assert!(line(&draw(&ui), 47).contains("Keys:"));
        let agent = ui.world.agents.items()[0].id.clone();
        ui.open(&agent);
        draw(&ui);
        let tab = target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 0)));
        pointer(&mut ui, MouseEventKind::Moved, tab.x, tab.y);
        assert!(line(&draw(&ui), 47).contains("Right: tab/split menu"));
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Right),
            tab.x,
            tab.y,
        );
        assert!(matches!(&ui.context.as_ref().unwrap().items[0].1,
            glass::MenuAction::Activate { hit, .. } if **hit == Hit::GlassTab(0, 0)));
    }

    #[test]
    fn unattached_terminal_button_hovers_and_runs_the_keyboard_attach_action() {
        for shell in [false, true] {
            let mut ui = Ui::new(demo::test_world());
            let id = if shell {
                "terminal/example-shell".to_owned()
            } else {
                ui.world.agents.items()[0].id.clone()
            };
            ui.open_in_glass(Pane::Terminal(id.clone()), glass::Open::Tab);
            let before = draw(&ui);
            assert!(ui.terminal.is_none());
            assert!((0..48).any(|row| line(&before, row).contains("Not attached. Enter attaches")));
            let button = target(&ui, |hit| matches!(hit, Hit::Attach));
            assert!(line(&before, button.y).contains("Attach"));
            pointer(&mut ui, MouseEventKind::Moved, button.x, button.y);
            let hovered = draw(&ui);
            assert_eq!(
                hovered[(button.x, button.y)].bg,
                theme::hover_background(before[(button.x, button.y)].bg)
            );
            assert!(line(&hovered, 47).contains("Click: attach terminal"));
            ui.live = true;
            ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            let keyboard = ui.effects.drain(..).collect::<Vec<_>>();
            pointer(&mut ui, MouseEventKind::Down(MouseButton::Left), button.x, button.y);
            assert!(matches!(keyboard.as_slice(), [Effect::OpenTerminal { agent }] if agent == &id));
            assert!(matches!(ui.effects.as_slice(), [Effect::OpenTerminal { agent }] if agent == &id));
        }
    }

    #[test]
    fn middle_click_closes_only_on_release_over_the_pressed_tab() {
        let mut ui = Ui::new(demo::test_world());
        let agent = ui.world.agents.items()[0].id.clone();
        ui.open(&agent);
        let other = ui.world.agents.items()[1].id.clone();
        ui.open(&other);
        draw(&ui);
        let tab = target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 1)));
        let home = target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 0)));
        pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), tab.x, tab.y);
        draw(&ui);
        assert_eq!(target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 1))), tab);
        pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), home.x, home.y);
        draw(&ui);
        assert_eq!(target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 1))), tab);
        pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), tab.x, tab.y);
        draw(&ui);
        assert_eq!(target(&ui, |hit| matches!(hit, Hit::GlassTab(0, 1))), tab);
        pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), tab.x, tab.y);
        pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), tab.x, tab.y);
        draw(&ui);
        assert!(!ui.frame.borrow().hits.iter()
            .any(|(_, hit)| matches!(hit, Hit::GlassTab(0, 1))));
    }

    fn inbox(kind: &str) -> (Ui, String) {
        let mut world = demo::test_world();
        let mut item = world.attention.items()[0].clone();
        item.id = "attention/middle-target".into();
        item.kind = match kind {
            "request" | "named" => AttentionKind::Request {
                from: "Planner".into(), from_id: "agent/example/planner".into(),
                question: "Choose a runner".into(), images: Vec::new(),
                structured: (kind == "named").then(|| Box::new(fractal_client::StructuredRequest {
                    version: 1, entry_type: "decision".into(), question: "Choose a runner".into(),
                    answers: vec![fractal_client::RequestAnswerOption {
                        id: "yes".into(), label: "Yes".into(), ..Default::default()
                    }], ..Default::default()
                })),
            },
            "update" => AttentionKind::Update {
                from: "Planner".into(), body: "Finished".into(), about: String::new(), subjects: Vec::new(),
            },
            "prompt" => AttentionKind::Prompt {
                seat: "Builder".into(), seat_id: "agent/example/builder".into(), text: "Allow?".into(),
                question: false, answers: Vec::new(), episode: String::new(), call: None,
                calls: Ok(Vec::new()), allow: false,
            },
            "closed" => item.kind.clone(),
            word => world.attention.items().iter().find(|item| item.kind.word() == word).unwrap().kind.clone(),
        };
        item.actions = vec!["work.done".into(), "message.read".into()];
        let id = item.id.clone();
        let mut other = item.clone();
        other.id = "attention/other".into();
        world.attention = Load::Ready(vec![item, other]);
        let mut ui = Ui::new(world);
        ui.live = true;
        ui.open_home();
        if kind == "closed" {
            ui.closed.insert(id.clone());
        }
        (ui, id)
    }

    #[test]
    fn middle_click_inbox_releases_only_over_the_pressed_item_for_every_kind() {
        for kind in ["request", "named", "update", "message", "closed", "review",
            "feedback", "launch", "revision", "prompt", "fault"] {
            let (mut ui, id) = inbox(kind);
            let index = ui.listing_for(0, 40).ids.iter().position(|candidate| candidate == &id).unwrap();
            ui.select(1 - index);
            draw(&ui);
            let row = target(&ui, |hit| *hit == Hit::Row(index));
            let other = target(&ui, |hit| *hit == Hit::Row(1 - index));
            pointer(&mut ui, MouseEventKind::Moved, row.x, row.y);
            let footer = line(&draw(&ui), 47);
            assert!(footer.contains("Middle: dismiss"), "{kind}: {footer}; close target: {:?}", ui.close_target(row.x, row.y).is_some());
            pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), row.x, row.y);
            assert!(ui.effects.is_empty() && ui.confirm.is_none() && ui.flash.is_none(), "{kind}");
            pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), other.x, other.y);
            pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), row.x, row.y);
            assert!(ui.effects.is_empty() && ui.flash.is_none(), "{kind}");
            pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), row.x, row.y);
            pointer(&mut ui, MouseEventKind::Drag(MouseButton::Middle), other.x, other.y);
            pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), row.x, row.y);
            assert!(ui.confirm.is_none(), "{kind}");
            match kind {
                // Requests follow main's dismissal semantics: the person's own dismissal
                // (`attention.dismiss`), not a work.done answer.
                "request" | "named" => {
                    assert!(matches!(ui.effects.as_slice(), [Effect::Dismiss { id: sent, .. }]
                        if sent == &id), "{kind}: {:?}", ui.effects);
                }
                "update" | "message" => {
                    let expected = if kind == "message" { "message.read" } else { "work.done" };
                    assert!(matches!(ui.effects.as_slice(), [Effect::Attention { id: sent, action, reason, answer, .. }]
                        if sent == &id && action == expected && match kind {
                            "update" => reason.as_deref() == Some("Read") && answer.as_deref() == Some("read"),
                            _ => answer.is_none(),
                        }), "{kind}: {:?}", ui.effects);
                }
                "closed" => {
                    assert!(!ui.closed.contains(&id));
                    assert!(!ui.world.attention.items().iter().any(|item| item.id == id));
                    assert!(ui.effects.is_empty());
                }
                _ => {
                    assert!(ui.effects.is_empty(), "{kind}");
                    let flash = &ui.flash.as_ref().unwrap().0;
                    assert!(flash.contains(if kind == "fault" { "A fault clears" } else { "x cannot dismiss it" }), "{flash}");
                }
            }
        }
    }

    #[test]
    fn middle_click_inbox_card_x_dismisses_its_card() {
        let (mut ui, id) = inbox("request");
        let index = ui.listing_for(0, 40).ids.iter().position(|candidate| candidate == &id).unwrap();
        ui.select(index);
        draw(&ui);
        let button = target(&ui, |hit| *hit == Hit::Key('x'));
        ui.select(1 - index); // The painted card, not the keyboard selection, owns the action.
        pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), button.x, button.y);
        assert!(ui.effects.is_empty() && ui.confirm.is_none());
        pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), button.x, button.y);
        assert!(matches!(ui.effects.as_slice(), [Effect::Dismiss { id: sent, .. }]
            if sent == &id), "{:?}", ui.effects);
        assert!(ui.confirm.is_none());
    }

    #[test]
    fn inbox_middle_click_respects_popover_and_palette_guards() {
        for palette in [false, true] {
            let (mut ui, _) = inbox("request");
            draw(&ui);
            let row = target(&ui, |hit| *hit == Hit::Row(0));
            if palette {
                ui.open_palette(None, glass::Open::Here);
            } else {
                ui.popover = Some("agent/example/planner".into());
            }
            pointer(&mut ui, MouseEventKind::Down(MouseButton::Middle), row.x, row.y);
            pointer(&mut ui, MouseEventKind::Up(MouseButton::Middle), row.x, row.y);
            assert!(ui.effects.is_empty() && ui.confirm.is_none());
        }
    }

    #[test]
    fn message_hover_still_allows_text_selection_and_more_precise_links() {
        let mut ui = Ui::new(demo::test_world());
        let agent = ui.world.agents.items()[0].id.clone();
        ui.world.conversations.insert(
            agent.clone(),
            Load::Ready(vec![Entry {
                id: "message/copper".into(),
                at: "12:00".into(),
                body: Body::Assistant("Copper text https://example.com/notes".into()),
            }]),
        );
        ui.open(&agent);
        draw(&ui);
        let rect = target(&ui, |hit| matches!(hit, Hit::Message));
        pointer(&mut ui, MouseEventKind::Moved, rect.x, rect.y);
        let buf = draw(&ui);
        let footer = line(&buf, 47);
        assert!(
            !footer.contains("Click:") && footer.contains("Keys:"),
            "the normal keys stay under the pointer: {footer}"
        );
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Left),
            rect.x,
            rect.y,
        );
        pointer(
            &mut ui,
            MouseEventKind::Drag(MouseButton::Left),
            rect.x + 4,
            rect.y,
        );
        assert!(ui.dragging && ui.conversation_state.selection.is_some());
        assert!(ui.selected_text().is_some());
        pointer(
            &mut ui,
            MouseEventKind::Up(MouseButton::Left),
            rect.x + 4,
            rect.y,
        );
        ui.flash = None;
        draw(&ui);
        let link = target(&ui, |hit| matches!(hit, Hit::Link(_)));
        pointer(&mut ui, MouseEventKind::Moved, link.x, link.y);
        assert!(line(&draw(&ui), 47).contains("Click: copy link"));
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Right),
            rect.x,
            rect.y,
        );
        assert!(matches!(
            ui.context.as_ref().unwrap().items[0].1,
            glass::MenuAction::CopyText(_)
        ));
    }

    #[test]
    fn popover_links_take_clicks_without_focusing_or_resizing_an_underlying_split() {
        let mut ui = Ui::new(demo::test_world());
        let agent = ui.world.agents.items()[0].id.clone();
        if let Load::Ready(agents) = &mut ui.world.agents {
            agents[0].details.progress = Some("https://example.com/notes".into());
        }
        ui.open_in_glass(Pane::Agent(Some(agent.clone())), glass::Open::Right);
        ui.popover = Some(agent.clone());
        draw(&ui);
        let link = target(&ui, |hit| matches!(hit, Hit::Link(_)));
        let group = ui
            .frame
            .borrow()
            .glass_leaves
            .iter()
            .position(|rect| contains(*rect, link.x, link.y))
            .unwrap();
        ui.focus_group(1 - group);
        draw(&ui);
        let focus = ui.focus();
        pointer(&mut ui, MouseEventKind::Moved, link.x, link.y);
        let buf = draw(&ui);
        assert!(line(&buf, 47).contains("Click: copy link"));
        assert!(
            buf[(link.x, link.y)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Left),
            link.x,
            link.y,
        );
        assert_eq!(ui.focus(), focus);
        assert_eq!(ui.popover.as_deref(), Some(agent.as_str()));
        assert!(
            ui.flash
                .as_ref()
                .unwrap()
                .0
                .contains("Copied https://example.com/notes")
        );
        ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(ui.popover.is_none());
    }

    #[test]
    fn subject_text_and_dividers_show_their_gestures_and_unfocused_buttons_name_focus() {
        let mut ui = Ui::new(demo::test_world());
        let mission = ui.world.missions.items()[0].id.clone();
        ui.open_in_glass(Pane::Mission(Some(mission)), glass::Open::Right);
        draw(&ui);
        let subject = target(&ui, |hit| matches!(hit, Hit::Subject));
        pointer(&mut ui, MouseEventKind::Moved, subject.x, subject.y);
        assert!(!line(&draw(&ui), 47).contains("Click:"), "text keeps the normal footer");
        let divider = target(&ui, |hit| matches!(hit, Hit::Resize));
        pointer(&mut ui, MouseEventKind::Moved, divider.x, divider.y);
        assert!(line(&draw(&ui), 47).contains("drag resize; double-click equalize"));
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Left),
            divider.x,
            divider.y,
        );
        pointer(
            &mut ui,
            MouseEventKind::Drag(MouseButton::Left),
            divider.x + 8,
            divider.y,
        );
        pointer(
            &mut ui,
            MouseEventKind::Up(MouseButton::Left),
            divider.x + 8,
            divider.y,
        );
        draw(&ui);
        let button = target(&ui, |hit| matches!(hit, Hit::ToggleTool(_)));
        ui.focus_group(0);
        draw(&ui);
        pointer(&mut ui, MouseEventKind::Moved, button.x, button.y);
        assert!(line(&draw(&ui), 47).contains("focus split; click again to act"));
    }

    #[test]
    fn host_and_person_are_status_and_inert_references_are_plain() {
        let mut ui = Ui::new(demo::test_world());
        let buf = draw(&ui);
        let word = target(&ui, |hit| matches!(hit, Hit::Connection));
        assert!(!ui.frame.borrow().hits.iter().any(|(rect, _)| contains(
            *rect,
            word.right() + 2,
            0
        )));
        pointer(
            &mut ui,
            MouseEventKind::Down(MouseButton::Left),
            word.x,
            word.y,
        );
        assert!(!ui.help);
        assert!(
            ui.tab() == 3 || ui.flash.is_some(),
            "connection shows the machine or connection status"
        );
        assert!(line(&buf, 0).contains("live"));
        let item = ui
            .world
            .attention
            .items()
            .iter()
            .find(|item| matches!(item.kind, AttentionKind::Feedback { .. }))
            .unwrap()
            .id
            .clone();
        if let Load::Ready(items) = &mut ui.world.attention
            && let AttentionKind::Feedback { link, .. } = &mut items
                .iter_mut()
                .find(|candidate| candidate.id == item)
                .unwrap()
                .kind
        {
            *link = Some("change/copper".into());
        }
        ui.popover = None;
        ui.open(&item);
        let buf = draw(&ui);
        for y in 0..48 {
            if line(&buf, y).contains("change/copper") {
                assert!(line(&buf, y).contains("reference"));
                assert!(!(0..160).any(|x| buf[(x, y)].modifier.contains(Modifier::UNDERLINED)));
                return;
            }
        }
        panic!("missing reference");
    }

    #[test]
    fn sweeping_the_pointer_over_every_cell_of_a_busy_conversation_never_panics() {
        // Nathan, 2026-10-07: hovering messages seemed to crash the terminal UI.
        for (width, height) in [(60, 20), (100, 30), (160, 48)] {
            let mut ui = Ui::new(demo::test_world());
            ui.live = true;
            ui.open_in_glass(
                Pane::Agent(Some("agent/example/atlas/builder".to_owned())),
                super::glass::Open::Tab,
            );
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui.render(frame)).unwrap();
            for y in 0..height {
                for x in 0..width {
                    pointer(&mut ui, MouseEventKind::Moved, x, y);
                    terminal.draw(|frame| ui.render(frame)).unwrap();
                }
            }
        }
    }
}
