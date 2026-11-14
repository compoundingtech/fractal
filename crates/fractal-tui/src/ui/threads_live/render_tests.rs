//! Presentation of mounted thread panels: disclosure, status placement and selection.

use super::*;

fn snapshot() -> Snapshot {
    serde_json::from_value(serde_json::json!({
        "id":"snapshot/render", "host_id":"host/test", "store_index":1,
        "projection_version":"1","created_at":"2026-11-07T00:00:00Z"
    }))
    .unwrap()
}

fn summary(id: &str, title: &str) -> ThreadSummary {
    serde_json::from_value(serde_json::json!({
        "id":id, "agent_id":"agent/atlas", "title":title,
        "revision":"claim/current", "title_revision":"claim/title", "state":"open",
        "message_count":0,"unread":false
    }))
    .unwrap()
}

/// An agent pane listing `threads`, with the given connection grants and no loaded history.
fn listed(threads: &[(&str, &str)], access: ThreadAccess) -> (Threads, Pane) {
    let mut live = Threads::default();
    live.enable(true);
    live.set_access(access);
    live.window(
        ThreadInterest::Agent("agent/atlas".into()),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(
            threads.iter().map(|(id, title)| summary(id, title)).collect(),
        ),
        vec![],
        threads.iter().map(|(id, _)| (*id).into()).collect(),
        false,
    );
    (live, Pane::Agent(Some("agent/atlas".into())))
}

