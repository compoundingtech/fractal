#![cfg(feature = "ratatui")]
use std::collections::BTreeMap;
use fractal_sidebar::{model::{ArrangementInput, NodeRef, Parent, Source, node_key}, projection::build,
    widget::{self, Action, Kind, Presentation, Row, Styles}};
use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Modifier, Style}};
use serde_json::{Value, json};

type OwnedRow = (String, String, Kind);
fn subject(node: &fractal_sidebar::model::ProjectedNode) -> Option<&str> {
    match &node.node { NodeRef::Subject { subject } => Some(subject), _ => None }
}

// Host presentation adapter for the same synthetic catalog used by the original TUI capture.
// The public core remains the authority for resolved folders, subjects and ordering keys.
fn rows(fixtures: &Value, state: &str) -> Vec<OwnedRow> {
    let case = &fixtures["cases"][1];
    let source = &fixtures["cases"][usize::from(state != "retired")]["initial"]["arrangement"];
    let input = ArrangementInput {
        subject: source["id"].as_str().unwrap().into(), owner: "person/test".into(),
        revision: source["revision"].as_str().unwrap().into(), deleted: false,
        body: serde_json::from_value(source["body"].clone()).unwrap(),
        resolved: serde_json::from_value(source["resolved"].clone()).unwrap(),
    };
    let catalog: Vec<String> = serde_json::from_value(case["catalog"].clone()).unwrap();
    let labels: BTreeMap<_, _> = catalog.iter().enumerate()
        .map(|(i, id)| (id.as_str(), format!("Synthetic Resource {i}"))).collect();
    let tree = build(&input, &catalog).unwrap();
    let title = |id: &str| labels.get(id).cloned().unwrap_or_else(|| id.into());
    let visible = |node: &&fractal_sidebar::model::ProjectedNode| subject(node)
        .is_some_and(|id| state != "search" || id.contains("test") || title(id).to_lowercase().contains("test"));
    let resource = |node: &fractal_sidebar::model::ProjectedNode| {
        let id = subject(node).unwrap(); let available = labels.contains_key(id);
        (node_key(&node.node), format!("   {}{}", title(id), if available { "" } else { " · unavailable" }), Kind::Resource { available })
    };
    let mut folders: Vec<_> = tree.nodes.values().filter(|n| matches!(n.node, NodeRef::Folder { .. })).collect();
    folders.sort_by(|a, b| (&a.key, &a.node).cmp(&(&b.key, &b.node)));
    let mut rows = Vec::new();
    let fractal_schema::arrangements::VersionedBody::V1(body) = &input.body else { panic!("v1 capture") };
    for folder in folders {
        let NodeRef::Folder { id } = &folder.node else { unreachable!() };
        let mut children: Vec<_> = tree.nodes.values().filter(visible)
            .filter(|n| n.shown_under == (Parent::Folder { id: id.clone() })).collect();
        children.sort_by(|a, b| (&a.key, &a.node).cmp(&(&b.key, &b.node)));
        let closed = state == "fold" && rows.is_empty();
        rows.push((node_key(&folder.node), format!(" {} {} {}", if closed { "▸" } else { "▾" }, body.folders[id].name.value, children.len()), Kind::Folder));
        if !closed { rows.extend(children.into_iter().map(resource)); }
    }
    let mut unfiled: Vec<_> = tree.nodes.values().filter(visible)
        .filter(|n| matches!(n.shown_under, Parent::Root {})).collect();
    unfiled.sort_by_key(|n| (n.key.as_deref().unwrap_or("~"), title(subject(n).unwrap()), &n.node));
    rows.push(("group:everything".into(), format!(" ▾ Everything else {}", unfiled.len()), Kind::Group));
    for (kind, available) in [("Agents", true), ("Unavailable", false)] {
        let members: Vec<_> = unfiled.iter().filter(|n| (n.source == Source::Fallback || labels.contains_key(subject(n).unwrap())) == available).collect();
        if members.is_empty() { continue; }
        rows.push((format!("group:kind/{kind}"), format!(" ▾ {kind} {}", members.len()), Kind::Group));
        rows.extend(members.into_iter().map(|n| resource(n)));
    }
    rows
}

