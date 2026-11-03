use super::*;
use crate::ui::pane::Pane;
use fractal_client::{DocumentContent, DocumentItem, DocumentsPage};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// 2026-10-13 12:00:00 UTC, and three hours later.
const CHANGED: u64 = 1_791_892_800_000;
const NOW: u64 = CHANGED + 3 * 3_600_000;
/// A wide pane: the kdl line with its whole hash fits on one row.
const WIDE: usize = 120;

/// The shared long document (tables, fenced code, nested lists, a link cycle partner) with a
/// nested list that holds a fenced block, a relative link and a doc/ link added at its end.
fn long_text() -> String {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../../fixtures/clients/documents-v1.json")).unwrap();
    let mut text = fixture["links"][0]["text"].as_str().unwrap().to_owned();
    text.push_str(concat!(
        "\n## Appendix\n\n",
        "- Tools\n",
        "  - Run it:\n",
        "    ```sh\n",
        "    garden plant --bed north\n",
        "    ```\n",
        "  - See [the soil notes](./soil.md) and [the harvest](doc/garden/harvest).\n",
    ));
    text
}

fn content(reference: &str, text: &str, latest: bool) -> DocumentContent {
    let (name, version) = reference.split_once('@').unwrap();
    serde_json::from_value(serde_json::json!({
        "reference": reference,
        "bytes": text.as_bytes(),
        "name": name,
        "version": version,
        "size": text.len(),
        "changed_at_unix_ms": CHANGED,
        "latest": latest,
        "owner": "agent/example/orchard/keeper",
    }))
    .unwrap()
}

fn plain(doc: &Doc) -> Vec<String> {
    doc.lines.iter().map(|line| text::plain(line)).collect()
}

fn hit_on(doc: &Doc, line: usize) -> Option<&Hit> {
    doc.targets.iter().find(|target| target.line == line).map(|target| &target.hit)
}

