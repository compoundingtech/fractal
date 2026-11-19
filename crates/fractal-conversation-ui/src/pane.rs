use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Application actions requested by a conversation pane, never performed by the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneIntent {
    Send(String),
    Expand(String),
    Open(String),
    LoadOlder,
    /// Show an image a message carries.
    Image(crate::entry::MailImage),
}

#[derive(Default, Clone, Copy, Debug)]
pub struct PaneState {
    pub top: usize,
    pub follow: bool,
    pub seen: usize,
    pub unseen: usize,
}

impl PaneState {
    pub fn reconcile(&mut self, total: usize, height: usize) {
        let max_top = total.saturating_sub(height);
        if self.follow {
            self.top = max_top;
            self.unseen = 0;
        } else {
            self.top = self.top.min(max_top);
            if total > self.seen {
                self.unseen += total - self.seen;
            }
        }
        self.seen = total;
    }

    pub fn scroll(&mut self, delta: isize, total: usize, height: usize, follow_at_end: bool) {
        let max_top = total.saturating_sub(height);
        self.top = self.top.saturating_add_signed(delta).min(max_top);
        self.follow = self.top >= max_top && follow_at_end;
        if self.follow {
            self.unseen = 0;
        }
    }

    pub fn follow_latest(&mut self) {
        self.follow = true;
        self.unseen = 0;
    }
}

/// Selection coordinates are document lines and display columns, not byte offsets.
#[derive(Clone, Debug)]
pub struct Selection {
    pub pane: String,
    pub anchor: (usize, u16),
    pub head: (usize, u16),
}

/// A displayed line's text and whether wrapping joined it to the previous line.
/// Implement this for an embedding UI's line type; the `ratatui` feature supplies its `Line`
/// implementation. Plain strings start their own lines.
#[derive(Debug)]
pub struct CopyFragment {
    pub text: String,
    pub link: Option<Arc<str>>,
    pub column: Option<usize>,
}

pub trait SelectionLine {
    fn plain_text(&self) -> Cow<'_, str>;

    /// `Some(" ")` for wrapping at a space, `Some("")` inside a word, `None` for a real newline.
    fn continuation(&self) -> Option<&str> {
        None
    }
    fn copy_fragments(&self, from: usize, to: usize) -> Vec<CopyFragment> {
        vec![CopyFragment { text: columns(&self.plain_text(), from, to), link: None, column: None }]
    }
    fn decoration_only(&self) -> bool { false }
    /// Stable source row identity and number of cells, when this is part of a table.
    fn table_row(&self) -> Option<(usize, usize)> { None }
    fn cell_continuation(&self, _column: usize) -> Option<&str> { None }
    fn table_header(&self) -> Vec<Vec<CopyFragment>> { Vec::new() }
}

