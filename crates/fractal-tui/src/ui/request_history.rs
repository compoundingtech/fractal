//! A thread's Related requests: each request as it was asked, keyed by its exact source and
//! episode, with that episode's own status and answers.
//!
//! Everything shown comes from the history page as served. A request whose retained payload is
//! unavailable says why; it never borrows today's text of its source. A relation (the request a
//! request followed, and why) shows only when both of its requests are on the page, in `items` or
//! `boundary_items`; there are no placeholders for the rest. Paging controls are the host's.
//!
//! No link claims to open a past episode: a source opens as it is now and says so, and a retained
//! reference opens only when it names a pinned document version.

use std::collections::BTreeSet;

use fractal_client::{
    EpisodeState, HistoricalPayload, HistoricalReference, HistorySourceSupport, PayloadUnavailable,
    ReferenceSemantics, RequestHistoryItem, RequestHistoryPage, SourceEpisode,
};
use ratatui::style::Style;
use fractal_conversation_ui::text::{Line, Span};

use super::doc::{Doc, Hit, Target};
use super::text::{self, Run, run};
use super::{documents, theme};

const TITLE: &str = "Related requests";
const NONE: &str = "No related requests.";
const PARTIAL: &str = "Showing part of the history.";
const UNSUPPORTED: &str = "Related requests are not kept for this source.";
const CURRENT_SOURCE: &str = "current source";
const CURRENT_REFERENCE: &str = "current reference";

/// One run of a single-row line: its text, style, and what clicking it does.
type Part = (String, Style, Option<Hit>);

pub(super) fn render(doc: &mut Doc, page: &fractal_client::RequestHistoryPage, width: usize) {
    doc.line(Line::from(Span::styled(TITLE, theme::label())));
    if matches!(page.source_support, Some(HistorySourceSupport::UnsupportedSource)) {
        wrapped(doc, "", vec![run(UNSUPPORTED, theme::dim())], width);
        return;
    }
    if page.items.is_empty() && page.complete {
        doc.line(Line::from(Span::styled(NONE, theme::dim())));
    }
    for (source, items) in groups(page) {
        wrapped(doc, "", vec![run(text::sanitize(source), theme::bold())], width);
        row(doc, vec![plain("  open ", theme::dim()), link(CURRENT_SOURCE, source)]);
        for item in items {
            entry(doc, page, item, width);
        }
    }
    if !page.complete {
        doc.line(Line::from(Span::styled(PARTIAL, theme::dim())));
    }
}

/// The page's items by source: the served `source_episodes` order first, then any item no group
/// names, under its own source in page order. Each item appears once.
fn groups(page: &RequestHistoryPage) -> Vec<(&str, Vec<&RequestHistoryItem>)> {
    fn place<'a>(groups: &mut Vec<(&'a str, Vec<&'a RequestHistoryItem>)>, item: &'a RequestHistoryItem) {
        let source = item.key.source_id.as_str();
        match groups.iter_mut().find(|(id, _)| *id == source) {
            Some((_, items)) => items.push(item),
            None => groups.push((source, vec![item])),
        }
    }
    let mut seen = BTreeSet::new();
    let mut groups = Vec::new();
    let served = page.source_episodes.iter().flat_map(|group| {
        group.episodes.iter().filter(move |key| key.source_id == group.source_id)
    });
    for key in served {
        if let Some(item) = page.items.iter().find(|item| &item.key == key)
            && seen.insert((key.source_id.as_str(), key.episode.as_str()))
        {
            place(&mut groups, item);
        }
    }
    for item in &page.items {
        if seen.insert((item.key.source_id.as_str(), item.key.episode.as_str())) {
            place(&mut groups, item);
        }
    }
    groups
}

