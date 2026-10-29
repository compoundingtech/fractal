//! The shared client contract in `fixtures/clients`: what every Fractal client shows.
//!
//! Fractal TUI writes these files and the iOS app tests against them, so both apps clean the same
//! transcripts the same way, use the same words and colours, draw a conversation's entries
//! by the same rules, and share one demo world.
//! Run `STUI_UPDATE_CONTRACT=1 cargo test -p fractal-tui contract` after an intended change.

use super::adapt;
use super::demo;
use super::theme;
use super::view::*;
use ratatui::style::Color;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The lists every client offers, by name. Worktrees stays hidden until the graph models
/// worktrees: its screens only have invented data to show (Nathan, 2026-09-28).
const LISTS: [&str; 5] = ["Now", "Agents", "Missions", "Fleet", "Usage"];

fn root() -> PathBuf {
    PathBuf::from(test_env!("CARGO_MANIFEST_DIR")).join("../../fixtures/clients")
}

fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}"),
    }
}

pub fn theme_tokens() -> Value {
    let tokens = [
        ("base", theme::BASE),
        ("mantle", theme::MANTLE),
        ("crust", theme::CRUST),
        ("surface0", theme::SURFACE0),
        ("surface1", theme::SURFACE1),
        ("surface2", theme::SURFACE2),
        ("overlay0", theme::OVERLAY0),
        ("overlay1", theme::OVERLAY1),
        ("subtext0", theme::SUBTEXT0),
        ("subtext1", theme::SUBTEXT1),
        ("text", theme::TEXT),
        ("lavender", theme::LAVENDER),
        ("blue", theme::BLUE),
        ("sapphire", theme::SAPPHIRE),
        ("teal", theme::TEAL),
        ("green", theme::GREEN),
        ("yellow", theme::YELLOW),
        ("peach", theme::PEACH),
        ("red", theme::RED),
        ("mauve", theme::MAUVE),
        ("accent", theme::ACCENT),
        ("person", theme::PERSON),
        ("working", theme::WORKING),
        ("idle", theme::IDLE),
        ("done", theme::DONE),
        ("fault", theme::FAULT),
        ("waiting", theme::WAITING),
        ("quiet", theme::QUIET),
        ("row_selected", theme::ROW_SELECTED),
        ("user_bg", theme::USER_BG),
        ("tool_bg", theme::TOOL_BG),
        ("tool_ok_bg", theme::TOOL_OK_BG),
        ("tool_err_bg", theme::TOOL_ERR_BG),
        ("selection_bg", theme::SELECTION_BG),
    ];
    json!({
        "name": "Catppuccin Mocha",
        "rule": "person (mauve) means a person is needed and is used for nothing else",
        "colors": tokens.iter().map(|(name, color)| (name.to_string(), json!(hex(*color)))).collect::<serde_json::Map<_, _>>(),
    })
}

pub fn words() -> Value {
    let mission = [
        Word::Decision,
        Word::Stalled,
        Word::Unstaffed,
        Word::Unclaimed,
        Word::Queued,
        Word::Working,
        Word::Watching,
        Word::Held,
        Word::Idle,
        Word::Done,
        Word::Failed,
        Word::Cancelled,
        Word::NotStarted,
        Word::Unknown,
    ]
    .iter()
    .map(|word| {
        json!({
            "id": serde_json::to_value(word).unwrap(),
            "name": word.name(),
            "explain": word.explain(),
            "glyph": super::screens::word_style(*word, "⠋").0,
        })
    })
    .collect::<Vec<_>>();
    let agent = [
        AgentState::NeedsYou,
        AgentState::NeedsLogin,
        AgentState::Fault,
        AgentState::Working,
        AgentState::Compacting,
        AgentState::Idle,
        AgentState::Starting,
        AgentState::Down,
        AgentState::Stopped,
        AgentState::Retired,
        AgentState::Unknown,
    ]
    .iter()
    .map(|state| {
        json!({
            "id": serde_json::to_value(state).unwrap(),
            "name": super::screens::agent_word(*state),
            "glyph": super::screens::agent_glyph(*state, "⠋").0,
        })
    })
    .collect::<Vec<_>>();
    let tier = [Tier::Stopped, Tier::Alert, Tier::Today, Tier::Later]
        .iter()
        .map(|tier| json!({ "id": serde_json::to_value(tier).unwrap(), "title": tier.title() }))
        .collect::<Vec<_>>();
    json!({ "mission": mission, "agent": agent, "tier": tier, "tabs": LISTS })
}

/// The cleaned conversation. `at` is a local display time, so it is left out: it depends on
/// the machine's time zone, not on the cleaning rules.
fn conversation_of(fixture: &str) -> Value {
    let timeline: Vec<fractal_client::TimelineEntry> = serde_json::from_str(fixture).unwrap();
    let mut value =
        serde_json::to_value(adapt::conversation(&timeline, &Default::default())).unwrap();
    for entry in value.as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("at");
    }
    value
}

