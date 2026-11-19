//! Text that is safe to put in a cell, wrapped the way a reader expects.
//!
//! Every string from the graph passes through `sanitize` before it is measured. A control
//! character or an escape sequence that reaches the terminal moves its cursor, and the frame
//! is then drawn in the wrong cells: the stray digits on the old Fractal TUI's borders.

pub use crate::pane::truncate;
use crate::theme::{self, Theme};
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use std::sync::Arc;
#[path = "semantic_text.rs"]
mod semantic;
pub use semantic::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[path = "ask_markdown.rs"]
mod ask;
pub use ask::markdown as ask_markdown;

/// Ask prose with per-cell safe link targets; activation belongs to the caller.
pub fn ask_markdown_with_links(input: &str, width: usize, base: Style, theme: &Theme) -> (Vec<Line<'static>>, Vec<DrawnLink>) {
    with_drawn_links(ask_markdown(input, width, base, theme))
}

/// Remove escape sequences, control characters and zero-width marks; expand tabs.
pub fn sanitize(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\u{1b}' => {
                // CSI: ESC [ params final; OSC: ESC ] ... BEL or ESC \; others: one more char.
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if ('@'..='~').contains(&next) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        while let Some(next) = chars.next() {
                            if next == '\u{7}' {
                                break;
                            }
                            if next == '\u{1b}' {
                                chars.next();
                                break;
                            }
                        }
                    }
                    Some(_) => {
                        chars.next();
                    }
                    None => {}
                }
            }
            '\n' => output.push('\n'),
            '\t' => output.push_str("    "),
            '\r' => {}
            '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}' | '\u{fe0e}' | '\u{fe0f}' => {}
            character if character.is_control() => {}
            character => output.push(character),
        }
    }
    output
}

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// A run of text with one style: the unit the wrapper moves around.
#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub style: Style,
    pub link: Option<Arc<str>>,
}

pub fn run(text: impl Into<String>, style: Style) -> Run {
    Run {
        text: text.into(),
        style,
        link: None,
    }
}

/// A labeled link with an independent, shared target (never inferred from drawn cells).
pub fn link(label: impl Into<String>, target: impl Into<Arc<str>>, style: Style) -> Run {
    Run { text: label.into(), style, link: Some(target.into()) }
}

/// Word-wrap runs into lines of `width` columns. The first line starts with `first`, later
/// lines with `rest` (a hanging indent). With `fill`, every line is padded to full width
/// in that background, so a tinted block has straight edges.
pub fn wrap(
    runs: &[Run],
    width: usize,
    first: &[Run],
    rest: &[Run],
    fill: Option<Color>,
) -> Vec<Line<'static>> {
    let width = width.max(4);
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let prefix_width = |prefix: &[Run]| {
        prefix
            .iter()
            .map(|run| self::width(&run.text))
            .sum::<usize>()
    };
    let start = |current: &mut Vec<Span<'static>>, used: &mut usize, prefix: &[Run]| {
        for run in prefix {
            current.push(Span { content: run.text.clone().into(), style: with_bg(run.style, fill), link: run.link.clone() });
        }
        *used = prefix_width(prefix);
    };
    let finish = |current: &mut Vec<Span<'static>>,
                  used: usize,
                  lines: &mut Vec<Line<'static>>,
                  joining: Option<Alignment>| {
        if let Some(bg) = fill
            && used < width
        {
            current.push(Span::styled(
                " ".repeat(width - used),
                Style::default().bg(bg),
            ));
        }
        let mut line = Line::from(std::mem::take(current));
        line.alignment = joining;
        lines.push(line);
    };
    // How the line being built continues the one before it (see `continues`).
    let mut joining = None;
    start(&mut current, &mut used, first);
    let mut line_has_words = false;
    let tokens = tokens(runs, fill);
    let mut next = 0;
    while next < tokens.len() {
        match &tokens[next] {
            Token::Newline => {
                // A source newline is a real row boundary, including blank paragraphs.
                // Ratatui spans cannot carry it: Buffer::set_line drops control characters.
                finish(&mut current, used, &mut lines, joining);
                joining = None;
                start(&mut current, &mut used, rest);
                line_has_words = false;
                next += 1;
            }
            Token::Space(style, link) => {
                // A space: keep it unless it would start a line.
                if line_has_words && used < width {
                    push(&mut current, " ", *style, link);
                    used += 1;
                }
                next += 1;
            }
            Token::Piece(..) => {
                // A word is every piece up to the next space or newline, whatever runs it spans,
                // so a styled word and the punctuation after it wrap together.
                let end = tokens[next..]
                    .iter()
                    .position(|token| !matches!(token, Token::Piece(..)))
                    .map_or(tokens.len(), |offset| next + offset);
                let word: Vec<(&str, Style, &Option<Arc<str>>)> = tokens[next..end]
                    .iter()
                    .filter_map(|token| match token {
                        Token::Piece(text, style, link) => Some((text.as_str(), *style, link)),
                        _ => None,
                    })
                    .collect();
                next = end;
                let word_width = word.iter().map(|(text, _, _)| self::width(text)).sum::<usize>();
                if used + word_width > width && line_has_words {
                    trim_trailing_space(&mut current, &mut used);
                    finish(&mut current, used, &mut lines, joining);
                    joining = Some(SPACE_WRAP);
                    start(&mut current, &mut used, rest);
                }
                if used + word_width <= width {
                    for (text, style, link) in word {
                        push(&mut current, text, style, link);
                    }
                    used += word_width;
                } else {
                    // A word longer than the line: break it by character.
                    for (text, style, link) in word {
                        for character in text.chars() {
                            let w = character.width().unwrap_or(0);
                            if used + w > width {
                                finish(&mut current, used, &mut lines, joining);
                                joining = Some(WORD_WRAP);
                                start(&mut current, &mut used, rest);
                            }
                            push(&mut current, &character.to_string(), style, link);
                            used += w;
                        }
                    }
                }
                line_has_words = true;
            }
        }
    }

    finish(&mut current, used, &mut lines, joining);
    lines
}