fn render(threads: &Threads, pane: &Pane, width: usize) -> Vec<String> {
    mounted(threads, pane, width)
        .lines
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn mounted(threads: &Threads, pane: &Pane, width: usize) -> Doc {
    threads.mount(
        pane,
        width,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &HashMap::new(),
    )
}

/// Optional before/after evidence in the `thread_preview_evidence` convention: with
/// THREAD_PREVIEW_CAPTURE_DIR set, write `<name>.txt` and a 24-bit color `<name>.ansi`.
fn capture(name: &str, screen: &ratatui::buffer::Buffer) {
    use ratatui::style::{Color, Modifier};
    let Ok(dir) = std::env::var("THREAD_PREVIEW_CAPTURE_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let header = "HARNESS, NOT PRODUCTION BINARY PATH — production thread rendering\n";
    let (mut plain, mut ansi) = (String::from(header), String::from(header));
    for row in screen.content.chunks(usize::from(screen.area.width)) {
        let mut last = None;
        for cell in row {
            let style = (cell.fg, cell.bg, cell.modifier);
            if last != Some(style) {
                let mut codes = vec!["0".to_owned()];
                if cell.modifier.contains(Modifier::BOLD) {
                    codes.push("1".into());
                }
                if let Color::Rgb(r, g, b) = cell.fg {
                    codes.push(format!("38;2;{r};{g};{b}"));
                }
                if let Color::Rgb(r, g, b) = cell.bg {
                    codes.push(format!("48;2;{r};{g};{b}"));
                }
                ansi.push_str(&format!("\x1b[{}m", codes.join(";")));
                last = Some(style);
            }
            ansi.push_str(cell.symbol());
            plain.push_str(cell.symbol());
        }
        ansi.push_str("\x1b[0m\n");
        plain.push('\n');
    }
    std::fs::write(dir.join(format!("{name}.txt")), plain).unwrap();
    std::fs::write(dir.join(format!("{name}.ansi")), ansi).unwrap();
}

/// Capture a mounted thread document exactly as its lines and styles are produced.
fn capture_mount(name: &str, threads: &Threads, pane: &Pane, width: u16) {
    let doc = mounted(threads, pane, usize::from(width));
    let area = ratatui::layout::Rect::new(0, 0, width, doc.lines.len().max(1) as u16);
    let mut screen = ratatui::buffer::Buffer::empty(area);
    for (row, line) in doc.lines.iter().enumerate() {
        screen.set_line(0, row as u16, line, width);
    }
    capture(name, &screen);
}

#[test]
fn paused_threads_stay_visibly_open_with_their_own_notice_and_one_view_explanation() {
    let first = "thread/first";
    let second = "thread/second";
    let (mut threads, pane) = listed(
        &[(first, "First follow-up"), (second, "Second follow-up")],
        ThreadAccess::default(),
    );
    threads
        .open
        .insert(pane.key(), [first.into(), second.into()].into());
    for id in [first, second] {
        threads.failure(
            ThreadInterest::Timeline(id.into()),
            THREAD_UPDATES_PAUSED.into(),
            false,
        );
    }
    let lines = render(&threads, &pane, 200);
    for title in ["First follow-up", "Second follow-up"] {
        let heading = lines
            .iter()
            .position(|line| line.starts_with(&format!("▾ {title}")))
            .unwrap_or_else(|| panic!("{title} keeps its requested open chevron: {lines:#?}"));
        assert_eq!(
            lines[heading + 1],
            PAUSED_NOTICE,
            "the paused notice sits under its own heading"
        );
    }
    assert!(!lines.iter().any(|line| line.starts_with('▸')));
    let explanations: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.as_str() == THREAD_UPDATES_PAUSED)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(explanations.len(), 1, "the full reason appears once per view");
    let last_notice = lines.iter().rposition(|line| line == PAUSED_NOTICE).unwrap();
    assert!(explanations[0] > last_notice);
    assert!(!lines.iter().any(|line| line.contains("live-window")));
    capture_mount("p1-2-paused-open-80", &threads, &pane, 80);

    let mut texts = HashMap::new();
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    let collapsed = render(&threads, &pane, 200);
    assert!(collapsed.iter().any(|line| line.starts_with("▸ First follow-up")));
    assert!(collapsed.iter().any(|line| line.starts_with("▾ Second follow-up")));
    capture_mount("p1-2-paused-one-collapsed-80", &threads, &pane, 80);
    assert_eq!(
        collapsed
            .iter()
            .filter(|line| line.as_str() == PAUSED_NOTICE)
            .count(),
        1
    );
}

#[test]
fn the_selected_thread_gutter_remains_visible_while_its_composer_takes_focus() {
    let first = "thread/first";
    let second = "thread/second";
    let (mut threads, pane) = listed(
        &[(first, "First follow-up"), (second, "Second follow-up")],
        ThreadAccess {
            close: true,
            ..ThreadAccess::default()
        },
    );
    for id in [first, second] {
        threads.histories.insert(id.into(), History::default());
    }
    let mut texts = HashMap::new();
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    threads.tap(&pane.key(), thread::Tap::Toggle(second.into()), &mut texts);
    let heading = |lines: &[String], title: &str| {
        lines
            .iter()
            .find(|line| line.contains(&format!("▾ {title}")))
            .unwrap_or_else(|| panic!("{title}: {lines:#?}"))
            .clone()
    };
    let selected = render(&threads, &pane, 100);
    assert!(heading(&selected, "Second follow-up").starts_with("▌▾ Second follow-up"));
    assert!(heading(&selected, "First follow-up").starts_with("▾ First follow-up"));
    assert_eq!(
        selected.iter().filter(|line| line.starts_with('▌')).count(),
        1,
        "exactly one heading is marked"
    );
    capture_mount("p1-4-selected-thread-100", &threads, &pane, 100);

    threads.set_access(ThreadAccess {
        reply: true,
        close: true,
        ..ThreadAccess::default()
    });
    threads.tap(&pane.key(), thread::Tap::Compose(second.into()), &mut texts);
    let composing = render(&threads, &pane, 100);
    assert!(
        heading(&composing, "Second follow-up").starts_with("▌▾ Second follow-up"),
        "composer focus must not hide the selected thread destination"
    );
    threads.key(
        &pane,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &edit::Cursor::default(),
        &mut texts,
    );
    let left = render(&threads, &pane, 100);
    assert!(heading(&left, "Second follow-up").starts_with("▌▾ Second follow-up"));
}

#[test]
fn help_lists_thread_keys_in_lowercase_with_the_selection_context() {
    let ui = super::super::Ui::new(super::super::demo::test_world());
    let draw = |width: u16| {
        let area = ratatui::layout::Rect::new(0, 0, width, 200);
        let mut screen = ratatui::buffer::Buffer::empty(area);
        ui.draw_help(&mut screen, area);
        screen
    };
    // One column, so a wrapped meaning continues on the next row and can be rejoined.
    let screen = draw(90);
    let rows: Vec<String> = screen
        .content
        .chunks(90)
        .map(|row| {
            let row: String = row.iter().map(|cell| cell.symbol()).collect();
            row.trim_matches(|c: char| matches!(c, ' ' | '│' | '┃')).to_owned()
        })
        .collect();
    let joined = rows.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    for expected in [
        "threads",
        "expand or collapse a thread; ▌ marks the expanded thread the keys below act on",
        "Inbox item: start a thread about it",
        "open the marked thread in its own tab",
        "close the thread, or reopen a closed one",
        "rename the thread",
        "mark the visible replies read",
        "leave the thread's message box; the letters above work again",
        "in a thread's message box: quote the agent's latest message",
    ] {
        assert!(joined.contains(expected), "{expected:?} missing from help:\n{}", rows.join("\n"));
    }
    for (key, meaning) in [
        ("t", "Inbox item: start a thread"),
        ("p", "open the marked thread"),
        ("z", "close the thread, or reopen"),
        ("n", "rename the thread"),
        ("v", "mark the visible replies"),
        ("ctrl+r", "in a thread's message box"),
    ] {
        let row = rows.iter().find(|row| row.contains(meaning)).unwrap();
        assert!(row.starts_with(&format!("{key} ")), "lowercase {key:?} keycap: {row:?}");
    }
    capture("p1-4-help-90", &screen);
    capture("p1-4-help-140", &draw(140));
}
