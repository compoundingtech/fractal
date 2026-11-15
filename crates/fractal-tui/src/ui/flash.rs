//! The footer's passing note: how it reads at a glance and where it sits on the row.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::{text, theme};

/// How a footer note reads at a glance. Thread call sites state it; notes from before
/// severities were explicit are still read from their words (`Severity::guess`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    /// Something is under way or there was nothing to do.
    Info,
    Success,
    /// Nothing happened yet and the person can fix it; their words are kept.
    Warn,
    /// It failed or was refused.
    Error,
}

impl Severity {
    /// The rule for notes that do not state a severity: a failure or refusal is named in its
    /// words, anything else went well.
    pub(super) fn guess(message: &str) -> Self {
        let trouble = [
            "Failed",
            "Could not",
            "not sent",
            "Detach failed",
            "Key not sent",
        ]
        .iter()
        .any(|word| message.contains(word));
        if trouble { Self::Error } else { Self::Success }
    }

    fn style(self) -> Style {
        let (fg, bg) = match self {
            Self::Info => (theme::TEXT, theme::SURFACE1),
            Self::Success => (theme::CRUST, theme::GREEN),
            Self::Warn => (theme::CRUST, theme::YELLOW),
            Self::Error => (theme::CRUST, theme::RED),
        };
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
    }
}

/// A note at the right of the footer row, one column in from the edge, cut with `…` when the
/// row is too narrow for it. The key hints end before `start`, so none shows through in pieces.
pub(super) struct Pill {
    text: String,
    style: Style,
    start: u16,
}

impl Pill {
    pub(super) fn new(message: &str, severity: Severity, area: Rect) -> Self {
        // One column of footer on each side and one of padding inside the pill.
        let room = area.width.saturating_sub(4) as usize;
        let text = format!(" {} ", text::truncate(message, room));
        let width = text::width(&text) as u16;
        let start = (area.x + area.width).saturating_sub(width + 1).max(area.x);
        Self {
            text,
            style: severity.style(),
            start,
        }
    }

    pub(super) fn start(&self) -> u16 {
        self.start
    }

    pub(super) fn draw(&self, buf: &mut Buffer, area: Rect) {
        buf.set_stringn(
            self.start,
            area.y,
            &self.text,
            (area.x + area.width - self.start) as usize,
            self.style,
        );
    }
}