/// A line that only wraps the one before it, at a space (the space is dropped) or inside a long
/// word, is marked in its alignment: drawing a line (`Buffer::set_line`) ignores alignment, and
/// copying joins the two back into one line, so only real newlines reach the clipboard
/// (Nathan, 2026-10-03).
const SPACE_WRAP: Alignment = Alignment::Left;
const WORD_WRAP: Alignment = Alignment::Right;

/// What joins `line` to the line before it when copied: a space or nothing if it only wraps that
/// line, `None` if it starts a line of its own.
pub fn continues(line: &Line<'_>) -> Option<&'static str> {
    match line.alignment {
        Some(SPACE_WRAP) => Some(" "),
        Some(WORD_WRAP) => Some(""),
        _ => None,
    }
}

impl crate::SelectionLine for Line<'_> {
    fn plain_text(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(plain(self))
    }

    fn continuation(&self) -> Option<&str> {
        continues(self)
    }
}

/// `line` drawn after `prefix` (an edge, an indent), still wrapping the line before it if it did.
pub fn prefixed(prefix: Vec<Span<'static>>, line: Line<'static>) -> Line<'static> {
    let alignment = line.alignment;
    let style = line.style;
    let mut spans = prefix;
    spans.extend(line.spans);
    let mut line = Line::from(spans);
    line.alignment = alignment;
    line.style = style;
    line
}

/// What `wrap` lays out: the runs cut at every space and newline, each piece keeping its style.
enum Token {
    Piece(String, Style, Option<Arc<str>>),
    Space(Style, Option<Arc<str>>),
    Newline,
}

fn tokens(runs: &[Run], fill: Option<Color>) -> Vec<Token> {
    let mut tokens = Vec::new();
    for run in mark_urls(runs).iter() {
        let style = with_bg(run.style, fill);
        for (line_index, logical_line) in run.text.split('\n').enumerate() {
            if line_index > 0 {
                tokens.push(Token::Newline);
            }
            for (index, piece) in logical_line.split(' ').enumerate() {
                if index > 0 {
                    tokens.push(Token::Space(style, run.link.clone()));
                }
                if !piece.is_empty() {
                    tokens.push(Token::Piece(piece.to_owned(), style, run.link.clone()));
                }
            }
        }
    }
    tokens
}

fn trim_trailing_space(current: &mut [Span<'static>], used: &mut usize) {
    if let Some(last) = current.last_mut()
        && last.content.ends_with(' ')
    {
        let trimmed = last.content.trim_end_matches(' ').to_owned();
        *used -= last.content.len() - trimmed.len();
        last.content = trimmed.into();
    }
}

fn push(current: &mut Vec<Span<'static>>, text: &str, style: Style, link: &Option<Arc<str>>) {
    if let Some(last) = current.last_mut()
        && last.style == style && last.link == *link
    {
        last.content.to_mut().push_str(text);
        return;
    }
    current.push(Span { content: text.to_owned().into(), style, link: link.clone() });
}

fn with_bg(style: Style, fill: Option<Color>) -> Style {
    match fill {
        Some(bg) if style.bg.is_none() => style.bg(bg),
        _ => style,
    }
}

/// Split raw HTTP(S) URLs before wrapping, preserving punctuation and existing targets.
fn mark_urls(runs: &[Run]) -> std::borrow::Cow<'_, [Run]> {
    if !runs.iter().any(|run| run.link.is_none() && (run.text.contains("http://") || run.text.contains("https://") || run.text.contains("doc/"))) {
        return std::borrow::Cow::Borrowed(runs);
    }
    let mut marked = Vec::new();
    for run in runs {
        if run.link.is_some() {
            marked.push(run.clone());
            continue;
        }
        let mut rest = run.text.as_str();
        while let Some(start) = rest.find("https://").into_iter().chain(rest.find("http://")).min() {
            if start > 0 { marked.push(self::run(&rest[..start], run.style)); }
            rest = &rest[start..];
            let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'')).unwrap_or(rest.len());
            let mut target = &rest[..end];
            target = target.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            // Balanced URL parentheses belong to the URL; surrounding prose parentheses do not.
            while target.ends_with(')') && target.matches(')').count() > target.matches('(').count() {
                target = &target[..target.len() - 1];
            }
            while target.ends_with(']') && target.matches(']').count() > target.matches('[').count() {
                target = &target[..target.len() - 1];
            }
            marked.push(link(target, target, run.style));
            rest = &rest[target.len()..];
        }
        if !rest.is_empty() { marked.push(self::run(rest, run.style)); }
    }
    let references = marked.iter().filter(|run| run.link.is_none())
        .flat_map(|run| fractal_ui_model::documents::document_links(&run.text))
        .map(|link| link.reference).collect::<Vec<_>>();
    std::borrow::Cow::Owned(mark_references(marked, &references))
}

