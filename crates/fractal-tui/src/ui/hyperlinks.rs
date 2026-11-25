//! OSC 8 is terminal-output metadata, not text in ratatui cells. Semantic spans keep the full
//! target through wrapping; after ratatui draws, replay only visible link cells. Every physical
//! segment carries the same explicit ID, including across frames, so terminals group the link.

use ratatui::{backend::Backend, backend::CrosstermBackend, buffer::{Cell, CellDiffOption, CellWidth}};
use sha2::{Digest, Sha256};
use std::io::{self, Write};

use super::*;

/// One link's cells as drawn this frame.
pub(super) struct LinkCells {
    url: String,
    cells: Vec<(u16, u16, Cell)>,
}

/// OSC 8 is independent of image support. Unsupported emulators ignore OSC commands and retain
/// the ordinary text; known non-OSC terminals do not need the replay at all.
pub(super) fn supported(term: &str) -> bool {
    !matches!(term, "" | "dumb" | "linux")
}

impl Ui {
    /// The web addresses showing on screen, with the cells that draw them, read from the finished
    /// frame. An address hidden behind a later layer is left out.
    pub(super) fn link_cells(&self, buf: &mut Buffer) -> Vec<LinkCells> {
        let info = self.frame.borrow();
        collect_links(
            buf,
            info.hits.iter().enumerate().filter_map(|(index, (rect, hit))| {
                match hit {
                    Hit::Link(url) => Some((index, *rect, url.as_str())),
                    _ => None,
                }
            }),
            |index, x, y| info.covers.iter().any(|(cover, first)| {
                *first > index && cover.contains((x, y).into())
            }),
        )
    }
}

/// Capture visible link segments for any semantic text surface, including setup before Ui runs.
pub(super) fn collect_links<'a>(
    buf: &mut Buffer,
    regions: impl IntoIterator<Item = (usize, Rect, &'a str)>,
    covered: impl Fn(usize, u16, u16) -> bool,
) -> Vec<LinkCells> {
    let mut links: std::collections::BTreeMap<&str, Vec<(u16, u16, Cell)>> =
        std::collections::BTreeMap::new();
    for (index, rect, url) in regions {
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.chars().any(char::is_control)
        {
            continue;
        }
        let rect = rect.intersection(buf.area);
        for y in rect.y..rect.bottom() {
            let mut x = rect.x;
            while x < rect.right() {
                let width = buf[(x, y)].symbol().cell_width().max(1);
                let end = x.saturating_add(width).min(rect.right());
                if !(x..end).any(|column| covered(index, column, y)) {
                    // The diff option is part of cell equality, so removing a link forces a
                    // plain rewrite even when its text is unchanged, clearing stale OSC metadata.
                    for column in x..end {
                        buf[(column, y)].set_diff_option(CellDiffOption::Skip);
                    }
                    // Replaying wide-grapheme continuation spaces would erase the grapheme.
                    links.entry(url).or_default().push((x, y, buf[(x, y)].clone()));
                }
                x = end;
            }
        }
    }
    links.into_iter().map(|(url, mut cells)| {
        cells.sort_unstable_by_key(|(x, y, _)| (*y, *x));
        cells.dedup_by_key(|(x, y, _)| (*y, *x));
        LinkCells { url: url.to_owned(), cells }
    }).filter(|link| !link.cells.is_empty()).collect()
}