fn entry(doc: &mut Doc, page: &RequestHistoryPage, item: &RequestHistoryItem, width: usize) {
    let mut head = vec![run("episode ", theme::dim()), run(text::sanitize(&item.key.episode), theme::soft())];
    match &item.state {
        EpisodeState::Available { status, .. } => {
            head.push(run(" · ", theme::dim()));
            head.push(run(text::sanitize(status), theme::bold()));
        }
        EpisodeState::Unavailable => head.push(run(" · status unavailable", theme::dim())),
    }
    if !item.complete {
        head.push(run(" · incomplete", theme::dim()));
    }
    wrapped(doc, "  ", head, width);

    match &item.payload {
        HistoricalPayload::Available { title, text: body, references, .. } => {
            if let Some(title) = title.as_deref().filter(|title| !title.trim().is_empty()) {
                wrapped(doc, "    ", vec![run(text::sanitize(title), theme::bold())], width);
            }
            for line in body.lines() {
                wrapped(doc, "    ", vec![run(text::sanitize(line), theme::text())], width);
            }
            for reference in references {
                row(doc, reference_row(reference));
            }
        }
        HistoricalPayload::Unavailable { reason } => wrapped(
            doc,
            "    ",
            vec![run(format!("Request as asked is unavailable: {}.", unavailable(reason)), theme::dim())],
            width,
        ),
    }

    if let EpisodeState::Available { answers, .. } = &item.state {
        for answer in answers {
            let answer = match answer_summary(answer) {
                Some(summary) => run(text::sanitize(summary), theme::text()),
                None => run("answer recorded", theme::dim()),
            };
            doc.lines(text::wrap(
                &[answer],
                width,
                &[run("    ↳ ", theme::dim())],
                &[run("      ", theme::dim())],
                None,
            ));
        }
    }

    for relation in &page.relations {
        if !supplied(page, &relation.predecessor) || !supplied(page, &relation.request) {
            continue;
        }
        let (word, other) = if relation.request == item.key {
            ("after ", &relation.predecessor)
        } else if relation.predecessor == item.key && !shown(page, &relation.request) {
            // A follower on another page: its own entry is not here to carry the reason.
            ("before ", &relation.request)
        } else {
            continue;
        };
        wrapped(
            doc,
            "    ",
            vec![
                run(word, theme::dim()),
                run(text::sanitize(&other.source_id), theme::soft()),
                run(" · ", theme::dim()),
                run(text::sanitize(&other.episode), theme::soft()),
                run(": ", theme::dim()),
                run(text::sanitize(&relation.reason), theme::text()),
            ],
            width,
        );
        if other.source_id != item.key.source_id {
            row(doc, vec![plain("      open its ", theme::dim()), link(CURRENT_SOURCE, &other.source_id)]);
        }
    }
}

/// A live reference follows its target as it is now, so it opens it and says so. A retained one
/// opens only as a pinned document version (`doc/…@<hash>`); otherwise opening its target today
/// would show a different revision, so it stays text.
fn reference_row(reference: &HistoricalReference) -> Vec<Part> {
    let target = reference.target_id.as_str();
    match (&reference.semantics, reference.revision.as_deref()) {
        (ReferenceSemantics::LiveReference, revision) => {
            let mut parts = vec![plain("    → ", theme::dim()), plain(target, theme::soft())];
            if let Some(revision) = revision {
                parts.push(plain(&format!("@{revision}"), theme::dim()));
            }
            parts.push(plain(" · open ", theme::dim()));
            parts.push(link(CURRENT_REFERENCE, target));
            parts
        }
        (ReferenceSemantics::RetainedReference, Some(revision)) => {
            let pinned = format!("{target}@{revision}");
            match documents::openable(&pinned) {
                Some(pinned) => vec![
                    plain("    → ", theme::dim()),
                    (text::sanitize(pinned), theme::fg(theme::ACCENT), Some(Hit::Link(pinned.to_owned()))),
                    plain(" · as retained", theme::dim()),
                ],
                None => vec![plain("    → ", theme::dim()), plain(&format!("{pinned} · as retained"), theme::dim())],
            }
        }
        (ReferenceSemantics::RetainedReference, None) => {
            vec![plain("    → ", theme::dim()), plain(&format!("{target} · as retained"), theme::dim())]
        }
    }
}

/// An answer's own words: a person answer view's `summary`, or a retained claim's
/// `fields.summary`. Nothing else of the answer is shown.
fn answer_summary(answer: &serde_json::Value) -> Option<&str> {
    answer
        .get("summary")
        .or_else(|| answer.pointer("/fields/summary"))
        .and_then(serde_json::Value::as_str)
        .filter(|summary| !summary.trim().is_empty())
}