/// Explicit references share the same metadata path as labeled and raw links.
fn mark_references(runs: Vec<Run>, references: &[String]) -> Vec<Run> {
    if references.is_empty() { return runs; }
    let mut marked = Vec::with_capacity(runs.len());
    for run in runs {
        if run.link.is_some() {
            marked.push(run);
            continue;
        }
        let mut rest = run.text.as_str();
        while let Some((start, reference)) = next_reference(rest, references) {
            if start > 0 { marked.push(self::run(&rest[..start], run.style)); }
            marked.push(link(&rest[start..start + reference.len()], reference, run.style));
            rest = &rest[start + reference.len()..];
        }
        if !rest.is_empty() { marked.push(self::run(rest, run.style)); }
    }
    marked
}

/// The first of `references` written out in `text` as a whole word, the longest where several
/// start at the same place (a pinned reference over its name).
fn next_reference<'a>(text: &str, references: &'a [String]) -> Option<(usize, &'a str)> {
    let name = |c: char| c.is_ascii_alphanumeric() || "_~+-".contains(c);
    let mut best: Option<(usize, &str)> = None;
    for reference in references {
        if reference.is_empty() { continue; }
        let mut from = 0;
        while let Some(offset) = text[from..].find(reference.as_str()) {
            let start = from + offset;
            let end = start + reference.len();
            from = start + 1;
            let before = text[..start].chars().next_back();
            let mut after = text[end..].chars();
            let joined_before = before.is_some_and(|c| name(c) || matches!(c, '/' | '.' | '@'));
            let joined_after = match after.next() {
                Some(c) if name(c) => true,
                Some('.' | '/') => after.next().is_some_and(name),
                _ => false,
            };
            if joined_before || joined_after {
                continue;
            }
            if best.is_none_or(|(at, found)| start < at || start == at && reference.len() > found.len()) {
                best = Some((start, reference.as_str()));
            }
            break;
        }
    }
    best
}

/// Where a link was drawn by `markdown_with_links`: a row, the columns it covers, its address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawnLink {
    pub line: usize,
    pub column: usize,
    pub width: usize,
    pub url: String,
}

/// `markdown`, with where each link was drawn, for a caller that keeps the lines and gives each
/// link's cells their own address rather than looking the text up. `references` are addresses
/// written out in the text (a document's `doc/...` references): each place one is drawn, on every
/// row it wraps to, comes back with its whole address too.
pub fn markdown_with_links(input: &str, width: usize, base: Style, theme: &Theme, references: &[String]) -> (Vec<Line<'static>>, Vec<DrawnLink>) {
    with_drawn_links(markdown_references(input, width, base, theme, references))
}

fn with_drawn_links(lines: Vec<Line<'static>>) -> (Vec<Line<'static>>, Vec<DrawnLink>) {
    let drawn = lines.iter().enumerate().flat_map(|(index, line)| {
        line.drawn_links().into_iter().map(move |mut link| { link.line = index; link })
    }).collect();
    (lines, drawn)
}

/// Inline markdown: **bold**, `code`, [text](url), and an image ![alt](url) as a line that opens
/// it: an image on the web is never fetched just to draw it.
pub fn inline(text: &str, base: Style, theme: &Theme) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    let flush = |plain: &mut String, runs: &mut Vec<Run>| {
        if !plain.is_empty() {
            runs.push(run(std::mem::take(plain), base));
        }
    };
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut plain, &mut runs);
            runs.push(run(&after[..end], base.add_modifier(Modifier::BOLD)));
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            flush(&mut plain, &mut runs);
            runs.push(run(&after[..end], base.fg(theme.teal)));
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix("![")
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            flush(&mut plain, &mut runs);
            let alt = after[..close].trim();
            let label = format!("▣ {}", if alt.is_empty() { "image" } else { alt });
            let url = &after[close + 2..close + 2 + end];
            runs.push(link(label, url.trim(), base.fg(theme.blue).add_modifier(Modifier::UNDERLINED)));
            runs.push(run(" · image link", base.add_modifier(Modifier::DIM)));
            rest = &after[close + 2 + end + 1..];
        } else if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            flush(&mut plain, &mut runs);
            let url = &after[close + 2..close + 2 + end];
            runs.push(link(&after[..close], url.trim(), base.fg(theme.blue).add_modifier(Modifier::UNDERLINED)));
            rest = &after[close + 2 + end + 1..];
        } else {
            let character = rest.chars().next().unwrap_or(' ');
            plain.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    flush(&mut plain, &mut runs);
    runs
}

/// Block markdown, rendered the way pi renders an assistant reply: real headings, bullets
/// with a hanging indent, code set off in a gutter, quotes in a bar, and simple tables.
pub fn markdown(input: &str, width: usize, base: Style, theme: &Theme) -> Vec<Line<'static>> {
    markdown_references(input, width, base, theme, &[])
}

