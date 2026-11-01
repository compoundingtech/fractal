//! Documents, read-only: one document's page, the documents list, and the documents a mission and
//! its steps name. Every read goes through the client's `document.get`, `documents.list` and
//! `document-references.get`, and every page is `fractal_ui_model::documents`, the model web and the
//! phone read too. A document is shown as its author wrote it: rendered, never filtered.

use super::doc::{Doc, DocExt, Hit, Target};
use super::{text, theme};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use fractal_client::{ClientError, DocumentContent, DocumentReferences, DocumentsPage};
use fractal_ui_model::documents::{DocumentList, DocumentTrail, DocumentView, Unavailable, parse_reference};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A document page or the list asks again this long after it could not be read: a document not
/// synced yet shows once it arrives, without a key.
const RETRY: Duration = Duration::from_secs(15);
/// At most this many steps of each mission are asked which documents they name.
pub(crate) const STEPS: usize = 40;
/// A page of the documents list.
pub(crate) const PAGE: usize = 100;

/// What a click on a document page does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Back one document on the trail.
    Back,
    /// Read the document or the list again.
    Retry,
    /// The list's next page.
    More,
}

/// A read stui wants; the live loop sends it and hands the answer back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    Get(String),
    List { prefix: Option<String>, cursor: Option<String> },
    References(String),
}

/// Which `Unavailable` a failed read means.
pub(crate) fn unavailable(error: &ClientError) -> Unavailable {
    match error {
        ClientError::Api(code, _, _) => {
            let code = serde_json::to_value(code).ok();
            Unavailable::from_error(None, code.as_ref().and_then(|code| code.as_str()).unwrap_or_default())
        }
        _ => Unavailable::RemoteUnavailable,
    }
}

/// A `doc/…` reference a person can open, or `None`.
pub(crate) fn openable(reference: &str) -> Option<&str> {
    let reference = reference.trim();
    parse_reference(reference).map(|_| reference)
}

/// Whether a reference found at this place is only part of a longer word: `mydoc/x`, or
/// `doc/x` inside `doc/x-more`.
pub(crate) fn inside_word(before: &str, after: &str) -> bool {
    let name = |c: char| c.is_ascii_alphanumeric() || "_~+-".contains(c);
    let joined_before = before.chars().next_back().is_some_and(|c| name(c) || c == '/' || c == '.' || c == '@');
    let mut rest = after.chars();
    let joined_after = match rest.next() {
        Some(c) if name(c) => true,
        Some('.' | '/') => rest.next().is_some_and(name),
        _ => false,
    };
    joined_before || joined_after
}