/// With THREAD_PREVIEW_CAPTURE_DIR set, the whole shell at 80×24 as text, with the footer
/// note's colors named, for before/after evidence.
#[cfg(test)]
pub(super) fn capture_shell(ui: &super::Ui, name: &str) {
    let Ok(dir) = std::env::var("THREAD_PREVIEW_CAPTURE_DIR") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let (width, height) = (80u16, 24u16);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui.render(frame)).unwrap();
    let screen = terminal.backend().buffer().clone();
    let mut text =
        String::from("HARNESS — production Ui::render at 80×24, not the installed TUI binary\n");
    for row in screen.content.chunks(width as usize) {
        text.push_str(&row.iter().map(|cell| cell.symbol()).collect::<String>());
        text.push('\n');
    }
    let notes = screen
        .content
        .iter()
        .filter(|cell| [theme::RED, theme::YELLOW, theme::GREEN, theme::SURFACE1].contains(&cell.bg))
        .map(|cell| format!("{:?} on {:?}", cell.fg, cell.bg))
        .collect::<std::collections::BTreeSet<_>>();
    for note in notes {
        text.push_str(&format!("note colors: {note}\n"));
    }
    std::fs::write(std::path::Path::new(&dir).join(format!("{name}.txt")), text).unwrap();
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::{Severity, capture_shell};

    fn footer(ui: &Ui, width: u16) -> (Vec<String>, Vec<ratatui::style::Color>) {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        ui.footer(&mut buf, area);
        (0..width)
            .map(|x| {
                let cell = &buf[(x, 0)];
                (cell.symbol().to_owned(), cell.bg)
            })
            .unzip()
    }

    fn live_ui() -> Ui {
        let mut ui = Ui::new(demo::test_world());
        ui.world.link = Link::Live;
        ui
    }

    #[test]
    fn thread_errors_and_refusals_never_flash_green() {
        let mut ui = live_ui();
        for (severity, message, bg) in [
            (Severity::Error, "the connection to Fractal broke (Reconnect to send this thread action)", theme::RED),
            (Severity::Error, "The original thread action was rejected", theme::RED),
            (Severity::Error, "This item has no available thread context. Your draft is kept.", theme::RED),
            (Severity::Error, "Not sent: not allowed: this device cannot write", theme::RED),
            (Severity::Warn, "The thread context is not ready", theme::YELLOW),
            (Severity::Warn, "Upgrade the connected Fractal to use threads. Your pairing is kept.", theme::YELLOW),
            (Severity::Info, "No visible replies to mark read", theme::SURFACE1),
            (Severity::Success, "Attached notes.md", theme::GREEN),
        ] {
            ui.flash_as(severity, message);
            capture_shell(&ui, &format!("p1-3-{}-{severity:?}", message.split(' ').take(3).collect::<Vec<_>>().join("-")));
            let (cells, colors) = footer(&ui, 140);
            let start = colors.iter().position(|color| *color != theme::CRUST).expect("a pill");
            let shown: String = cells[start..].concat();
            assert!(shown.contains(message), "{message}: {shown}");
            assert!(
                colors[start..start + message.len()].iter().all(|color| *color == bg),
                "{message} is drawn on {:?}, not {bg:?}",
                colors[start]
            );
        }
    }

    #[test]
    fn notes_without_a_stated_severity_keep_the_words_rule() {
        assert_eq!(Severity::guess("Could not attach notes.md"), Severity::Error);
        assert_eq!(Severity::guess("Key not sent: the terminal closed"), Severity::Error);
        assert_eq!(Severity::guess("Copied 3 lines"), Severity::Success);
        let mut ui = live_ui();
        ui.flash_as(Severity::Error, "Not sent: refused");
        // A note that states none does not inherit the last one's.
        ui.flash("Copied 3 lines");
        let (_, colors) = footer(&ui, 80);
        assert!(colors.contains(&theme::GREEN) && !colors.contains(&theme::RED));
    }

    #[test]
    fn a_flash_leaves_only_whole_key_hints_beside_it_and_ends_in_an_ellipsis_when_long() {
        let mut ui = live_ui();
        let (plain, _) = footer(&ui, 80);
        let hints = plain.concat();
        for message in [
            "Not sent: not allowed: this device cannot write to the thread",
            "The original thread action was rejected",
        ] {
            ui.flash_as(Severity::Error, message);
            capture_shell(&ui, &format!("p1-3-hints-beside-{}", message.len()));
            let (cells, colors) = footer(&ui, 80);
            let start = colors.iter().position(|color| *color != theme::CRUST).unwrap();
            let before = cells[..start].concat();
            let kept = before.trim_end();
            // What shows beside the pill is the footer's own start, cut between two hints.
            assert!(
                kept.is_empty() || hints.starts_with(kept) && hints[kept.len()..].starts_with("   "),
                "hints garbled beside the note: {before:?} | {hints:?}"
            );
        }
        let long = "The original thread action was rejected because Fractal changed while it was on its way and the draft is kept";
        ui.flash_as(Severity::Error, long);
        capture_shell(&ui, "p1-3-long-error-ellipsis");
        let (cells, colors) = footer(&ui, 80);
        let row = cells.concat();
        assert!(!row.contains("Keys"), "{row}");
        let start = colors.iter().position(|color| *color != theme::CRUST).unwrap();
        assert_eq!(start, 1, "{row}");
        assert_eq!(colors[79], theme::CRUST, "one column of footer at the edge: {row}");
        assert!(row.trim_end().ends_with('…'), "{row}");
    }

    #[test]
    fn a_flash_clears_previous_footer_glyphs_before_drawing() {
        let mut ui = live_ui();
        ui.flash_as(Severity::Error, "The original thread action was rejected");
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        for cell in &mut buf.content {
            cell.set_char('#');
        }
        ui.footer(&mut buf, area);
        assert!(buf.content.iter().all(|cell| cell.symbol() != "#"));
    }
}