fn inline_references(text: &str, base: Style, theme: &Theme, references: &[String]) -> Vec<Run> {
    mark_references(inline(text, base, theme), references)
}

fn markdown_references(input: &str, width: usize, base: Style, theme: &Theme, references: &[String]) -> Vec<Line<'static>> {
    let input = sanitize(input);
    let source = input.lines().collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let line = source[index];
        let trimmed = line.trim_start();
        if let Some(language) = trimmed.strip_prefix("```") {
            lines.push(Line::from(Span::styled(
                format!("```{}", language.trim()),
                theme.dim(),
            )));
            index += 1;
            // Code keeps its indentation: wrapping drops spaces that start a line, so each line's
            // own indent (past the fence's) goes into its prefix. The prefix repeats on every row
            // the line wraps to, so it takes at most half the width: the code always has room.
            let fence = line.len() - trimmed.len();
            let deepest = (width.max(4) / 2).saturating_sub(2);
            while index < source.len() && !source[index].trim_start().starts_with("```") {
                let code = source[index];
                let spaces = code.len() - code.trim_start_matches(' ').len();
                let indent = run(format!("  {}", " ".repeat(spaces.saturating_sub(fence).min(deepest))), base);
                lines.extend(wrap(
                    &mark_references(vec![run(&code[spaces..], Style::default().fg(theme.subtext1))], references),
                    width,
                    std::slice::from_ref(&indent),
                    std::slice::from_ref(&indent),
                    None,
                ));
                index += 1;
            }
            lines.push(Line::from(Span::styled("```", theme.dim())));
            index += 1;
            continue;
        }
        if trimmed.starts_with('|') {
            let mut rows = Vec::new();
            while index < source.len() && source[index].trim_start().starts_with('|') {
                rows.push(source[index].trim());
                index += 1;
            }
            lines.extend(table(&rows, width, base, theme, references));
            continue;
        }
        if trimmed.is_empty() {
            lines.push(Line::default());
        } else if let Some(heading) = heading(trimmed) {
            let (level, text) = heading;
            let mut style = Style::default()
                .fg(theme.peach)
                .add_modifier(Modifier::BOLD);
            if level == 1 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            lines.extend(wrap(&inline_references(text, style, theme, references), width, &[], &[], None));
        } else if matches!(trimmed, "---" | "***" | "___") {
            lines.push(Line::from(Span::styled(
                "─".repeat(width.min(80)),
                theme::fg(theme.surface2),
            )));
        } else if let Some(quote) = trimmed.strip_prefix("> ").or(trimmed.strip_prefix('>')) {
            let style = base.fg(theme.subtext0).add_modifier(Modifier::ITALIC);
            let bar = [run("│ ", theme::fg(theme.surface2))];
            lines.extend(wrap(&inline_references(quote, style, theme, references), width, &bar, &bar, None));
        } else if let Some((marker, item)) = list_item(trimmed) {
            let indent = " ".repeat(line.len() - trimmed.len());
            let first = [
                run(indent.clone(), base),
                run(marker.clone(), theme::fg(theme.lavender)),
            ];
            let rest = [run(indent + &" ".repeat(width_of(&marker)), base)];
            lines.extend(wrap(&inline_references(item, base, theme, references), width, &first, &rest, None));
        } else {
            lines.extend(wrap(&inline_references(trimmed, base, theme, references), width, &[], &[], None));
        }
        index += 1;
    }
    while lines.last().is_some_and(|line| line.width() == 0) {
        lines.pop();
    }
    lines
}

fn width_of(text: &str) -> usize {
    width(text)
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    (level > 0 && level <= 6 && line[level..].starts_with(' '))
        .then(|| (level, line[level..].trim()))
}

fn list_item(line: &str) -> Option<(String, &str)> {
    if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(("• ".into(), item));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits < 4 && line[digits..].starts_with(". ") {
        return Some((format!("{} ", &line[..digits + 1]), &line[digits + 2..]));
    }
    None
}

/// The narrowest a column is squeezed to before the table is shown as stacked records instead.
const MIN_COLUMN: usize = 6;