#[test]
fn widget_matches_original_tui_buffer_and_stable_hits() {
    let fixtures: Value = serde_json::from_str(include_str!("../../../fixtures/clients/sidebar/behavior-v1.json")).unwrap();
    let schema: Value = serde_json::from_str(include_str!("../../../fixtures/clients/sidebar/behavior.schema.json")).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&fixtures));
    let expected: Value = serde_json::from_str(include_str!("../../../fixtures/clients/sidebar/tui-v1.snapshot")).unwrap();
    let styles = Styles {
        background: Style::default().bg(Color::Rgb(0x18, 0x18, 0x25)),
        dim: Style::default().fg(Color::Rgb(0x6c, 0x70, 0x86)),
        label: Style::default().fg(Color::Rgb(0x7f, 0x84, 0x9c)).add_modifier(Modifier::BOLD),
        text: Style::default().fg(Color::Rgb(0xcd, 0xd6, 0xf4)),
        accent: Style::default().fg(Color::Rgb(0x89, 0xb4, 0xfa)),
        selected: Style::default().fg(Color::Rgb(0x11, 0x11, 0x1b)).bg(Color::Rgb(0x89, 0xb4, 0xfa)),
    };
    let rect = |r: Rect| json!([r.x, r.y, r.width, r.height]);
    let mut actual = Vec::new();
    for state in ["reserved", "legacy", "retired", "unfocused", "fold", "search", "scroll", "narrow", "zero-width", "zero-height", "zero"] {
        let owned = rows(&fixtures, state);
        let rows: Vec<_> = owned.iter().map(|(id, label, kind)| Row { id, label, kind: *kind }).collect();
        let selected = if state == "scroll" { rows.len().saturating_sub(1) } else { 0 };
        let (width, height) = match state {
            "scroll" => (38, 4), "narrow" => (7, 6), "zero-width" => (0, 10),
            "zero-height" => (38, 0), "zero" => (0, 0), _ => (38, 10),
        };
        let area = Rect::new(2, 1, width, height);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 44, 14));
        let regions = widget::render(&mut buffer, area, &rows, &Presentation {
            filter: if state == "search" { "test" } else { "" }, selected, focused: state != "unfocused",
        }, &styles);
        let hits: Vec<_> = regions.hits.iter().map(|hit| {
            let action = match hit.action { Action::Filter => "filter", Action::New => "new", Action::Row => "row",
                Action::Add => "add", Action::Menu => "menu", Action::Drag => "drag" };
            json!([rect(hit.rect), action, hit.row])
        }).collect();
        let contexts: Vec<_> = regions.contexts.iter().map(|(r, id)| json!([rect(*r), id, 0])).collect();
        let mut cells: Vec<Value> = Vec::new();
        for cell in &buffer.content {
            let value = json!([cell.symbol(), format!("{cell:?}")]);
            if cells.last().is_some_and(|run| run[1] == value) {
                let run = cells.last_mut().unwrap(); run[0] = json!(run[0].as_u64().unwrap() + 1);
            } else { cells.push(json!([1, value])); }
        }
        let arrangement = match state {
            "retired" => None,
            "legacy" => Some("arrangement/person/test/00000000-0000-7000-8000-000000000002"),
            _ => Some("arrangement/person/test/00000000-0000-7000-8000-000000000001"),
        };
        actual.push(json!({"state": state, "area": rect(area), "arrangement": arrangement,
            "selected": rows.get(selected).map(|r| r.id), "rows": rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            "cells": cells, "hits": hits, "contexts": contexts}));
    }
    assert_eq!(json!(actual), expected["cases"]);
}