#[test]
fn a_long_document_renders_as_written_with_its_version_and_changed_time() {
    let mut documents = Documents::default();
    let reference = format!("doc/garden/plan@{HASH_A}");
    let pane = Pane::Document(reference.clone());
    assert_eq!(documents.wanted(std::slice::from_ref(&pane), &[], &[]), vec![Want::Get(reference.clone())]);
    assert!(documents.wanted(std::slice::from_ref(&pane), &[], &[]).is_empty(), "one read in flight at a time");
    assert_eq!(plain(&documents.page(&reference, None, WIDE, NOW)), vec![format!("Loading {reference}…")]);

    let text = long_text();
    documents.got(&reference, Ok(content(&reference, &text, true)));
    let doc = documents.page(&reference, None, WIDE, NOW);
    let lines = plain(&doc);
    assert_eq!(lines[0], "doc/garden/plan  latest");
    assert!(lines[1].starts_with("version aaaaaaaaaaaa · changed 3h ago (2026-10-13 "), "{}", lines[1]);
    assert!(format!("{} {}", lines[1], lines[2]).contains("by agent/example/orchard/keeper"), "{lines:?}");
    let body = lines.join("\n");
    // Headings, a table as a grid, fenced code with its language, nested lists with code: all there.
    assert!(lines.iter().any(|line| line == "Garden release plan"));
    assert!(lines.iter().any(|line| line.starts_with("Bed") && line.contains(" │ Owner") && line.contains(" │ Notes")), "{body}");
    assert!(lines.iter().any(|line| line.starts_with("North") && line.contains("doc/garden/beds/north")), "{body}");
    assert!(lines.iter().any(|line| line == "```kdl"));
    assert!(lines.iter().any(|line| line.trim() == r#"document "doc/garden/seeds@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa""#), "{body}");
    assert!(lines.iter().any(|line| line.trim_start().starts_with("1. Water the beds")), "{body}");
    assert!(lines.iter().any(|line| line.trim() == "garden plant --bed north"), "{body}");
    // Every line of the author's text is on the page: none filtered away.
    assert!(lines.iter().any(|line| line == "Line 399: plain text with no links."), "{body}");
    // Markdown links show their words, and the address behind them is the link.
    assert!(lines.iter().any(|line| line.contains("See the soil notes and the harvest.")), "{body}");
    assert_eq!(fractal_conversation_ui::text::link_for("the harvest").as_deref(), Some("doc/garden/harvest"));
    assert_eq!(fractal_conversation_ui::text::link_for("the soil notes").as_deref(), Some("./soil.md"));

    // The page ends with its outgoing links, each once, each one a click on that document.
    let links = lines.iter().position(|line| line.starts_with("Links ")).expect(&body);
    let expected = fractal_ui_model::documents::document_links(&text);
    assert_eq!(lines[links], format!("Links {} {}", expected.len(), "─".repeat(WIDE - 7 - expected.len().to_string().len())));
    for (offset, link) in expected.iter().enumerate() {
        assert_eq!(lines[links + 1 + offset], format!("→ {}", link.reference));
        assert_eq!(hit_on(&doc, links + 1 + offset), Some(&Hit::Link(link.reference.clone())));
    }
    assert!(expected.iter().any(|link| link.reference == "doc/garden/harvest"));
    assert!(expected.iter().any(|link| link.reference == "doc/garden/checklist"));
    assert!(!expected.iter().any(|link| link.reference.contains("not-a-link")));
}

#[test]
fn a_document_that_cannot_be_shown_says_why() {
    for (why, title, words) in [
        (Unavailable::NotFound, "not found", "No document named doc/garden/gone is on this host."),
        (Unavailable::NotSynced, "not synced yet", "doc/garden/gone has not reached this host yet. It shows once it syncs."),
        (Unavailable::RemoteUnavailable, "remote unavailable", "The host did not answer, so doc/garden/gone cannot be read now."),
    ] {
        let mut documents = Documents::default();
        documents.wanted(&[Pane::Document("doc/garden/gone".into())], &[], &[]);
        documents.got("doc/garden/gone", Err(why));
        let doc = documents.page("doc/garden/gone", None, 80, NOW);
        let lines = plain(&doc);
        assert!(lines[0].to_lowercase().contains(title), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains(words)), "{lines:?}");
        let retry = doc.targets.iter().find(|target| target.hit == Hit::Document(Tap::Retry));
        assert!(retry.is_some(), "a way to read it again");
    }
    // Bytes that are not UTF-8 are not text: said, never shown as a blank page.
    let mut documents = Documents::default();
    let reference = format!("doc/garden/photo@{HASH_A}");
    let mut photo = content(&reference, "", true);
    photo.bytes = vec![0xff, 0xfe, 0x00];
    documents.got(&reference, Ok(photo));
    assert!(plain(&documents.page(&reference, None, 80, NOW)).iter().any(|line| line.contains("is not text, so it cannot be shown.")));
}

#[test]
fn a_document_not_synced_is_read_again_and_a_read_page_stays_up() {
    let mut documents = Documents::default();
    let pane = Pane::Document("doc/garden/late".into());
    documents.wanted(std::slice::from_ref(&pane), &[], &[]);
    documents.got("doc/garden/late", Err(Unavailable::NotSynced));
    assert!(documents.wanted(std::slice::from_ref(&pane), &[], &[]).is_empty(), "not again at once");
    documents.retry(&pane);
    assert_eq!(documents.wanted(std::slice::from_ref(&pane), &[], &[]), vec![Want::Get("doc/garden/late".into())]);
    let reference = format!("doc/garden/late@{HASH_A}");
    documents.got("doc/garden/late", Ok(content(&reference, "# Late\n\nArrived.", true)));
    documents.retry(&pane);
    documents.wanted(std::slice::from_ref(&pane), &[], &[]);
    documents.got("doc/garden/late", Err(Unavailable::RemoteUnavailable));
    assert!(plain(&documents.page("doc/garden/late", None, 80, NOW)).iter().any(|line| line == "Arrived."));
}

#[test]
fn following_links_goes_back_and_a_cycle_never_grows_the_trail() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../../fixtures/clients/documents-v1.json")).unwrap();
    let mut documents = Documents::default();
    let split = "glass/0";
    let mut shown = "doc/cycle/a".to_owned();
    let opened = fixture["trail"]["open"].as_array().unwrap();
    for next in opened.iter().skip(1) {
        shown = documents.follow(split, &shown, next.as_str().unwrap());
    }
    let expected = fixture["trail"]["expected"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(documents.trails[split].references, expected);
    assert_eq!(shown, "doc/cycle/b");
    // The page shows the way back, and back goes one document.
    assert_eq!(documents.previous(split, &shown), Some("doc/cycle/a"));
    let page = documents.page(&shown, documents.previous(split, &shown), 80, NOW);
    assert_eq!(plain(&page)[0].trim_end(), " b  back to doc/cycle/a");
    assert_eq!(hit_on(&page, 0), Some(&Hit::Document(Tap::Back)));
    assert_eq!(documents.back(split, &shown).as_deref(), Some("doc/cycle/a"));
    assert_eq!(documents.back(split, "doc/cycle/a"), None, "the first stays");
    // A split that shows a document opened some other way starts its trail there.
    assert_eq!(documents.follow(split, "doc/garden/plan", "doc/garden/checklist"), "doc/garden/checklist");
    assert_eq!(documents.previous(split, "doc/garden/checklist"), Some("doc/garden/plan"));
}

#[test]
fn a_page_read_again_draws_its_new_text() {
    let mut documents = Documents::default();
    let reference = "doc/garden/plan";
    documents.got(reference, Ok(content(&format!("{reference}@{HASH_A}"), "First draft.", true)));
    assert!(plain(&documents.page(reference, None, 80, NOW)).contains(&"First draft.".to_owned()));
    assert!(plain(&documents.page(reference, None, 80, NOW)).contains(&"First draft.".to_owned()), "drawn again from what was rendered");
    documents.got(reference, Ok(content(&format!("{reference}@{HASH_B}"), "Second draft.", true)));
    let lines = plain(&documents.page(reference, None, 80, NOW));
    assert!(lines.contains(&"Second draft.".to_owned()) && !lines.contains(&"First draft.".to_owned()), "{lines:?}");
}

/// Each link target in a page's text (above its Links section): the text under it and the
/// address it opens, top to bottom.
fn link_targets(doc: &Doc) -> Vec<(String, String)> {
    let lines = plain(doc);
    let body = lines.iter().position(|line| line.starts_with("Links ")).unwrap_or(lines.len());
    doc.targets
        .iter()
        .filter_map(|target| match &target.hit {
            Hit::Link(url) if target.line < body => {
                let under = lines[target.line].chars().skip(target.column as usize).take(target.width as usize).collect::<String>();
                Some((under, url.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn each_link_opens_its_own_address_however_the_page_is_drawn() {
    // The same text for different documents: twice on one page, in a table whose cells wrap side
    // by side, and on another page drawn in between. The cached page keeps every target.
    let mut documents = Documents::default();
    let (first, second) = (format!("doc/garden/north@{HASH_A}"), format!("doc/garden/south@{HASH_B}"));
    let text = concat!(
        "See [the details](doc/garden/north-bed).\n\n",
        "Then [the details](doc/garden/east-bed).\n\n",
        "| Bed | Notes |\n|---|---|\n",
        "| [the long north bed by the fence](doc/garden/fence) | [the details](doc/garden/west-bed) |\n",
    );
    documents.got(&first, Ok(content(&first, text, true)));
    documents.got(&second, Ok(content(&second, "See [the details](doc/garden/south-bed).", true)));
    let targets = |doc: &Doc| link_targets(doc).into_iter().map(|(_, url)| url).collect::<Vec<_>>();
    let shown = documents.page(&first, None, 30, NOW);
    let mut urls = targets(&shown);
    urls.dedup();
    assert_eq!(
        urls,
        ["doc/garden/north-bed", "doc/garden/east-bed", "doc/garden/fence", "doc/garden/west-bed", "doc/garden/fence", "doc/garden/west-bed"],
        "the table's first cell wraps beside the second: {:?}",
        plain(&shown)
    );
    for (under, url) in link_targets(&shown) {
        let words = if url == "doc/garden/fence" { "the long north bed by the fence" } else { "the details" };
        assert!(!under.trim().is_empty() && words.contains(under.trim()), "{under:?} opens {url}");
    }
    assert_eq!(targets(&documents.page(&second, None, 30, NOW)), ["doc/garden/south-bed"]);
    assert_eq!(link_targets(&documents.page(&first, None, 30, NOW)), link_targets(&shown), "drawn from the cache, with its own targets");
}

#[test]
fn a_wrapped_pinned_reference_opens_its_pinned_version_from_every_row() {
    // A doc/ reference written out with its hash is wider than the pane: every row it wraps to
    // opens that version, never the latest one the name alone would open.
    let mut documents = Documents::default();
    let reference = format!("doc/garden/notes@{HASH_A}");
    let pinned = format!("doc/garden/plan@{HASH_B}");
    documents.got(&reference, Ok(content(&reference, &format!("The plan as it was: {pinned}."), true)));
    let shown = documents.page(&reference, None, 30, NOW);
    let targets = link_targets(&shown);
    assert!(targets.len() >= 3, "{:?}", plain(&shown));
    assert!(targets.iter().all(|(_, url)| *url == pinned), "{targets:?}");
    assert_eq!(targets.iter().map(|(under, _)| under.as_str()).collect::<String>(), pinned);
}

#[test]
fn a_page_with_thousands_of_links_keeps_every_target() {
    // More links than the shared text-to-address table holds before it starts again.
    let mut documents = Documents::default();
    let reference = format!("doc/garden/index@{HASH_A}");
    let text = (0..2100).map(|bed| format!("- [bed {bed}](doc/garden/bed-{bed})\n")).collect::<String>();
    documents.got(&reference, Ok(content(&reference, &text, true)));
    let targets = link_targets(&documents.page(&reference, None, 80, NOW));
    assert_eq!(targets.len(), 2100);
    for (bed, (under, url)) in targets.iter().enumerate() {
        assert_eq!((under.as_str(), url.as_str()), (format!("bed {bed}").as_str(), format!("doc/garden/bed-{bed}").as_str()));
    }
}

#[test]
fn n_and_p_choose_a_link_that_enter_opens() {
    let mut documents = Documents::default();
    let reference = format!("doc/garden/plan@{HASH_A}");
    documents.got(&reference, Ok(content(&reference, "See doc/garden/one and doc/garden/two.", true)));
    let pane = Pane::Document(reference.clone());
    assert_eq!(documents.chosen(&pane), None);
    documents.choose(&pane, 1);
    assert_eq!(documents.chosen(&pane).as_deref(), Some("doc/garden/one"));
    documents.choose(&pane, 1);
    documents.choose(&pane, 1);
    assert_eq!(documents.chosen(&pane).as_deref(), Some("doc/garden/one"), "round again");
    documents.choose(&pane, -1);
    assert_eq!(documents.chosen(&pane).as_deref(), Some("doc/garden/two"));
    let lines = plain(&documents.page(&reference, None, 80, NOW));
    assert!(lines.contains(&"▸ doc/garden/two".to_owned()), "{lines:?}");
}

fn item(name: &str, hash: &str) -> DocumentItem {
    serde_json::from_value(serde_json::json!({
        "reference": format!("{name}@{hash}"), "name": name, "version": hash, "size": 12,
        "changed_at_unix_ms": CHANGED, "latest": true,
    }))
    .unwrap()
}

#[test]
fn the_list_pages_under_a_prefix_and_keeps_loading_empty_and_failed_apart() {
    let mut documents = Documents::default();
    let pane = Pane::Documents(Some("doc/garden/".into()));
    assert_eq!(documents.wanted(std::slice::from_ref(&pane), &[], &[]), vec![Want::List { prefix: Some("doc/garden/".into()), cursor: None }]);
    assert!(plain(&documents.list(Some("doc/garden/"), 80, NOW)).contains(&"Loading…".to_owned()));
    let page: DocumentsPage = serde_json::from_value(serde_json::json!({
        "items": [item("doc/garden/beds", HASH_A), item("doc/garden/plan", HASH_B)],
        "has_more": true, "next_cursor": "next-1", "limit": 100, "history": false,
    }))
    .unwrap();
    documents.listed(Some("doc/garden/"), Ok(page));
    let doc = documents.list(Some("doc/garden/"), 80, NOW);
    let lines = plain(&doc);
    assert_eq!(lines[0], "Documents  under doc/garden/  2+");
    assert!(lines[2].starts_with("→ doc/garden/beds  aaaaaaaaaaaa · changed 3h ago"), "{lines:?}");
    // A row opens the document by name, as a link to it does: following links back returns.
    assert_eq!(hit_on(&doc, 2), Some(&Hit::Link("doc/garden/beds".into())));
    assert!(doc.targets.iter().any(|target| target.hit == Hit::Document(Tap::More)));
    // m asks for the next page, once.
    documents.more(Some("doc/garden/"));
    assert_eq!(documents.wanted(std::slice::from_ref(&pane), &[], &[]), vec![Want::List { prefix: Some("doc/garden/".into()), cursor: Some("next-1".into()) }]);
    assert!(documents.wanted(std::slice::from_ref(&pane), &[], &[]).is_empty());
    let last: DocumentsPage = serde_json::from_value(serde_json::json!({ "items": [item("doc/garden/seeds", HASH_A)], "has_more": false, "limit": 100, "history": false })).unwrap();
    documents.listed(Some("doc/garden/"), Ok(last));
    let lines = plain(&documents.list(Some("doc/garden/"), 80, NOW));
    assert_eq!(lines[0], "Documents  under doc/garden/  3");
    assert_eq!(documents.listed_rows().len(), 3);

    let mut empty = Documents::default();
    empty.wanted(&[Pane::Documents(None)], &[], &[]);
    empty.listed(None, Ok(serde_json::from_value(serde_json::json!({ "items": [], "has_more": false, "limit": 100, "history": false })).unwrap()));
    assert!(plain(&empty.list(None, 80, NOW)).contains(&"No documents under doc/.".to_owned()));
    let mut failed = Documents::default();
    failed.wanted(&[Pane::Documents(None)], &[], &[]);
    failed.listed(None, Err(Unavailable::RemoteUnavailable));
    let lines = plain(&failed.list(None, 80, NOW));
    assert!(lines.contains(&"The host did not answer, so the documents cannot be listed now.".to_owned()), "{lines:?}");
}

#[test]
fn every_mission_on_screen_is_asked_for_its_documents() {
    // The first mission's 40 steps do not use up the second mission's turn.
    let mut documents = Documents::default();
    let mut subjects = vec!["mission/example/orchard/plan".to_owned()];
    subjects.extend((0..STEPS).map(|step| format!("step-run/0001/bed-{step}")));
    subjects.push("mission/example/orchard/harvest".to_owned());
    let wants = documents.wanted(&[], &[], &subjects);
    assert_eq!(wants.len(), STEPS + 2);
    assert_eq!(wants.last(), Some(&Want::References("mission/example/orchard/harvest".into())));
}

#[test]
fn a_mission_lists_the_documents_it_and_its_steps_name() {
    let mut documents = Documents::default();
    let subjects = vec![
        ("mission/example/orchard/plan".to_owned(), None),
        ("step-run/0001/plant".to_owned(), Some("plant".to_owned())),
    ];
    let wants = documents.wanted(&[], &[], &subjects.iter().map(|(subject, _)| subject.clone()).collect::<Vec<_>>());
    assert_eq!(wants, vec![Want::References("mission/example/orchard/plan".into()), Want::References("step-run/0001/plant".into())]);
    let mut doc = Doc::new();
    documents.mission_section(&mut doc, &subjects, 80);
    assert!(doc.lines.is_empty(), "nothing before st says");
    documents.referenced("mission/example/orchard/plan", Ok(serde_json::from_value(serde_json::json!({
        "subject": "mission/example/orchard/plan", "complete": true,
        "references": [{ "reference": "doc/garden/plan", "name": "doc/garden/plan", "source": "step", "step": "survey" }],
    })).unwrap()));
    documents.referenced("step-run/0001/plant", Ok(serde_json::from_value(serde_json::json!({
        "subject": "step-run/0001/plant", "complete": true,
        "references": [
            { "reference": format!("doc/garden/seeds@{HASH_A}"), "name": "doc/garden/seeds", "version": HASH_A, "source": "step" },
            { "reference": "doc/garden/plan", "name": "doc/garden/plan", "source": "step" },
        ],
    })).unwrap()));
    documents.mission_section(&mut doc, &subjects, 80);
    let lines = plain(&doc);
    assert!(lines[1].starts_with("Documents 2 "), "{lines:?}");
    assert_eq!(lines[2], "→ doc/garden/plan  step survey");
    assert_eq!(lines[3], "→ doc/garden/seeds@aaaaaaaaaaaa  step plant");
    assert_eq!(hit_on(&doc, 3), Some(&Hit::Link(format!("doc/garden/seeds@{HASH_A}"))));
}

#[test]
fn only_document_references_open_as_documents() {
    assert_eq!(openable(" doc/garden/plan "), Some("doc/garden/plan"));
    assert!(openable(&format!("doc/garden/plan@{HASH_A}")).is_some());
    for not in ["doc/", "doc/garden/", "doc/garden/plan@abc", "mission/example/orchard", "https://example.com/doc/x"] {
        assert_eq!(openable(not), None, "{not}");
    }
    assert!(inside_word("my", "x"));
    assert!(inside_word("", "-more"));
    assert!(!inside_word("see ", ". Then"));
    assert!(!inside_word("(", ")"));
}

#[test]
fn a_failed_read_says_which_state_it_is() {
    let api = |code: &str| {
        let envelope: fractal_client::ErrorEnvelope = serde_json::from_value(serde_json::json!({
            "api_version": "v0", "error_version": "1", "request_id": "request-1", "code": code, "message": "no", "retryable": false,
        }))
        .unwrap();
        ClientError::Api(serde_json::from_value(serde_json::json!(code)).unwrap(), "no".into(), Box::new(envelope))
    };
    assert_eq!(unavailable(&api("document-not-synced")), Unavailable::NotSynced);
    assert_eq!(unavailable(&api("not-found")), Unavailable::NotFound);
    assert_eq!(unavailable(&ClientError::Transport("reset".into())), Unavailable::RemoteUnavailable);
}

#[test]
fn without_a_connection_a_page_says_the_host_did_not_answer() {
    let mut documents = Documents::default();
    documents.offline(&[Pane::Document("doc/garden/plan".into())]);
    assert!(plain(&documents.page("doc/garden/plan", None, 80, NOW)).iter().any(|line| line.contains("The host did not answer")));
    assert_eq!(documents.wanted(&[Pane::Document("doc/garden/plan".into())], &[], &[]), vec![Want::Get("doc/garden/plan".into())]);
}