fn unavailable(reason: &PayloadUnavailable) -> &'static str {
    match reason {
        PayloadUnavailable::RetainedDefinitionUnavailable => "its retained definition is unavailable",
        PayloadUnavailable::RetainedStepUnavailable => "its retained step is unavailable",
        PayloadUnavailable::OpeningUnavailable => "its opening is unavailable",
        PayloadUnavailable::HistoricalInterpolationUnavailable => "its values at the time are unavailable",
        PayloadUnavailable::BoundaryUnavailable => "it is outside the retained history",
        PayloadUnavailable::PayloadUnavailable => "it was not retained",
        PayloadUnavailable::ReadBoundExceeded => "it is too large to read here",
    }
}

fn shown(page: &RequestHistoryPage, key: &SourceEpisode) -> bool {
    page.items.iter().any(|item| &item.key == key)
}

fn supplied(page: &RequestHistoryPage, key: &SourceEpisode) -> bool {
    shown(page, key) || page.boundary_items.iter().any(|item| &item.key == key)
}

/// Word-wrapped text under a hanging indent; none of it is a control.
fn wrapped(doc: &mut Doc, indent: &str, runs: Vec<Run>, width: usize) {
    let indent = [run(indent, theme::dim())];
    doc.lines(text::wrap(&runs, width, &indent, &indent, None));
}

fn plain(value: &str, style: Style) -> Part {
    (text::sanitize(value), style, None)
}

/// A link that opens `id` as it is now; the hit keeps the exact identity.
fn link(label: &str, id: &str) -> Part {
    (label.to_owned(), theme::fg(theme::ACCENT), Some(Hit::Open(id.to_owned())))
}

