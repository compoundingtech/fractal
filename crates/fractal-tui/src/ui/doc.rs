//! A detail pane is a document: lines plus the click targets inside them.
//!
//! Building every pane as lines (cards included) means one scroll model, one selection model
//! and one hit map for all of them. A click target is stored against its line, so it still
//! works after the pane scrolls.

use super::text::{self, run};
use super::theme;
use ratatui::style::{Color, Modifier, Style};
use fractal_conversation_ui::text::{Line, Span};

/// What a button on an alert does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlertTap {
    /// Send one of its typed answers: a prompt's (`allow`, `deny`, `skip`, ...), or a structured
    /// answer's id.
    Answer(String),
    /// Choose, or for a multi-select also unchoose, an option of a harness's question: the
    /// question's and the option's places in the pending call.
    Choose(usize, usize),
    /// Open the alert's whole card in Inbox: a call too long for the band is read whole there.
    Open,
    /// Expand or collapse the band: the `Hit` names the agent's conversation, not an alert.
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Expand or collapse the current step's text in an agent's details.
    DetailsPreview(String),
    /// A button on an alert shown in its agent's conversation.
    Alert(String, AlertTap),
    /// One of a structured request's named answers, by its place: chosen, then Enter sends it.
    Answer(usize),
    /// Speak into the focused message box instead of typing (voice mode).
    Voice,
    /// Start a plain shell in a new tab.
    NewTerminal,
    /// Show or hide the sidebar, exactly as Ctrl+S.
    SidebarToggle,
    /// A row of the Ctrl+S sidebar's list: select it and open it.
    SidebarRow(usize),
    ResourceRow(usize),
    ResourceFilter,
    ResourceNewFolder,
    ResourceAdd(String),
    ResourceFolderMenu(String),
    ResourceFolderDrag(String),
    /// One of the sidebar's sections across its top.
    SidebarSection(usize),
    /// Glasses: the top bar's usage slot, which shows and hides the sidebar's Usage.
    Usage,
    /// The top bar's connection word ("live"): this machine, or why it is not reached.
    Connection,
    Row(usize),
    /// A conversation entry: left drag selects text; right click opens message actions.
    Message,
    /// A subject's document: left drag selects text; right click opens its actions.
    Subject,
    /// A split border: drag resizes, a second press evens its split.
    Resize,
    SidebarResize,
    Key(char),
    Enter,
    Escape,
    ToggleTool(String),
    ContentImage(super::content::Key),
    ContentOutput(super::content::Key),
    InlineImage(super::content::Key),
    Pane(fractal_conversation_ui::PaneIntent),
    JumpLatest,
    Composer,
    ChatComposer,
    ChatEnter,
    Help,
    Open(String),
    /// Show a popover card for a graph subject.
    Peek(String),
    /// Open the actions menu for an agent.
    Actions(String),
    /// Focus a field of a form.
    Field(usize),
    /// Revoke a paired device (after a confirmation).
    Revoke(String),
    /// Attach the focused terminal tab, as Enter on a detached terminal pane.
    Attach,
    /// Leave the terminal view.
    Detach,
    /// Glasses: open the palette at the glasses section.
    GlassMenu,
    /// Glasses: open the palette at one section.
    PaletteSection(usize),
    /// Glasses: open or close Home over the glass.
    Home,
    /// A link: copied to the person's clipboard, wherever their terminal is.
    Link(String),
    /// Glasses: split the focused group, to the right (`true`) or below.
    Split(bool),
    /// Glasses: a row of the right-click menu.
    Menu(super::glass::MenuAction),
    /// Glasses: show a group's tab (Now opens from the top bar, not a fixed tab).
    GlassTab(usize, usize),
    /// Glasses: the marker for tabs that do not fit the strip; shows the nearest hidden one.
    GlassTabMore(usize, usize),
    /// Glasses: open the palette for a new tab in a group.
    GlassAdd(usize),
    /// Glasses: open a palette row.
    PaletteChoice(usize),
    /// A document page's or the documents list's own button: back, try again, more.
    Document(super::documents::Tap),
    /// An Availability pane button: pressing its key.
    Availability(char),
}