impl SelectionLine for str {
    fn plain_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl SelectionLine for String {
    fn plain_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl<T: SelectionLine + ?Sized> SelectionLine for &T {
    fn plain_text(&self) -> Cow<'_, str> {
        (*self).plain_text()
    }

    fn continuation(&self) -> Option<&str> {
        (*self).continuation()
    }
    fn copy_fragments(&self, from: usize, to: usize) -> Vec<CopyFragment> { (*self).copy_fragments(from, to) }
    fn decoration_only(&self) -> bool { (*self).decoration_only() }
    fn table_row(&self) -> Option<(usize, usize)> { (*self).table_row() }
    fn cell_continuation(&self, column: usize) -> Option<&str> { (*self).cell_continuation(column) }
    fn table_header(&self) -> Vec<Vec<CopyFragment>> { (*self).table_header() }
}

/// Equal targets may be distinct occurrences; wrapping shares the original target allocation.
pub(crate) fn same_link(left: &Option<Arc<str>>, right: &Option<Arc<str>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

#[derive(Default)]
struct CopyText {
    text: String,
    label: String,
    target: Option<Arc<str>>,
    separator: Cow<'static, str>,
}

impl CopyText {
    fn flush_link(&mut self, targets: bool) {
        self.text.push_str(&self.label);
        if targets && let Some(target) = &self.target && self.label != target.as_ref() {
            self.text.push_str(" (");
            self.text.push_str(target);
            self.text.push(')');
        }
        self.label.clear();
        self.target = None;
    }
    fn append(&mut self, fragment: CopyFragment, targets: bool) {
        if !same_link(&fragment.link, &self.target) {
            self.flush_link(targets);
            self.text.push_str(&self.separator);
        } else if self.target.is_some() {
            self.label.push_str(&self.separator);
        } else {
            self.text.push_str(&self.separator);
        }
        self.separator = Cow::Borrowed("");
        self.target = fragment.link;
        if self.target.is_some() { self.label.push_str(&fragment.text); }
        else { self.text.push_str(&fragment.text); }
    }
    fn join(&mut self, separator: &str) {
        if self.separator.is_empty() {
            self.separator = match separator {
                "" => Cow::Borrowed(""),
                " " => Cow::Borrowed(" "),
                "\n" => Cow::Borrowed("\n"),
                other => Cow::Owned(other.to_owned()),
            };
        } else {
            self.separator.to_mut().push_str(separator);
        }
    }
    fn finish(mut self, targets: bool) -> String {
        self.flush_link(targets);
        self.text
    }
}

impl Selection {
    /// Copy source content, excluding only explicitly marked presentation cells.
    pub fn text<L: SelectionLine>(&self, lines: &[L]) -> String {
        self.extract(lines, false)
    }

    /// Copy source prose and full link targets, once per contiguous link occurrence.
    pub fn text_with_link_targets<L: SelectionLine>(&self, lines: &[L]) -> String {
        self.extract(lines, true)
    }

    fn extract<L: SelectionLine>(&self, lines: &[L], targets: bool) -> String {
        let (start, end) = order(self.anchor, self.head);
        let mut output = Vec::new();
        let mut prose: Option<CopyText> = None;
        let mut table: Option<(usize, Vec<CopyText>)> = None;
        let flush_table = |table: &mut Option<(usize, Vec<CopyText>)>, output: &mut Vec<String>| {
            if let Some((_, cells)) = table.take() {
                output.push(cells.into_iter().map(|cell| cell.finish(targets)).collect::<Vec<_>>().join("\t"));
            }
        };
        for index in start.0..=end.0.min(lines.len().saturating_sub(1)) {
            let Some(line) = lines.get(index) else { break };
            if line.decoration_only() { continue; }
            let from = if index == start.0 { usize::from(start.1) } else { 0 };
            let to = if index == end.0 { usize::from(end.1) + 1 } else { usize::MAX };
            let fragments = line.copy_fragments(from, to);
            if let Some((id, columns)) = line.table_row() {
                if let Some(previous) = prose.take() { output.push(previous.finish(targets)); }
                if table.as_ref().is_some_and(|(previous, _)| *previous != id) { flush_table(&mut table, &mut output); }
                if table.is_none() && from == 0 {
                    let header = line.table_header();
                    if !header.is_empty() {
                        output.push(header.into_iter().map(|fragments| {
                            let mut cell = CopyText::default();
                            for fragment in fragments { cell.append(fragment, targets); }
                            cell.finish(targets)
                        }).collect::<Vec<_>>().join("\t"));
                    }
                }
                let (_, cells) = table.get_or_insert_with(|| (id, (0..columns).map(|_| CopyText::default()).collect()));
                let mut previous_column = None;
                for fragment in fragments {
                    let Some(column) = fragment.column else { continue };
                    let Some(cell) = cells.get_mut(column) else { continue };
                    if previous_column != Some(column) && (!cell.text.is_empty() || !cell.label.is_empty()) {
                        cell.join(line.cell_continuation(column).unwrap_or("\n"));
                    }
                    previous_column = Some(column);
                    cell.append(fragment, targets);
                }
            } else {
                flush_table(&mut table, &mut output);
                match (line.continuation(), prose.as_mut()) {
                    (Some(separator), Some(previous)) => previous.join(separator),
                    _ => {
                        if let Some(previous) = prose.take() { output.push(previous.finish(targets)); }
                        prose = Some(CopyText::default());
                    }
                }
                if let Some(current) = prose.as_mut() {
                    for fragment in fragments { current.append(fragment, targets); }
                }
            }
        }
        flush_table(&mut table, &mut output);
        if let Some(previous) = prose { output.push(previous.finish(targets)); }
        output.join("\n")
    }
}

/// Keep one state per embedding surface; keys retain scroll and drafts across pane switches.
#[derive(Default)]
pub struct State {
    pub panes: RefCell<HashMap<String, PaneState>>,
    pub expanded: HashSet<String>,
    pub drafts: HashMap<String, String>,
    pub selection: Option<Selection>,
}

impl State {
    pub fn expand(&mut self, id: String) -> PaneIntent {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id.clone());
        }
        PaneIntent::Expand(id)
    }

    /// Does not clear the draft: the caller clears it only after accepting the send intent.
    pub fn send(&self, key: &str) -> Option<PaneIntent> {
        self.drafts
            .get(key)
            .filter(|draft| !draft.trim().is_empty())
            .map(|draft| PaneIntent::Send(draft.clone()))
    }

    pub fn load_older(&self, has_more: bool) -> Option<PaneIntent> {
        has_more.then_some(PaneIntent::LoadOlder)
    }
}

pub fn order(a: (usize, u16), b: (usize, u16)) -> ((usize, u16), (usize, u16)) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The part of `line` between two display columns.
pub fn columns(line: &str, from: usize, to: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut column = 0;
    let mut out = String::new();
    for character in line.chars() {
        let width = character.width().unwrap_or(0);
        if column >= from && column < to {
            out.push(character);
        }
        column += width;
    }
    out
}

/// Cut to `max` columns, ending in `…` when anything was dropped.
pub fn truncate(text: &str, max: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if UnicodeWidthStr::width(text) <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut used = 0;
    for character in text.chars() {
        let w = character.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        used += w;
        output.push(character);
    }
    output.push('…');
    output
}