fn files() -> Vec<(&'static str, Value)> {
    vec![
        (
            "demo-world.json",
            serde_json::to_value(demo::contract_world()).unwrap(),
        ),
        ("demo-scenario-inputs.json", demo::contract_inputs()),
        ("theme.json", theme_tokens()),
        ("words.json", words()),
        (
            "conversation-style.json",
            serde_json::to_value(fractal_conversation_ui::style::RULES).unwrap(),
        ),
        (
            "transcripts/claude.expected.json",
            conversation_of(include_str!(
                "../../../../fixtures/clients/transcripts/claude.json"
            )),
        ),
        (
            "transcripts/codex.expected.json",
            conversation_of(include_str!(
                "../../../../fixtures/clients/transcripts/codex.json"
            )),
        ),
        (
            "transcripts/deliveries.expected.json",
            conversation_of(include_str!(
                "../../../../fixtures/clients/transcripts/deliveries.json"
            )),
        ),
        (
            "transcripts/noise.expected.json",
            conversation_of(include_str!(
                "../../../../fixtures/clients/transcripts/noise.json"
            )),
        ),
    ]
}

/// Sorts every object's keys, so the files read the same whether or not another crate in the
/// build turns on serde_json's `preserve_order` (the workspace build does; `-p fractal-tui` does not).
fn sorted_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, sorted_keys(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted_keys).collect()),
        other => other,
    }
}

#[test]
fn the_shared_client_contract_matches_tui() {
    let update = std::env::var_os("STUI_UPDATE_CONTRACT").is_some();
    for (name, value) in files() {
        let path = root().join(name);
        let text = serde_json::to_string_pretty(&sorted_keys(value)).unwrap() + "\n";
        if update {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let current = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{name} is missing; run with STUI_UPDATE_CONTRACT=1"));
        assert!(
            current == text,
            "{name} no longer matches Fractal TUI; if the change is intended, run STUI_UPDATE_CONTRACT=1 cargo test -p fractal-tui contract and update the iOS app"
        );
    }
}

/// Plain-text product review capture, rendered by the same TestBackend as UI tests.
#[test]
fn scenario_demo_first_frames_match_golden() {
    use super::{Ui, Pane, glass};
    use ratatui::{Terminal, backend::TestBackend};
    let world = demo::contract_world();
    let mut actual = String::new();
    for (title, pane) in [
        ("Now", Pane::List(0)),
        ("Roster", Pane::List(1)),
        ("Mission", Pane::Mission(Some(world.missions.items()[0].id.clone()))),
        ("Fleet", Pane::List(3)),
        ("Worktrees unavailable", Pane::List(5)),
        ("Machine and devices", Pane::Machine(Some("machine/meadow".into()))),
        ("Conversation", Pane::Agent(Some("agent/example/atlas/builder".into()))),
        ("Agent details", Pane::Agent(Some("agent/example/atlas/builder".into()))),
        ("Terminal", Pane::Terminal("agent/example/atlas/builder".into())),
    ] {
        let mut ui = Ui::new(world.clone());
        ui.glasses = glass::Glasses::open(None, None);
        ui.glasses.sidebar.shown = true;
        if title == "Agent details" { ui.details_here.insert("agent/example/atlas/builder".into()); }
        if matches!(pane, Pane::Terminal(_)) { ui.attach_terminal("agent/example/atlas/builder"); }
        ui.open_in_glass(pane, glass::Open::Tab);
        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        actual.push_str(&format!("=== {title} ===\n"));
        let buffer = terminal.backend().buffer();
        for y in 0..32 {
            let line: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
            actual.push_str(line.trim_end());
            actual.push('\n');
        }
        actual.push('\n');
    }
    let path = PathBuf::from(test_env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scenario-demo.golden");
    if std::env::var_os("STUI_UPDATE_CONTRACT").is_some() {
        std::fs::write(&path, &actual).unwrap();
    } else {
        assert_eq!(std::fs::read_to_string(path).unwrap(), actual,
            "scenario demo frames drifted; regenerate with STUI_UPDATE_CONTRACT=1");
    }
}

#[test]
fn agent_branch_and_checkout_label_match_the_phone() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/clients/agent-launch.json"
    ))
    .unwrap();
    for case in fixture["branches"].as_array().unwrap() {
        assert_eq!(
            fractal_client::agent_branch(case["name"].as_str().unwrap()),
            case["branch"]
        );
    }
    let checkout = serde_json::from_value(fixture["checkout"].clone()).unwrap();
    assert_eq!(
        fractal_client::checkout_label(&checkout),
        fixture["checkout_label"]
    );
}