/// A markdown table. It is a grid when it fits; when a cell is long or the pane is narrow the
/// columns are narrowed and their cells wrap inside them, so it is still a table. Only when even
/// the narrowest columns cannot fit is each row shown as a record, one `heading: value` per cell.
/// Cells are read as inline markdown: bold, code and links keep their look and stay clickable.
fn table(rows: &[&str], width: usize, base: Style, theme: &Theme, references: &[String]) -> Vec<Line<'static>> {
    let cells = rows
        .iter()
        .filter(|row| !row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')))
        .map(|row| {
            row.trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let columns = cells.iter().map(Vec::len).max().unwrap_or(0);
    let runs = cells
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index == 0 {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            };
            (0..columns)
                .map(|column| inline_references(row.get(column).map(String::as_str).unwrap_or(""), style, theme, references))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let natural = (0..columns)
        .map(|column| {
            runs.iter()
                .map(|row| row[column].iter().map(|run| width_of(&run.text)).sum::<usize>())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let budget = width.saturating_sub(columns.saturating_sub(1) * 3);
    let Some(widths) = fit_columns(&natural, budget) else {
        return records(&cells, &runs, width, base, theme);
    };
    let mut lines = Vec::new();
    for (index, row) in runs.iter().enumerate() {
        let wrapped = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| {
                if cell.is_empty() {
                    vec![Line::default()]
                } else {
                    wrap(cell, *w, &[], &[], None)
                }
            })
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let mut spans = Vec::new();
            for (column, w) in widths.iter().enumerate() {
                if column > 0 {
                    spans.push(Span::styled(" │ ", theme::fg(theme.surface2)));
                }
                let mut used = 0;
                if let Some(line) = wrapped[column].get(line_index) {
                    used = line.width();
                    spans.extend(line.spans.iter().cloned());
                }
                // The last column's trailing spaces would only be copied; the others hold the grid.
                if column + 1 < widths.len() {
                    spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), base));
                }
            }
            // Nothing after the last column but what is in it: no trailing spaces to copy.
            while let Some(last) = spans.last_mut() {
                let kept = last.content.trim_end().len();
                if kept == 0 {
                    spans.pop();
                } else {
                    last.content = last.content[..kept].to_owned().into();
                    break;
                }
            }
            lines.push(Line::from(spans));
        }
        if index == 0 {
            let rule = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─");
            lines.push(Line::from(Span::styled(rule, theme::fg(theme.surface2))));
        }
    }
    lines
}

/// Column widths that fit `budget`: each column's natural width when they all fit, else the
/// widest columns give up space first, down to `MIN_COLUMN`. `None` when even that is too wide.
fn fit_columns(natural: &[usize], budget: usize) -> Option<Vec<usize>> {
    if natural.iter().sum::<usize>() <= budget {
        return Some(natural.to_vec());
    }
    let mut widths = natural
        .iter()
        .map(|natural| (*natural).min(MIN_COLUMN))
        .collect::<Vec<_>>();
    let mut spare = budget.checked_sub(widths.iter().sum::<usize>())?;
    while spare > 0 {
        // The column that still wants the most gets the next cell of width.
        let Some((column, _)) = natural
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(column, (natural, width))| (column, natural - width))
            .filter(|(_, wants)| *wants > 0)
            .max_by_key(|(column, wants)| (*wants, std::cmp::Reverse(*column)))
        else {
            break;
        };
        widths[column] += 1;
        spare -= 1;
    }
    Some(widths)
}

/// A table too narrow for any grid: each row as a record, the heading before each value.
fn records(
    cells: &[Vec<String>],
    runs: &[Vec<Vec<Run>>],
    width: usize,
    base: Style,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(headings) = cells.first() else {
        return lines;
    };
    for (row, row_runs) in runs.iter().enumerate().skip(1) {
        if row > 1 {
            lines.push(Line::default());
        }
        for (column, cell) in row_runs.iter().enumerate() {
            let heading = headings.get(column).map(String::as_str).unwrap_or("");
            let first = [run(format!("{heading}: "), theme.dim())];
            lines.extend(wrap(cell, width, &first, &[run("  ", base)], None));
        }
    }
    if runs.len() == 1 {
        lines.extend(wrap(
            &[run(headings.join(" · "), base.add_modifier(Modifier::BOLD))],
            width,
            &[],
            &[],
            None,
        ));
    }
    lines
}