pub type Target = fractal_conversation_ui::doc::Target<Hit>;
pub type Doc = fractal_conversation_ui::doc::Doc<Hit>;

/// Shell-only cards, graph fields and buttons layered on the shared document.
pub trait DocExt {
    fn field(&mut self, label: &str, value: &str, width: usize, value_style: Style);
    fn section(&mut self, title: &str, count: Option<usize>, width: usize);
    fn buttons(&mut self, buttons: &[(&str, &str, Hit, Color)]);
    fn buttons_fit(&mut self, buttons: &[(&str, &str, Hit, Color)], width: usize);
    fn card(&mut self, title: &str, color: Color, heavy: bool, inner: Doc, width: usize);
}

impl DocExt for Doc {
    /// A "label   value" row with the value wrapped under itself.
    fn field(&mut self, label: &str, value: &str, width: usize, value_style: Style) {
        let (first, rest) = text::field_prefixes(label, 10, theme::dim());
        self.lines.extend(text::wrap(
            &text::inline(&text::sanitize(value), value_style),
            width,
            &first,
            &rest,
            None,
        ));
    }
    /// A section heading: a small bold label and a rule.
    fn section(&mut self, title: &str, count: Option<usize>, width: usize) {
        let mut spans = vec![Span::styled(title.to_owned(), theme::label())];
        if let Some(count) = count {
            spans.push(Span::decoration(format!(" {count}"), theme::dim()));
        }
        let used = spans.iter().map(|span| span.width()).sum::<usize>() + 1;
        spans.push(Span::decoration(" ", Style::default()));
        spans.push(Span::decoration(
            "─".repeat(width.saturating_sub(used)),
            theme::fg(theme::SURFACE0),
        ));
        let line = Line::from(spans);
        self.lines.push(if title.is_empty() { line.decoration() } else { line });
    }
    /// A row of buttons. Each is `[key label]`, and clicking it presses the key.
    fn buttons(&mut self, buttons: &[(&str, &str, Hit, Color)]) {
        let line = self.lines.len();
        let mut spans = Vec::new();
        let mut column = 0u16;
        for (index, (key, label, hit, color)) in buttons.iter().enumerate() {
            if index > 0 {
                spans.push(Span::decoration("  ", Style::default()));
                column += 2;
            }
            let key_text = format!(" {key} ");
            let label_text = format!(" {label} ");
            let width = (text::width(&key_text) + text::width(&label_text)) as u16;
            spans.push(Span::decoration(
                key_text,
                Style::default()
                    .fg(theme::CRUST)
                    .bg(*color)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::decoration(
                label_text,
                Style::default().fg(*color).bg(theme::SURFACE0),
            ));
            self.targets.push(Target {
                line,
                column,
                width,
                hit: hit.clone(),
            });
            column += width;
        }
        self.lines.push(Line::from(spans).decoration());
    }
    /// Place whole buttons on the same row when the available cells allow it.
    fn buttons_fit(&mut self, buttons: &[(&str, &str, Hit, Color)], width: usize) {
        let mut start = 0;
        let mut used = 0;
        for (index, (key, label, _, _)) in buttons.iter().enumerate() {
            let button_width = text::width(key) + text::width(label) + 4;
            if index > start && used + 2 + button_width > width {
                self.buttons(&buttons[start..index]);
                start = index;
                used = 0;
            }
            used += usize::from(index > start) * 2 + button_width;
        }
        if start < buttons.len() {
            self.buttons(&buttons[start..]);
        }
    }

    /// A card in deck's style: a rounded frame with its title in the top edge. A heavy
    /// card (thick frame, coloured) is reserved for the thing that needs a person.
    fn card(&mut self, title: &str, color: Color, heavy: bool, inner: Doc, width: usize) {
        let width = width.max(8);
        let (tl, tr, bl, br, h, v) = if heavy {
            ("┏", "┓", "┗", "┛", "━", "┃")
        } else {
            ("╭", "╮", "╰", "╯", "─", "│")
        };
        let border = theme::fg(if heavy { color } else { theme::SURFACE1 });
        let title = text::truncate(
            &format!(" {} ", title.to_uppercase()),
            width.saturating_sub(4),
        );
        let fill = width.saturating_sub(2 + 1 + text::width(&title));
        self.lines.push(Line::from(vec![
            Span::decoration(format!("{tl}{h}"), border),
            Span::decoration(title, theme::strong(color)),
            Span::decoration(format!("{}{tr}", h.repeat(fill)), border),
        ]).decoration());
        let base = self.lines.len();
        let inner_width = width.saturating_sub(4);
        for target in inner.targets {
            self.targets.push(Target {
                line: base + target.line,
                column: target.column + 2,
                ..target
            });
        }
        self.messages.extend(
            inner
                .messages
                .into_iter()
                .map(|(id, range)| (id, range.start + base..range.end + base)),
        );
        self.copy_blocks.extend(inner.copy_blocks.into_iter().map(|range| range.start + base..range.end + base));
        self.copy_blocks.push(base..base + inner.lines.len());
        for line in inner.lines {
            let used = line.width();
            let mut line = fractal_conversation_ui::text::prefixed(
                vec![Span::decoration(format!("{v} "), border)], line,
            );
            line.spans.push(Span::decoration(" ".repeat(inner_width.saturating_sub(used)), Style::default()));
            line.spans.push(Span::decoration(format!(" {v}"), border));
            self.lines.push(line);
        }
        self.lines.push(Line::from(Span::decoration(
            format!("{bl}{}{br}", h.repeat(width.saturating_sub(2))),
            border,
        )).decoration());
    }
}

/// A meter of eighth blocks.
pub fn meter(done: usize, total: usize, cells: usize, color: Color) -> Vec<Span<'static>> {
    fractal_mission_ui::meter(
        done,
        total,
        cells,
        color,
        &fractal_mission_ui::Palette::default(),
        &fractal_mission_ui::Glyphs::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_separate_keycaps_and_labels_and_include_the_gap_in_targets() {
        let mut doc = Doc::new();
        doc.buttons(&[
            ("a", "Choose an answer", Hit::Key('a'), theme::ACCENT),
            ("Esc", "Cancel", Hit::Escape, theme::OVERLAY1),
        ]);
        assert_eq!(doc.lines[0].to_string(), " a  Choose an answer    Esc  Cancel ");
        assert_eq!(doc.lines[0].spans[1].content, " Choose an answer ");
        assert_eq!(doc.targets[0].column, 0);
        assert_eq!(doc.targets[0].width, 21);
        assert_eq!(doc.targets[1].column, 23);
        assert_eq!(doc.targets[1].width, 13);
        let gap = 3;
        assert_eq!(
            doc.targets
                .iter()
                .find(|target| gap >= target.column && gap < target.column + target.width)
                .map(|target| &target.hit),
            Some(&Hit::Key('a'))
        );
    }

    #[test]
    fn buttons_fit_wraps_whole_click_targets_at_the_available_width() {
        let buttons = [
            ("a", "Choose an answer", Hit::Key('a'), theme::ACCENT),
            ("x", "Dismiss without answering", Hit::Key('x'), theme::OVERLAY1),
        ];
        let mut wide = Doc::new();
        wide.buttons_fit(&buttons, 53);
        assert_eq!(wide.lines.len(), 1);
        assert_eq!((wide.targets[1].line, wide.targets[1].column), (0, 23));

        let mut narrow = Doc::new();
        narrow.buttons_fit(&buttons, 52);
        assert_eq!(narrow.lines.len(), 2);
        assert_eq!((narrow.targets[1].line, narrow.targets[1].column), (1, 0));
        assert_eq!(narrow.targets[1].hit, Hit::Key('x'));
    }

    #[test]
    fn card_lines_have_one_width_and_targets_move_inside_the_frame() {
        let mut inner = Doc::new();
        inner.line(Line::from("hello"));
        inner.buttons(&[("a", "Approve", Hit::Key('a'), theme::GREEN)]);
        let mut doc = Doc::new();
        doc.card("review", theme::MAUVE, true, inner, 30);
        assert!(doc.lines.iter().all(|line| line.width() == 30));
        assert_eq!(doc.targets[0].line, 2);
        assert_eq!(doc.targets[0].column, 2);
    }

    fn copied(doc: &Doc) -> String {
        fractal_conversation_ui::Selection {
            pane: "card".into(),
            anchor: (0, 0),
            head: (doc.lines.len().saturating_sub(1), u16::MAX),
        }.text(&doc.lines)
    }

    #[test]
    fn card_copy_rejoins_prose_and_urls_and_copies_wrapped_tables_as_tsv() {
        let prose = "Wrapped prose should read as one paragraph without the frame.";
        let url = "https://example.com/a/long/path?complete=target";
        let source = format!("{prose}\n\nVista: {url}\n\n| Item | Notes |\n| --- | --- |\n| Copper | A long note that wraps inside its cell |\n| Silver | Short |");
        for heavy in [false, true] {
            let mut inner = Doc::new();
            inner.lines(text::markdown(&source, 26, theme::text()));
            inner.buttons(&[("a", "Approve", Hit::Key('a'), theme::GREEN)]);
            let mut card = Doc::new();
            card.card("request · waiting", theme::ACCENT, heavy, inner, 30);
            assert_eq!(copied(&card), format!("{prose}\n\nVista: {url}\n\nItem\tNotes\nCopper\tA long note that wraps inside its cell\nSilver\tShort"));
            assert!(card.lines.iter().all(|line| line.width() == 30));
            assert_eq!(card.copy_blocks.last(), Some(&(1..card.lines.len() - 1)));
        }
    }

    #[test]
    fn nested_cards_keep_copy_metadata_but_not_chrome_or_keycaps() {
        let mut body = Doc::new();
        body.wrap(&[run("A paragraph whose words wrap across several rows.", theme::text())], 16);
        body.blank();
        body.lines(text::markdown("> Quoted words are content, not the quote bar.", 16, theme::text()));
        let mut nested = Doc::new();
        nested.card("inner", theme::ACCENT, false, body, 20);
        let mut outer = Doc::new();
        outer.card("outer", theme::ACCENT, true, nested, 24);
        assert_eq!(copied(&outer), "A paragraph whose words wrap across several rows.\n\nQuoted words are content, not the quote bar.");
        assert_eq!(outer.copy_blocks.len(), 2);
        assert!(outer.lines.iter().all(|line| line.width() == 24));
    }

    #[test]
    fn field_copy_uses_a_semantic_gap_not_alignment_padding() {
        let mut doc = Doc::new();
        doc.field("owner", "Copper gardener with a long description", 24, theme::text());
        assert_eq!(copied(&doc), "owner Copper gardener with a long description");
    }

    #[test]
    fn subject_link_prefixes_copy_one_gap_without_alignment_padding() {
        let mut doc = Doc::new();
        super::super::screens::link(&mut doc, "agent    ", "Copper", "agent/copper");
        super::super::screens::link(&mut doc, "mission  ", "Garden", "mission/garden");
        assert_eq!(copied(&doc), "agent Copper\nmission Garden");
        assert_eq!(doc.targets[0].column, 9);
        assert_eq!(doc.lines[0].width(), 15);
    }
}