/// One row whose parts may be links; used only for short controls, so it is not wrapped.
fn row(doc: &mut Doc, parts: Vec<Part>) {
    let line = doc.lines.len();
    let mut column = 0usize;
    let mut spans = Vec::with_capacity(parts.len());
    for (value, style, hit) in parts {
        let width = text::width(&value);
        if let Some(hit) = hit {
            doc.targets.push(Target {
                line,
                column: u16::try_from(column).unwrap_or(u16::MAX),
                width: u16::try_from(width).unwrap_or(u16::MAX),
                hit,
            });
        }
        column += width;
        spans.push(Span::styled(value, style));
    }
    doc.line(Line::from(spans));
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_client::{RequestHistoryRelation, RequestProvenance, SourceEpisodeGroup};
    use serde_json::json;

    const WIDTH: usize = 80;

    fn key(source: &str, episode: &str) -> SourceEpisode {
        SourceEpisode { source_id: source.into(), episode: episode.into() }
    }

    fn item(key: SourceEpisode, title: &str, body: &str, status: &str) -> RequestHistoryItem {
        RequestHistoryItem {
            key,
            payload: HistoricalPayload::Available {
                title: Some(title.into()),
                text: body.into(),
                request: None,
                references: Vec::new(),
                provenance: RequestProvenance::RuntimeAsk { asking_claim_id: "claim/asking".into() },
            },
            state: EpisodeState::Available { status: status.into(), answers: Vec::new() },
            complete: true,
        }
    }

    fn page(items: Vec<RequestHistoryItem>) -> RequestHistoryPage {
        RequestHistoryPage {
            items,
            relations: Vec::new(),
            source_episodes: Vec::new(),
            boundary_items: Vec::new(),
            source_support: Some(HistorySourceSupport::PersonStep),
            next_cursor: None,
            complete: true,
        }
    }

    fn draw_at(page: &RequestHistoryPage, width: usize) -> (Doc, Vec<String>) {
        let mut doc = Doc::new();
        render(&mut doc, page, width);
        let lines = doc.lines.iter().map(Line::to_string).collect();
        (doc, lines)
    }

    fn draw(page: &RequestHistoryPage) -> (Doc, Vec<String>) {
        draw_at(page, WIDTH)
    }

    /// The text under each target, with its hit.
    fn links(doc: &Doc, lines: &[String]) -> Vec<(String, Hit)> {
        doc.targets
            .iter()
            .map(|target| {
                let under = lines[target.line]
                    .chars()
                    .skip(target.column as usize)
                    .take(target.width as usize)
                    .collect();
                (under, target.hit.clone())
            })
            .collect()
    }

    #[test]
    fn an_entry_names_its_exact_source_and_episode_with_its_own_status_and_answers() {
        let mut first = item(key("work/acme/ask", "claim/e1"), "Ship it?", "Approve the release.\nIt is tagged.", "done");
        first.state = EpisodeState::Available {
            status: "done".into(),
            answers: vec![json!({"summary": "Yes, ship", "respondent": "person/avery"})],
        };
        let (doc, lines) = draw(&page(vec![first]));
        assert_eq!(
            lines,
            [
                "Related requests",
                "work/acme/ask",
                "  open current source",
                "  episode claim/e1 · done",
                "    Ship it?",
                "    Approve the release.",
                "    It is tagged.",
                "    ↳ Yes, ship",
            ]
        );
        // The only control opens the source as it is now, by its exact id, and says so.
        assert_eq!(
            links(&doc, &lines),
            [(CURRENT_SOURCE.to_owned(), Hit::Open("work/acme/ask".into()))]
        );
    }

    #[test]
    fn long_text_wraps_whole_at_the_width() {
        let long = "word ".repeat(40);
        let mut entry = item(key("work/acme/ask", "claim/e1"), "Title", long.trim(), "done");
        entry.state = EpisodeState::Available {
            status: "done".into(),
            answers: vec![json!({"summary": long.trim()})],
        };
        let (_, lines) = draw_at(&page(vec![entry]), 30);
        assert!(lines.iter().all(|line| text::width(line) <= 30), "{lines:?}");
        let words = |prefix: &str| {
            lines.iter().filter(|line| line.starts_with(prefix)).map(|line| line.matches("word").count()).sum::<usize>()
        };
        assert_eq!(words("    word"), 40, "the whole request text is shown: {lines:?}");
        assert_eq!(words("    ↳ ") + words("      word"), 40, "the whole answer is shown: {lines:?}");
    }

    #[test]
    fn episodes_of_one_source_group_under_it_in_served_order() {
        let mut history = page(vec![
            item(key("work/acme/ask", "claim/e1"), "First", "one", "done"),
            item(key("work/other/ask", "claim/x"), "Other", "x", "ready"),
            item(key("work/acme/ask", "claim/e2"), "Second", "two", "ready"),
        ]);
        history.source_episodes = vec![SourceEpisodeGroup {
            source_id: "work/acme/ask".into(),
            episodes: vec![key("work/acme/ask", "claim/e2"), key("work/acme/ask", "claim/e1")],
        }];
        let (doc, lines) = draw(&history);
        assert_eq!(lines.iter().filter(|line| *line == "work/acme/ask").count(), 1);
        let at = |needle: &str| lines.iter().position(|line| line == needle).unwrap();
        assert!(at("work/acme/ask") < at("  episode claim/e2 · ready"));
        assert!(at("  episode claim/e2 · ready") < at("  episode claim/e1 · done"));
        assert!(at("  episode claim/e1 · done") < at("work/other/ask"));
        let sources: Vec<_> = links(&doc, &lines).into_iter().map(|(_, hit)| hit).collect();
        assert_eq!(sources, [Hit::Open("work/acme/ask".into()), Hit::Open("work/other/ask".into())]);
    }

    #[test]
    fn a_reason_shows_only_when_both_requests_are_supplied() {
        let earlier = key("work/first/ask", "claim/e0");
        let later = key("work/acme/ask", "claim/e1");
        let missing = key("work/gone/ask", "claim/z");
        let mut history = page(vec![item(later.clone(), "Again?", "retry", "ready")]);
        history.boundary_items = vec![item(earlier.clone(), "BOUNDARY TITLE", "BOUNDARY TEXT", "done")];
        history.relations = vec![
            RequestHistoryRelation { predecessor: earlier, request: later.clone(), reason: "asked again".into() },
            RequestHistoryRelation { predecessor: missing, request: later, reason: "HIDDEN REASON".into() },
        ];
        let (doc, lines) = draw(&history);
        let after = lines
            .iter()
            .position(|line| line == "    after work/first/ask · claim/e0: asked again")
            .unwrap_or_else(|| panic!("{lines:?}"));
        let text = lines.join("\n");
        assert!(!text.contains("HIDDEN REASON") && !text.contains("work/gone"), "{text}");
        assert!(!text.contains("BOUNDARY"), "a boundary item is an endpoint, not an entry: {text}");
        // The other source opens as it is now, by its exact id, labelled as current.
        assert_eq!(lines[after + 1], "      open its current source");
        let predecessor = doc.targets.iter().find(|target| target.line == after + 1).unwrap();
        assert_eq!(
            (predecessor.column, predecessor.width, &predecessor.hit),
            (15, 14, &Hit::Open("work/first/ask".into()))
        );
    }

    #[test]
    fn an_unavailable_payload_says_why_and_borrows_no_text() {
        let history = page(vec![RequestHistoryItem {
            key: key("work/acme/ask", "claim/old"),
            payload: HistoricalPayload::Unavailable { reason: PayloadUnavailable::OpeningUnavailable },
            state: EpisodeState::Unavailable,
            complete: false,
        }]);
        let (_, lines) = draw(&history);
        assert_eq!(
            lines,
            [
                "Related requests",
                "work/acme/ask",
                "  open current source",
                "  episode claim/old · status unavailable · incomplete",
                "    Request as asked is unavailable: its opening is unavailable.",
            ]
        );
    }

    #[test]
    fn private_fields_stay_hidden_and_references_open_only_honestly() {
        let hash = "a".repeat(64);
        let mut private = item(key("work/acme/ask", "claim/e1"), "Title", "Body", "done");
        private.payload = HistoricalPayload::Available {
            title: None,
            text: "Body".into(),
            request: Some(json!({"secret": "REQUEST SECRET"})),
            references: vec![
                HistoricalReference {
                    target_id: "doc/acme/plan".into(),
                    revision: Some("abc".into()),
                    semantics: ReferenceSemantics::RetainedReference,
                },
                HistoricalReference {
                    target_id: "doc/acme/pinned".into(),
                    revision: Some(hash.clone()),
                    semantics: ReferenceSemantics::RetainedReference,
                },
                HistoricalReference {
                    target_id: "mission/acme/ship".into(),
                    revision: None,
                    semantics: ReferenceSemantics::LiveReference,
                },
            ],
            provenance: RequestProvenance::AuthoredEpisode {
                mission_revision: "REVISION SECRET".into(),
                opening_claim_id: "claim/OPENING SECRET".into(),
            },
        };
        private.state = EpisodeState::Available {
            status: "done".into(),
            answers: vec![
                json!({"fields": {"summary": "Fine", "acted_for": "ACTED SECRET"}}),
                json!({"respondent": "RESPONDENT SECRET"}),
            ],
        };
        let mut history = page(vec![private]);
        history.complete = false;
        history.next_cursor = Some("CURSOR SECRET".into());
        let (doc, lines) = draw_at(&history, 120);
        let text = lines.join("\n");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(lines.contains(&"    → doc/acme/plan@abc · as retained".to_owned()), "{lines:?}");
        assert!(lines.contains(&format!("    → doc/acme/pinned@{hash} · as retained")), "{lines:?}");
        assert!(lines.contains(&"    → mission/acme/ship · open current reference".to_owned()), "{lines:?}");
        assert!(lines.contains(&"    ↳ Fine".to_owned()) && lines.contains(&"    ↳ answer recorded".to_owned()));
        assert_eq!(lines.last().map(String::as_str), Some(PARTIAL));
        assert_eq!(
            links(&doc, &lines),
            [
                (CURRENT_SOURCE.to_owned(), Hit::Open("work/acme/ask".into())),
                (format!("doc/acme/pinned@{hash}"), Hit::Link(format!("doc/acme/pinned@{hash}"))),
                (CURRENT_REFERENCE.to_owned(), Hit::Open("mission/acme/ship".into())),
            ]
        );
    }

    #[test]
    fn an_unsupported_source_shows_no_entries() {
        let mut history = page(vec![item(key("work/acme/ask", "claim/e1"), "Title", "Body", "done")]);
        history.source_support = Some(HistorySourceSupport::UnsupportedSource);
        assert_eq!(draw(&history).1, ["Related requests", UNSUPPORTED]);
    }
}