/// Write each link's cells again inside an OSC 8 hyperlink.
pub(super) fn write_links<W: Write>(
    backend: &mut CrosstermBackend<W>,
    links: &[LinkCells],
) -> io::Result<()> {
    for link in links {
        let id = Sha256::digest(link.url.as_bytes());
        // Cursor positioning creates physical segments, not terminal soft wraps. Explicit IDs
        // join them; close before any cursor jump so borders and padding are never in the link.
        let mut start = 0;
        while start < link.cells.len() {
            let mut end = start + 1;
            while end < link.cells.len()
                && link.cells[end].1 == link.cells[end - 1].1
                && link.cells[end].0 == link.cells[end - 1].0
                    + link.cells[end - 1].2.symbol().cell_width().max(1)
            {
                end += 1;
            }
            write!(backend, "\x1b]8;id=fractal-{id:x};{}\x1b\\", link.url)?;
            backend.draw(link.cells[start..end].iter().map(|(x, y, cell)| (*x, *y, cell)))?;
            write!(backend, "\x1b]8;;\x1b\\")?;
            start = end;
        }
    }
    Write::flush(backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    /// A writer the test can read back after the backend takes it.
    #[derive(Clone, Default)]
    struct Sink(Rc<RefCell<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn links(urls: &[&str]) -> Vec<LinkCells> {
        urls.iter()
            .enumerate()
            .map(|(row, url)| LinkCells {
                url: (*url).into(),
                cells: "abc"
                    .chars()
                    .enumerate()
                    .map(|(x, c)| {
                        let mut cell = Cell::default();
                        cell.set_symbol(&c.to_string());
                        (x as u16 + 2, row as u16, cell)
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn each_link_is_rewritten_between_an_open_and_a_close() {
        let sink = Sink::default();
        let mut backend = CrosstermBackend::new(sink.clone());
        write_links(&mut backend, &links(&["https://example.com/a", "https://example.com/b"]))
            .unwrap();
        let out = String::from_utf8_lossy(&sink.0.borrow()).into_owned();
        let open_a = out.find(";https://example.com/a\x1b\\").unwrap();
        let open_b = out.find(";https://example.com/b\x1b\\").unwrap();
        let closes: Vec<_> = out.match_indices("\x1b]8;;\x1b\\").map(|(at, _)| at).collect();
        assert_eq!(closes.len(), 2, "{out:?}");
        // The text sits between its own open and close, and the second link starts after the
        // first has closed.
        assert!(open_a < closes[0] && closes[0] < open_b && open_b < closes[1]);
        assert!(out[open_a..closes[0]].contains('a') && out[open_a..closes[0]].contains('c'));
    }

    #[test]
    fn an_address_in_a_message_is_marked_with_the_cells_that_draw_it() {
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
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 48)).unwrap();
        let mut found = Vec::new();
        terminal
            .draw(|frame| {
                ui.render(frame);
                found = ui.link_cells(frame.buffer_mut());
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://example.com/notes");
        let drawn: String = found[0].cells.iter().map(|(_, _, cell)| cell.symbol()).collect();
        assert_eq!(drawn, "https://example.com/notes");
    }

    #[test]
    fn only_visible_web_addresses_are_marked() {
        let mut ui = Ui::new(demo::test_world());
        ui.live = true;
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        {
            let mut info = ui.frame.borrow_mut();
            let area = Rect::new(0, 1, 20, 1);
            for url in [
                "https://example.com/ok",
                "/srv/not-a-web-address",
                "javascript:alert(1)",
                "https://example.com/\u{1b}bad",
            ] {
                info.hits.push((area, Hit::Link(url.into())));
            }
        }
        let marked = ui.link_cells(terminal.current_buffer_mut());
        let urls: Vec<_> = marked.iter().map(|link| link.url.as_str()).collect();
        assert_eq!(urls, ["https://example.com/ok"]);
    }

    #[test]
    fn wrapped_card_segments_keep_one_id_and_the_whole_target() {
        let url = "https://example.com/a/very/long/path?query=complete";
        let ui = Ui::new(demo::test_world());
        let mut inner = Doc::new();
        inner.wrap(&[text::run(url, theme::text())], 18);
        let mut card = Doc::new();
        card.card("request · waiting", theme::ACCENT, false, inner, 22);
        let mut buf = Buffer::empty(Rect::new(0, 0, 22, card.lines.len() as u16));
        for (row, line) in card.lines.iter().enumerate() {
            ui.draw_line(&mut buf, 0, row as u16, line, 22);
        }
        let links = ui.link_cells(&mut buf);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, url);
        let visible: String = links[0].cells.iter().map(|(_, _, cell)| cell.symbol()).collect();
        assert_eq!(visible, url);
        assert!(buf.content.iter().all(|cell| !cell.symbol().contains('\x1b')));
        let rows: std::collections::BTreeSet<_> =
            links[0].cells.iter().map(|(_, y, _)| *y).collect();
        assert!(rows.len() > 1);
        let before = buf.clone();
        let sink = Sink::default();
        write_links(&mut CrosstermBackend::new(sink.clone()), &links).unwrap();
        assert_eq!(buf, before, "OSC output must not change buffer width or symbols");
        let bytes = sink.0.borrow();
        let output = String::from_utf8_lossy(&bytes);
        let opens: Vec<_> = output.split("\x1b]8;id=")
            .skip(1).map(|part| part.split("\x1b\\").next().unwrap()).collect();
        assert_eq!(opens.len(), rows.len());
        assert!(opens.iter().all(|open| *open == opens[0]));
        assert!(opens[0].ends_with(&format!(";{url}")));
        assert_eq!(output.matches("\x1b]8;;\x1b\\").count(), rows.len());
    }

    #[test]
    fn non_osc_terminals_keep_plain_text_and_app_click_targets() {
        for term in ["", "dumb", "linux"] {
            assert!(!supported(term));
        }
        for term in ["xterm-ghostty", "xterm-kitty", "xterm-256color", "tmux-256color"] {
            assert!(supported(term));
        }
        let ui = Ui::new(demo::test_world());
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let line = text::wrap(&[text::run("https://example.com", theme::text())], 10, &[], &[], None);
        ui.draw_line(&mut buf, 0, 0, &line[0], 10);
        assert_eq!(buf.content.iter().map(|cell| cell.symbol()).collect::<String>(), "https://ex");
        assert!(ui.frame.borrow().hits.iter().any(|(_, hit)|
            matches!(hit, Hit::Link(url) if url == "https://example.com")));
        let sink = Sink::default();
        if supported("dumb") {
            write_links(&mut CrosstermBackend::new(sink.clone()), &ui.link_cells(&mut buf)).unwrap();
        }
        assert!(sink.0.borrow().is_empty());
    }

    #[test]
    fn removing_link_metadata_invalidates_unchanged_visible_text() {
        let ui = Ui::new(demo::test_world());
        let mut linked = Buffer::empty(Rect::new(0, 0, 12, 1));
        let mut plain = linked.clone();
        let line = text::wrap(&[text::run("https://x.io", theme::text())], 12, &[], &[], None);
        ui.draw_line(&mut linked, 0, 0, &line[0], 12);
        ui.draw_line(&mut plain, 0, 0, &line[0], 12);
        assert_eq!(ui.link_cells(&mut linked).len(), 1);
        let erased = linked.diff(&plain);
        assert_eq!(erased.len(), "https://x.io".len());
        assert!(plain.content.iter().all(|cell| cell.diff_option == CellDiffOption::None));
    }

    #[test]
    fn wide_link_labels_replay_only_the_leading_cells() {
        let ui = Ui::new(demo::test_world());
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        let runs = [fractal_conversation_ui::text::link("漢字", "https://example.com", theme::text())];
        let lines = text::wrap(&runs, 8, &[], &[], None);
        ui.draw_line(&mut buf, 0, 0, &lines[0], 8);
        let links = ui.link_cells(&mut buf);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].cells.iter().map(|(x, _, _)| *x).collect::<Vec<_>>(), vec![0, 2]);
        let sink = Sink::default();
        write_links(&mut CrosstermBackend::new(sink.clone()), &links).unwrap();
        let bytes = sink.0.borrow();
        let out = String::from_utf8_lossy(&bytes);
        assert_eq!(out.matches("\x1b]8;id=").count(), 1);
        assert_eq!(out.matches('漢').count(), 1);
        assert_eq!(out.matches('字').count(), 1);
    }

    #[test]
    fn halfwidth_katakana_replay_preserves_cell_positions_and_card_edges() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 7, 1));
        buf.set_string(0, 0, "| ｶﾞx |", ratatui::style::Style::default());
        let before = buf.clone();
        let links = collect_links(
            &mut buf,
            [(0, Rect::new(2, 0, 3, 1), "https://example.com")],
            |_, _, _| false,
        );
        assert_eq!(links.len(), 1);
        let cells: Vec<_> = links[0].cells.iter()
            .map(|(x, y, cell)| (*x, *y, cell.symbol())).collect();
        assert_eq!(cells, [(2, 0, "ｶﾞ"), (4, 0, "x")]);
        for x in 0..7 {
            assert_eq!(buf[(x, 0)].symbol(), before[(x, 0)].symbol());
            assert_eq!(
                buf[(x, 0)].diff_option,
                if (2..5).contains(&x) { CellDiffOption::Skip } else { CellDiffOption::None },
            );
        }
        let sink = Sink::default();
        write_links(&mut CrosstermBackend::new(sink.clone()), &links).unwrap();
        let bytes = sink.0.borrow();
        let out = String::from_utf8_lossy(&bytes);
        assert_eq!(out.matches("\x1b]8;id=").count(), 1, "{out:?}");
        assert_eq!(out.matches("\x1b]8;;\x1b\\").count(), 1, "{out:?}");
        assert!(out.contains("\x1b[1;3Hｶﾞ"), "{out:?}");
        assert!(out.contains("\x1b[1;5Hx"), "{out:?}");
        assert!(!out.contains(' ') && !out.contains('|'), "{out:?}");
    }
}