/// Plain text of a line, for copying.
pub fn plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// One styled line as terminal text: 24-bit colors, bold, italic, underline and dim, reset at the
/// end. For a program with no screen of its own, such as `st documents get`.
pub fn ansi(line: &Line<'_>) -> String {
    let mut out = String::new();
    for span in &line.spans {
        let style = line.style.patch(span.style);
        let mut codes: Vec<String> = Vec::new();
        for (add, code) in [
            (Modifier::BOLD, "1"),
            (Modifier::DIM, "2"),
            (Modifier::ITALIC, "3"),
            (Modifier::UNDERLINED, "4"),
            (Modifier::REVERSED, "7"),
            (Modifier::CROSSED_OUT, "9"),
        ] {
            if style.add_modifier.contains(add) {
                codes.push(code.to_owned());
            }
        }
        if let Some(Color::Rgb(r, g, b)) = style.fg {
            codes.push(format!("38;2;{r};{g};{b}"));
        }
        if let Some(Color::Rgb(r, g, b)) = style.bg {
            codes.push(format!("48;2;{r};{g};{b}"));
        }
        if codes.is_empty() {
            out.push_str(&span.content);
        } else {
            out.push_str(&format!("\x1b[{}m{}\x1b[0m", codes.join(";"), span.content));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_url_segments_retain_the_full_target_style_and_width() {
        let target = "https://example.com/界/path?q=long-value";
        let style = Style::default().fg(Color::Green).underline_color(Color::Magenta);
        let lines = wrap(&[run(format!("Read ({target})."), style)], 12, &[], &[], Some(Color::Black));
        let linked = lines.iter().flat_map(|line| &line.spans).filter(|span| span.link.is_some()).collect::<Vec<_>>();
        assert!(linked.len() > 2);
        assert_eq!(linked.iter().map(|span| span.content.as_ref()).collect::<String>(), target);
        assert!(linked.iter().all(|span| span.link.as_deref() == Some(target) && span.style == style.bg(Color::Black)));
        assert!(lines.iter().all(|line| line.width() == 12));
        for line in &lines {
            let native = line.to_ratatui();
            assert_eq!(native.width(), line.width());
            assert_eq!(native.style, line.style);
            assert_eq!(native.alignment, line.alignment);
        }
    }

    #[test]
    fn equal_labels_keep_distinct_targets_without_style_encoding() {
        let style = Style::default().underline_color(Color::Rgb(1, 2, 3));
        let lines = wrap(&[link("same", "https://a", style), link("same", "https://b", style)], 4, &[], &[], None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].link.as_deref(), Some("https://a"));
        assert_eq!(lines[1].spans[0].link.as_deref(), Some("https://b"));
        assert!(lines.iter().flat_map(|line| &line.spans).all(|span| span.style == style));
    }

    #[test]
    fn automatic_document_references_keep_pinned_targets_across_wraps() {
        let target = format!("doc/example/plan@{}", "a".repeat(64));
        let lines = wrap(&[run(format!("Read {target}."), Style::default())], 16, &[], &[], None);
        let linked = lines.iter().flat_map(|line| &line.spans).filter(|span| span.link.is_some()).collect::<Vec<_>>();
        assert!(linked.len() > 4);
        assert_eq!(linked.iter().map(|span| span.content.as_ref()).collect::<String>(), target);
        assert!(linked.iter().all(|span| span.link.as_deref() == Some(target.as_str())));
    }

    #[test]
    fn tables_prefixes_and_document_appends_preserve_link_ranges() {
        let theme = Theme::default();
        let source = "| key | value |\n|---|---|\n| row | [long label wraps here](https://example.com/target) |";
        for width in [10, 24, 80] {
            let (lines, links) = markdown_with_links(source, width, Style::default(), &theme, &[]);
            assert!(!links.is_empty());
            assert!(links.iter().all(|link| link.url == "https://example.com/target"));
            let mut child = crate::doc::Doc::<()>::new();
            child.lines(lines.clone());
            let mut parent = crate::doc::Doc::<()>::new();
            parent.append(child, 3);
            for (line, prefixed) in lines.iter().zip(&parent.lines) {
                assert_eq!(prefixed.width(), line.width() + 3);
                let expected = line.drawn_links().into_iter().map(|mut link| { link.column += 3; link }).collect::<Vec<_>>();
                assert_eq!(prefixed.drawn_links(), expected);
            }
        }
        let line = Line::from(Span { content: "label".into(), style: theme.dim(), link: Some(Arc::from("https://target")) })
            .style(Style::default().bg(Color::Blue)).alignment(WORD_WRAP);
        let prefixed = prefixed(vec![Span::raw("│ ")], line.clone());
        assert_eq!(prefixed.style, line.style);
        assert_eq!(prefixed.alignment, line.alignment);
        assert_eq!(prefixed.drawn_links()[0].column, 2);
        assert_eq!(Line::from(line.to_ratatui()).width(), line.width());
    }

    #[test]
    fn sanitize_drops_escapes_and_zero_width_marks() {
        assert_eq!(
            sanitize("a\u{1b}[38;5;97mb\u{1b}[0m\tc\u{200b}d⚠\u{fe0f}"),
            "ab    cd⚠"
        );
        assert_eq!(sanitize("x\u{1b}]8;;http://a\u{7}y"), "xy");
    }

    #[test]
    fn code_keeps_its_indentation_past_the_fence() {
        let theme = Theme::default();
        let source = "```kdl\nstep \"plant\" {\n  document \"doc/a\"\n}\n```\n- Run it:\n    ```sh\n    garden plant\n      --bed north\n    ```";
        let text = markdown(source, 40, Style::default(), &theme).iter().map(plain).collect::<Vec<_>>();
        assert_eq!(
            text,
            vec!["```kdl", "  step \"plant\" {", "    document \"doc/a\"", "  }", "```", "• Run it:", "```sh", "  garden plant", "    --bed north", "```"]
        );
    }

    #[test]
    fn drawn_links_keep_their_own_addresses_and_their_look() {
        let theme = crate::tests::theme();
        let input = "[the details](doc/a) and [the details](doc/b)\n\n![north bed](https://example.com/n.png)";
        let (lines, links) = markdown_with_links(input, 80, Style::default(), &theme, &[]);
        assert_eq!(
            links,
            [
                DrawnLink { line: 0, column: 0, width: 11, url: "doc/a".into() },
                DrawnLink { line: 0, column: 16, width: 11, url: "doc/b".into() },
                DrawnLink { line: 2, column: 0, width: 11, url: "https://example.com/n.png".into() },
            ]
        );
        assert_eq!(lines, markdown(input, 80, Style::default(), &theme), "drawn exactly as without recording");
        assert!(lines.iter().flat_map(|line| &line.spans).all(|span| span.style.underline_color.is_none()));
    }

    #[test]
    fn a_written_out_reference_keeps_its_whole_address_on_every_row_it_wraps_to() {
        let theme = crate::tests::theme();
        let pinned = format!("doc/garden/plan@{}", "c".repeat(64));
        let references = vec!["doc/garden/plan".to_owned(), pinned.clone(), "doc/garden/plan-b".to_owned()];
        let input = format!("Read {pinned} first.\n\n```\nopen {pinned}\n```\n\nNot doc/garden/planner, but doc/garden/plan-b.");
        let (lines, links) = markdown_with_links(&input, 24, Style::default(), &theme, &references);
        let rows = lines.iter().map(plain).collect::<Vec<_>>();
        let pinned_rows = links.iter().filter(|link| link.url == pinned).collect::<Vec<_>>();
        let drawn = |link: &&DrawnLink| rows[link.line].chars().skip(link.column).take(link.width).collect::<String>();
        // In the text and in the code block: each time the reference wraps over several rows,
        // and every row of it opens the whole pinned address.
        let joined = pinned_rows.iter().map(drawn).collect::<String>();
        assert_eq!(joined, format!("{pinned}{pinned}"), "{rows:?}");
        assert!(pinned_rows.len() >= 4, "{rows:?}");
        assert!(!links.iter().any(|link| link.url == "doc/garden/plan"), "never the name alone: {links:?}");
        assert_eq!(links.iter().filter(|link| link.url == "doc/garden/plan-b").count(), 1, "planner is another word: {links:?}");
        // Marking a reference splits its row into more spans; every cell looks the same.
        let cells = |lines: &[Line<'static>]| {
            lines
                .iter()
                .map(|line| (line.alignment, line.spans.iter().flat_map(|span| span.content.chars().map(move |c| (c, span.style))).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        };
        assert_eq!(cells(&lines), cells(&markdown(&input, 24, Style::default(), &theme)), "drawn exactly as without recording");
    }

    #[test]
    fn a_callers_own_underline_colour_is_never_read_as_a_link() {
        let theme = crate::tests::theme();
        let base = Style::default().add_modifier(Modifier::UNDERLINED).underline_color(Color::Rgb(0, 0, 0));
        let input = "plain [link](doc/a) and doc/b";
        let (lines, links) = markdown_with_links(input, 80, base, &theme, &["doc/b".to_owned()]);
        assert_eq!(links.iter().map(|link| link.url.as_str()).collect::<Vec<_>>(), ["doc/a", "doc/b"]);
        assert!(lines.iter().flat_map(|line| &line.spans).all(|span| span.style.underline_color == base.underline_color));
    }

    #[test]
    fn deep_code_indentation_never_crowds_out_the_code() {
        let theme = Theme::default();
        // Deeper than the width: the indent stops at half of it, and every character of the code is still drawn.
        let source = format!("```\n{}plant the north bed\n```", " ".repeat(30));
        let text = markdown(&source, 20, Style::default(), &theme).iter().map(plain).collect::<Vec<_>>();
        assert!(text.iter().all(|line| width(line) <= 20), "{text:?}");
        assert!(text[1..text.len() - 1].iter().all(|line| line.starts_with("          ") && !line.starts_with("           ")), "{text:?}");
        assert_eq!(text[1..text.len() - 1].iter().map(|line| line.trim()).collect::<Vec<_>>().join(" "), "plant the north bed");
        // A hostile line (a huge indent and a huge word) stays bounded: rows are as many as the code needs, not more.
        let source = format!("```\n{}{}\n```", " ".repeat(100_000), "x".repeat(10_000));
        let lines = markdown(&source, 40, Style::default(), &theme);
        let drawn: usize = lines.iter().map(|line| width(&plain(line))).sum();
        assert!(lines.len() < 10_000 / 20 + 4, "{} rows", lines.len());
        assert!(drawn < 2 * 40 * lines.len(), "{drawn} columns over {} rows", lines.len());
    }

    #[test]
    fn wrap_uses_a_hanging_indent_and_never_exceeds_width() {
        let lines = wrap(
            &[run("one two three four five six", Style::default())],
            12,
            &[run("• ", Style::default())],
            &[run("  ", Style::default())],
            None,
        );
        let text = lines.iter().map(plain).collect::<Vec<_>>();
        assert_eq!(text, vec!["• one two", "  three four", "  five six"]);
        assert!(lines.iter().all(|line| line.width() <= 12));
    }

    #[test]
    fn a_word_spread_over_runs_wraps_as_one_word() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let lines = wrap(
            &[run("See ", Style::default()), run("doc/a", bold), run(". Then go", Style::default())],
            9,
            &[],
            &[],
            None,
        );
        let text = lines.iter().map(plain).collect::<Vec<_>>();
        assert_eq!(text, vec!["See", "doc/a.", "Then go"], "the full stop stays with its word");
        assert_eq!(lines[1].spans.iter().map(|span| (span.content.as_ref(), span.style)).collect::<Vec<_>>(), vec![("doc/a", bold), (".", Style::default())]);
        // A word longer than the line still breaks by character, keeping each piece's style.
        let lines = wrap(&[run("ab", bold), run("cdefg", Style::default())], 4, &[], &[], None);
        assert_eq!(lines.iter().map(plain).collect::<Vec<_>>(), vec!["abcd", "efg"]);
        assert_eq!(lines[0].spans[0].style, bold);
    }

    #[test]
    fn recording_a_reference_never_moves_where_a_line_wraps() {
        let theme = crate::tests::theme();
        for width in 4..14 {
            let input = "See doc/a. Then doc/a, then (doc/a).";
            let (lines, links) = markdown_with_links(input, width, Style::default(), &theme, &["doc/a".to_owned()]);
            let rows = lines.iter().map(plain).collect::<Vec<_>>();
            assert_eq!(rows, markdown(input, width, Style::default(), &theme).iter().map(plain).collect::<Vec<_>>(), "width {width}");
            assert!(links.iter().all(|link| link.url == "doc/a"), "{links:?}");
        }
    }

    #[test]
    fn filled_lines_are_padded_to_full_width() {
        let lines = wrap(
            &[run("hi", Style::default())],
            10,
            &[],
            &[],
            Some(Color::Red),
        );
        assert_eq!(lines[0].width(), 10);
    }

    #[test]
    fn an_image_in_markdown_is_a_line_that_opens_it_not_a_bang() {
        let theme = Theme::default();
        let runs = inline("Plant it?  ![north bed](https://example.com/north-bed.png)", Style::default(), &theme);
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "Plant it?  ▣ north bed · image link");
    }

    #[test]
    fn markdown_renders_lists_headings_and_tables() {
        let lines = markdown(
            "## Head\n\n- **a** b\n\n| k | v |\n|---|---|\n| x | y |",
            40,
            Style::default(),
            &crate::tests::theme(),
        );
        let text = lines.iter().map(plain).collect::<Vec<_>>();
        assert_eq!(text[0], "Head");
        assert_eq!(text[2], "• a b");
        assert!(text.iter().any(|line| line.starts_with("k │ v")));
    }

    /// The "Factory / Now / Target" table from a real conversation: long cells and links.
    const FACTORY: &str = "| Factory | Now | Target |\n|---|---|---|\n\
        | Merges an hour while PRs wait | 2.8 | 4 |\n\
        | Merges this hour | 2: [#2076](https://example.com/pull/2076), [#2090](https://example.com/pull/2090) | — |\n\
        | Merge queue | 12 deep; each group carries one PR and takes 30–60 min | — |\n\
        | Missions stalled | 9 | 0 |";

    fn rendered(input: &str, width: usize) -> Vec<String> {
        markdown(input, width, Style::default(), &crate::tests::theme())
            .iter()
            .map(plain)
            .collect()
    }

    #[test]
    fn a_table_with_long_cells_is_still_a_grid_that_fits_the_pane() {
        for width in [60, 80, 100, 140] {
            let text = rendered(FACTORY, width);
            assert!(
                text.iter().all(|line| self::width(line) <= width),
                "{width}: {text:#?}"
            );
            // Still a table: a header, a rule and a separator on every row.
            assert!(text[0].contains(" │ ") && text[0].starts_with("Factory"), "{width}: {text:#?}");
            assert!(text[1].contains("─┼─"), "{width}: {text:#?}");
            assert!(text[2..].iter().all(|line| line.contains(" │ ") || line.starts_with(' ')), "{width}: {text:#?}");
            // Links read as their text; the address is not printed into the cell.
            let all = text.join("\n");
            assert!(all.contains("#2076") && all.contains("#2090"), "{width}: {text:#?}");
            assert!(!all.contains("]("), "{width}: {text:#?}");
            assert!(!all.contains("example.com"), "{width}: {text:#?}");
            // Nothing is lost to the wrapping.
            for word in ["Merges", "queue", "stalled", "30–60", "deep;"] {
                assert!(all.contains(word), "{width}: missing {word}: {text:#?}");
            }
        }
    }

    #[test]
    fn a_table_that_fits_is_unchanged_and_wide_ones_do_not_pad_their_last_column() {
        let text = rendered("| k | value |\n|---|---|\n| x | y |", 40);
        assert_eq!(text, ["k │ value", "──┼──────", "x │ y"]);
        let wide = rendered(FACTORY, 60);
        assert!(wide.iter().all(|line| !line.ends_with(' ')), "{wide:#?}");
    }

    #[test]
    fn a_table_too_narrow_for_any_grid_is_stacked_as_records() {
        let text = rendered(FACTORY, 14);
        assert!(text.iter().all(|line| self::width(line) <= 14), "{text:#?}");
        assert!(text.iter().any(|line| line.starts_with("Now: ")), "{text:#?}");
        assert!(text.iter().any(|line| line.starts_with("Target: ")), "{text:#?}");
        assert!(text.contains(&String::new()), "records are told apart by a blank line: {text:#?}");
        assert!(!text.iter().any(|line| line.contains("─┼─")), "{text:#?}");
    }

    #[test]
    fn column_widths_give_up_space_widest_first_and_never_below_the_minimum() {
        assert_eq!(fit_columns(&[4, 5], 20), Some(vec![4, 5]));
        let squeezed = fit_columns(&[4, 60, 30], 50).unwrap();
        assert_eq!(squeezed.iter().sum::<usize>(), 50);
        assert_eq!(squeezed[0], 4, "a short column keeps its width");
        assert!(squeezed[1] > squeezed[2], "{squeezed:?}");
        assert_eq!(fit_columns(&[40, 40, 40], 12), None);
    }

    #[test]
    fn ansi_writes_colors_and_modifiers_and_resets() {
        let line = Line::from(vec![
            Span::styled("hi", Style::default().fg(Color::Rgb(1, 2, 3)).add_modifier(Modifier::BOLD)),
            Span::raw(" plain"),
        ]);
        assert_eq!(ansi(&line), "\x1b[1;38;2;1;2;3mhi\x1b[0m plain");
    }
}