/// A page's Markdown as drawn at one width, and the links it drew (text and address).
type Rendered = Rc<(Vec<Line<'static>>, Vec<text::DrawnLink>)>;

#[derive(Default)]
pub(crate) struct Documents {
    views: HashMap<String, DocumentView>,
    /// Definitive missing-document observations, even when a previously read page stays cached.
    missing: HashSet<String>,
    lists: HashMap<String, DocumentList>,
    references: HashMap<String, Result<DocumentReferences, String>>,
    /// Reads sent and not answered yet, by `get:`, `list:` or `refs:` and the subject.
    reading: HashSet<String>,
    /// When a read that failed may be sent again.
    retry_at: HashMap<String, Instant>,
    /// Lists whose next page was asked for.
    more: HashSet<String>,
    /// The documents followed in each split, by `GLASS/GROUP`.
    trails: HashMap<String, DocumentTrail>,
    /// The link (on a page) or row (in the list) n and p have chosen, by pane key.
    chosen: HashMap<String, usize>,
    /// Each page's Markdown as drawn, by reference and width: a frame copies it rather than
    /// rendering the whole document again, so typing elsewhere stays as quick with a page shown.
    rendered: RefCell<HashMap<(String, usize), Rendered>>,
}

fn list_key(prefix: Option<&str>) -> String {
    prefix.unwrap_or_default().to_owned()
}

impl Documents {
    pub(super) fn known_missing(&self, reference: &str) -> bool {
        self.missing.contains(reference)
    }

    /// The reads the shown panes need that are not sent yet. `background` panes are read without
    /// being on screen (the list behind Ctrl+K's rows). `subjects` are the missions and steps on
    /// screen whose documents are listed with them.
    pub(crate) fn wanted(&mut self, panes: &[super::pane::Pane], background: &[super::pane::Pane], subjects: &[String]) -> Vec<Want> {
        use super::pane::Pane;
        let now = Instant::now();
        let mut wants = Vec::new();
        let due = |retry_at: &HashMap<String, Instant>, key: &str| retry_at.get(key).is_some_and(|at| *at <= now);
        for pane in panes.iter().chain(background) {
            match pane {
                Pane::Document(reference) => {
                    let key = format!("get:{reference}");
                    let again = due(&self.retry_at, &key);
                    if (!self.views.contains_key(reference) || again) && !self.reading.contains(&key) {
                        self.retry_at.remove(&key);
                        self.views.entry(reference.clone()).or_insert_with(|| DocumentView::loading(reference));
                        self.reading.insert(key);
                        wants.push(Want::Get(reference.clone()));
                    }
                }
                Pane::Documents(prefix) => {
                    let name = list_key(prefix.as_deref());
                    let key = format!("list:{name}");
                    if self.reading.contains(&key) {
                        continue;
                    }
                    let again = due(&self.retry_at, &key);
                    match self.lists.get(&name) {
                        None => {
                            self.lists.insert(name.clone(), DocumentList::new(prefix.as_deref()));
                        }
                        Some(_) if again => {
                            self.lists.insert(name.clone(), DocumentList::new(prefix.as_deref()));
                        }
                        Some(list) if self.more.contains(&name) && list.next_cursor.is_some() => {
                            self.more.remove(&name);
                            self.reading.insert(key);
                            wants.push(Want::List { prefix: prefix.clone(), cursor: list.next_cursor.clone() });
                            continue;
                        }
                        Some(_) => continue,
                    }
                    self.retry_at.remove(&key);
                    self.reading.insert(key);
                    wants.push(Want::List { prefix: prefix.clone(), cursor: None });
                }
                _ => {}
            }
        }
        for subject in subjects {
            let key = format!("refs:{subject}");
            if !self.references.contains_key(subject) && !self.reading.contains(&key) {
                self.reading.insert(key);
                wants.push(Want::References(subject.clone()));
            }
        }
        wants
    }

    /// What `document.get` answered for `reference`.
    pub(crate) fn got(&mut self, reference: &str, outcome: Result<DocumentContent, Unavailable>) {
        let key = format!("get:{reference}");
        self.reading.remove(&key);
        if matches!(&outcome, Err(Unavailable::NotFound)) {
            self.missing.insert(reference.to_owned());
        } else {
            self.missing.remove(reference);
        }
        let view = match outcome {
            Ok(content) => DocumentView::from_content(reference, &content),
            Err(why) => {
                if matches!(why, Unavailable::NotSynced | Unavailable::RemoteUnavailable) {
                    self.retry_at.insert(key, Instant::now() + RETRY);
                }
                // A page already read stays up while a later read fails.
                if let Some(DocumentView::Ready(_)) = self.views.get(reference) {
                    return;
                }
                DocumentView::unavailable(reference, why)
            }
        };
        self.rendered.borrow_mut().retain(|(drawn, _), _| drawn != reference);
        self.views.insert(reference.to_owned(), view);
    }

    /// What `documents.list` answered for the list under `prefix`.
    pub(crate) fn listed(&mut self, prefix: Option<&str>, outcome: Result<DocumentsPage, Unavailable>) {
        let name = list_key(prefix);
        let key = format!("list:{name}");
        self.reading.remove(&key);
        let list = self.lists.entry(name).or_insert_with(|| DocumentList::new(prefix));
        match outcome {
            Ok(page) => list.push_page(&page),
            Err(why) => {
                list.fail(why);
                self.retry_at.insert(key, Instant::now() + RETRY);
            }
        }
    }

    /// What `document-references.get` answered for a mission or step.
    pub(crate) fn referenced(&mut self, subject: &str, outcome: Result<DocumentReferences, String>) {
        self.reading.remove(&format!("refs:{subject}"));
        self.references.insert(subject.to_owned(), outcome);
    }

    /// Read a document or the list again.
    pub(crate) fn retry(&mut self, pane: &super::pane::Pane) {
        use super::pane::Pane;
        match pane {
            Pane::Document(reference) => {
                self.retry_at.insert(format!("get:{reference}"), Instant::now());
            }
            Pane::Documents(prefix) => {
                self.retry_at.insert(format!("list:{}", list_key(prefix.as_deref())), Instant::now());
            }
            _ => {}
        }
    }

    /// Every document a list has shown, once by name, for Ctrl+K.
    pub(crate) fn listed_rows(&self) -> Vec<&fractal_ui_model::documents::DocumentRow> {
        let mut rows: Vec<&fractal_ui_model::documents::DocumentRow> = Vec::new();
        for list in self.lists.values() {
            for row in &list.rows {
                if !rows.iter().any(|seen| seen.name == row.name) {
                    rows.push(row);
                }
            }
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    pub(crate) fn more(&mut self, prefix: Option<&str>) {
        self.more.insert(list_key(prefix));
    }

    /// The trail of a split showing `reference`: started again when the split shows a document the
    /// trail does not end at (it was opened some other way).
    fn trail(&mut self, split: &str, reference: &str) -> &mut DocumentTrail {
        let trail = self.trails.entry(split.to_owned()).or_default();
        if trail.current() != Some(reference) {
            *trail = DocumentTrail::default();
            trail.open(reference);
        }
        trail
    }

    /// Follow a link from the page `from` in a split: the page to show next. Following one back to a
    /// document already on the trail returns to it, so a cycle of links never grows the trail.
    pub(crate) fn follow(&mut self, split: &str, from: &str, to: &str) -> String {
        let trail = self.trail(split, from);
        trail.open(to);
        trail.current().unwrap_or(to).to_owned()
    }

    /// Back one document from the page `from`: the page to show, or `None` at the first.
    pub(crate) fn back(&mut self, split: &str, from: &str) -> Option<String> {
        let trail = self.trail(split, from);
        if trail.references.len() < 2 {
            return None;
        }
        trail.back().map(str::to_owned)
    }

    /// The page back would show from `from` in a split.
    pub(crate) fn previous(&self, split: &str, from: &str) -> Option<&str> {
        let trail = self.trails.get(split)?;
        (trail.current() == Some(from) && trail.references.len() > 1)
            .then(|| trail.references[trail.references.len() - 2].as_str())
    }

    /// The `doc/…` references n and p move through on a pane: a page's links, or the list's rows.
    fn choices(&self, pane: &super::pane::Pane) -> Vec<String> {
        use super::pane::Pane;
        match pane {
            Pane::Document(reference) => match self.views.get(reference) {
                Some(DocumentView::Ready(document)) => document.links.iter().map(|link| link.reference.clone()).collect(),
                _ => Vec::new(),
            },
            Pane::Documents(prefix) => self
                .lists
                .get(&list_key(prefix.as_deref()))
                .map(|list| list.rows.iter().map(|row| list_target(row).to_owned()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// n (`1`) or p (`-1`): choose the next or previous link or row.
    pub(crate) fn choose(&mut self, pane: &super::pane::Pane, step: isize) {
        let count = self.choices(pane).len();
        if count == 0 {
            return;
        }
        let key = pane.key();
        let next = match self.chosen.get(&key) {
            None if step > 0 => 0,
            None => count - 1,
            Some(at) => ((*at as isize + step).rem_euclid(count as isize)) as usize,
        };
        self.chosen.insert(key, next);
    }

    /// The link or row chosen on a pane, which Enter opens.
    pub(crate) fn chosen(&self, pane: &super::pane::Pane) -> Option<String> {
        let at = *self.chosen.get(&pane.key())?;
        self.choices(pane).into_iter().nth(at)
    }

    /// One document's page.
    pub(crate) fn page(&self, reference: &str, back: Option<&str>, width: usize, now_ms: u64) -> Doc {
        let mut doc = Doc::new();
        let view = self.views.get(reference).cloned().unwrap_or_else(|| DocumentView::loading(reference));
        if let Some(back) = back {
            doc.buttons(&[("b", &format!("back to {}", short_reference(back)), Hit::Document(Tap::Back), theme::ACCENT)]);
            doc.blank();
        }
        match view {
            DocumentView::Loading { reference } => {
                doc.line(Line::from(Span::styled(format!("Loading {reference}…"), theme::dim())));
            }
            DocumentView::Unavailable { why, explanation, .. } => {
                let mut inner = Doc::new();
                inner.wrap(&[text::run(explanation, theme::soft())], width.saturating_sub(4));
                inner.blank();
                inner.buttons(&[("r", "try again", Hit::Document(Tap::Retry), theme::ACCENT)]);
                doc.card(why_title(why), theme::OVERLAY1, false, inner, width);
            }
            DocumentView::Ready(document) => {
                let mut title = vec![Span::styled(document.name.clone(), theme::strong(theme::TEXT))];
                match document.latest {
                    Some(true) => title.push(Span::styled("  latest", theme::fg(theme::GREEN))),
                    Some(false) => title.push(Span::styled("  an older version", theme::fg(theme::YELLOW))),
                    None => {}
                }
                doc.line(Line::from(title));
                doc.wrap(&[text::run(version_line(&document.short_version, document.changed_at_unix_ms, document.owner.as_deref(), now_ms), theme::dim())], width);
                doc.section("", None, width);
                if document.text.trim().is_empty() {
                    doc.line(Line::from(Span::styled("This document is empty.", theme::dim())));
                } else {
                    let drawn = self
                        .rendered
                        .borrow_mut()
                        .entry((reference.to_owned(), width))
                        .or_insert_with(|| {
                            let references = document.links.iter().map(|link| link.reference.clone()).collect::<Vec<_>>();
                            Rc::new(text::markdown_with_links(&document.text, width, theme::text(), &references))
                        })
                        .clone();
                    // Each link's cells, and each written-out doc/ reference's on every row it wraps
                    // to, open its own whole address, wherever the page is drawn from: two links
                    // with the same text stay apart, and a wrapped hash is never cut short.
                    let first = doc.lines.len();
                    doc.lines(drawn.0.iter().cloned());
                    for link in &drawn.1 {
                        doc.targets.push(Target {
                            line: first + link.line,
                            column: u16::try_from(link.column).unwrap_or(u16::MAX),
                            width: u16::try_from(link.width).unwrap_or(u16::MAX),
                            hit: Hit::Link(link.url.clone()),
                        });
                    }
                }
                if !document.links.is_empty() {
                    doc.blank();
                    doc.section("Links", Some(document.links.len()), width);
                    let chosen = self.chosen.get(&super::pane::Pane::Document(reference.to_owned()).key()).copied();
                    for (index, link) in document.links.iter().enumerate() {
                        link_row(&mut doc, &link.reference, None, chosen == Some(index), width);
                    }
                }
            }
        }
        doc
    }

    /// The documents list under `prefix`.
    pub(crate) fn list(&self, prefix: Option<&str>, width: usize, now_ms: u64) -> Doc {
        let mut doc = Doc::new();
        let name = list_key(prefix);
        let empty = DocumentList::new(prefix);
        let list = self.lists.get(&name).unwrap_or(&empty);
        let mut title = vec![Span::styled("Documents", theme::strong(theme::TEXT))];
        if let Some(prefix) = prefix {
            title.push(Span::styled(format!("  under {prefix}"), theme::dim()));
        }
        if !list.rows.is_empty() {
            title.push(Span::styled(format!("  {}{}", list.rows.len(), if list.next_cursor.is_some() { "+" } else { "" }), theme::dim()));
        }
        doc.line(Line::from(title));
        doc.blank();
        if let Some(placeholder) = list.placeholder() {
            doc.wrap(&[text::run(placeholder, theme::soft())], width);
            if list.failed.is_some() {
                doc.blank();
                doc.buttons(&[("r", "try again", Hit::Document(Tap::Retry), theme::ACCENT)]);
            }
            return doc;
        }
        let chosen = self.chosen.get(&super::pane::Pane::Documents(prefix.map(str::to_owned)).key()).copied();
        for (index, row) in list.rows.iter().enumerate() {
            let detail = format!("{} · {}", row.short_version, changed(Some(row.changed_at_unix_ms), now_ms));
            link_row(&mut doc, &row.name, Some((list_target(row), &detail)), chosen == Some(index), width);
        }
        if let Some(why) = list.failed {
            doc.blank();
            doc.wrap(&[text::run(why.explain(prefix.unwrap_or("doc/")), theme::soft())], width);
        }
        if list.next_cursor.is_some() {
            doc.blank();
            doc.buttons(&[("m", "more", Hit::Document(Tap::More), theme::ACCENT)]);
        }
        doc
    }

    /// The documents a mission and its steps name, under the mission's card; nothing until st has
    /// said, and nothing when they name none.
    pub(crate) fn mission_section(&self, doc: &mut Doc, subjects: &[(String, Option<String>)], width: usize) {
        let mut rows: Vec<(String, Option<String>)> = Vec::new();
        let mut failed = None;
        for (subject, step) in subjects {
            match self.references.get(subject) {
                Some(Ok(references)) => {
                    for reference in &references.references {
                        if !rows.iter().any(|(seen, _)| seen == &reference.reference) {
                            let step = reference.step.clone().or_else(|| step.clone());
                            rows.push((reference.reference.clone(), step));
                        }
                    }
                }
                Some(Err(error)) => failed = Some(error.clone()),
                None => {}
            }
        }
        if rows.is_empty() && failed.is_none() {
            return;
        }
        doc.blank();
        doc.section("Documents", Some(rows.len()), width);
        for (reference, step) in &rows {
            let label = short_reference(reference);
            let detail = step.as_ref().map(|step| format!("step {step}")).unwrap_or_default();
            link_row(doc, &label, Some((reference, &detail)), false, width);
        }
        if let Some(error) = failed {
            doc.wrap(&[text::run(format!("Some documents could not be listed: {error}"), theme::soft())], width);
        }
    }
}

impl Documents {
    /// Without a connection a page never read says the host did not answer, and is read once
    /// there is one again.
    pub(crate) fn offline(&mut self, panes: &[super::pane::Pane]) {
        for pane in panes {
            if let super::pane::Pane::Document(reference) = pane
                && !self.views.contains_key(reference)
            {
                self.views.insert(reference.clone(), DocumentView::unavailable(reference, Unavailable::RemoteUnavailable));
                self.retry_at.insert(format!("get:{reference}"), Instant::now());
            }
        }
    }
}

impl super::Ui {
    /// Open a `doc/…` reference. From a document page it opens in its place, on that split's
    /// trail; from anywhere else (a message, a mission, the list, Ctrl+K) in a new tab.
    pub(crate) fn open_document(&mut self, reference: &str) {
        use super::pane::Pane;
        let Some(reference) = openable(reference) else {
            self.flash(format!("{} is not a document name: doc/NAME or doc/NAME@HASH", text::truncate(reference, 80)));
            return;
        };
        if let (Some(Pane::Document(from)), Some(split)) = (self.focused_pane(), self.focused_split()) {
            let next = self.documents.follow(&split, &from, reference);
            self.swap_focused_pane(Pane::Document(next));
        } else {
            self.open_in_glass(Pane::Document(reference.to_owned()), super::glass::Open::Tab);
        }
    }

    pub(crate) fn document_tap(&mut self, tap: Tap) {
        use super::pane::Pane;
        let pane = self.focused_pane();
        match (tap, pane) {
            (Tap::Back, Some(Pane::Document(from))) => {
                if let Some(split) = self.focused_split()
                    && let Some(previous) = self.documents.back(&split, &from)
                {
                    self.swap_focused_pane(Pane::Document(previous));
                }
            }
            (Tap::Retry, Some(pane)) => self.documents.retry(&pane),
            (Tap::More, Some(Pane::Documents(prefix))) => self.documents.more(prefix.as_deref()),
            _ => {}
        }
    }

    /// A key on a focused document page or documents list. Neither has a box to type in, so its
    /// plain letters act: b or Backspace back, n and p choose a link or row, Enter opens it, r reads
    /// again, m reads the list's next page.
    pub(crate) fn document_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use super::pane::Pane;
        use crossterm::event::KeyCode;
        let Some(pane @ (Pane::Document(_) | Pane::Documents(_))) = self.focused_pane() else {
            return false;
        };
        if !key.modifiers.is_empty() {
            return false;
        }
        match (key.code, &pane) {
            (KeyCode::Backspace | KeyCode::Char('b'), Pane::Document(_)) => self.document_tap(Tap::Back),
            (KeyCode::Char('n'), _) => self.documents.choose(&pane, 1),
            (KeyCode::Char('p'), _) => self.documents.choose(&pane, -1),
            (KeyCode::Enter, _) => match self.documents.chosen(&pane) {
                Some(reference) => self.open_document(&reference),
                None => return false,
            },
            (KeyCode::Char('r'), _) => self.documents.retry(&pane),
            (KeyCode::Char('m'), Pane::Documents(_)) => self.document_tap(Tap::More),
            _ => return false,
        }
        true
    }
}

/// What a row of the list opens: the document by name when the row is its latest version, as a
/// link or Ctrl+K opens it, so following links back to it returns along the trail.
fn list_target(row: &fractal_ui_model::documents::DocumentRow) -> &str {
    if row.latest { &row.name } else { &row.reference }
}

/// `doc/NAME@HASH` with its hash cut to twelve characters, for a label.
fn short_reference(reference: &str) -> String {
    match parse_reference(reference) {
        Some((name, Some(version))) => format!("{name}@{}", &version[..12]),
        _ => reference.to_owned(),
    }
}

fn why_title(why: Unavailable) -> &'static str {
    match why {
        Unavailable::NotFound => "not found",
        Unavailable::NotSynced => "not synced yet",
        Unavailable::RemoteUnavailable => "remote unavailable",
        Unavailable::NotText => "not text",
    }
}

/// A clickable row: `label` (or the reference) and its detail; the chosen row is marked.
fn link_row(doc: &mut Doc, label: &str, target: Option<(&str, &str)>, chosen: bool, width: usize) {
    let (reference, detail) = target.unwrap_or((label, ""));
    let marker = if chosen { "▸ " } else { "→ " };
    let mut spans = vec![
        Span::styled(marker, theme::fg(theme::LAVENDER)),
        Span::styled(text::truncate(&text::sanitize(label), width.saturating_sub(4)), Style::default().fg(theme::BLUE).add_modifier(Modifier::UNDERLINED)),
    ];
    if !detail.is_empty() {
        spans.push(Span::styled(format!("  {detail}"), theme::dim()));
    }
    let line = doc.lines.len();
    doc.targets.push(Target { line, column: 0, width: width as u16, hit: Hit::Link(reference.to_owned()) });
    doc.line(Line::from(spans));
}

/// "version 0123456789ab · changed 3h ago (2026-10-12 14:03) · by agent/…".
pub(crate) fn version_line(short_version: &str, changed_at_unix_ms: Option<u64>, owner: Option<&str>, now_ms: u64) -> String {
    let mut line = format!("version {short_version} · {}", changed(changed_at_unix_ms, now_ms));
    if let Some(owner) = owner {
        line.push_str(&format!(" · by {owner}"));
    }
    line
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn changed(changed_at_unix_ms: Option<u64>, now_ms: u64) -> String {
    let Some(at) = changed_at_unix_ms else {
        return "this host does not say when it changed".into();
    };
    let seconds = now_ms.saturating_sub(at) / 1000;
    let ago = if seconds < 60 {
        "just now".to_owned()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    };
    let when = chrono::DateTime::from_timestamp_millis(at as i64)
        .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    format!("changed {ago} ({when})")
}

#[cfg(test)]
mod tests;
