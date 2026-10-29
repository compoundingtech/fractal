#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use fractal_client::{Mission, scenarios::{self, Slice}};
use fractal_ui_model::missions::{Fleet, Gate, Seat, present};
use serde_json::Value;
use std::{collections::{BTreeMap, BTreeSet}, fs, path::PathBuf};

#[derive(Default)]
struct ScenarioFleet {
    seats: BTreeMap<String, Seat>,
    queues: BTreeMap<String, String>,
}

impl ScenarioFleet {
    // Match the daemon's RosterFleet::from_cards in fractal-store/store/mission_list.rs.
    fn from_cards(cards: &[Value]) -> Self {
        let mut fleet = Self::default();
        for card in cards {
            let Some(id) = card["id"].as_str() else { continue };
            for step in card["next_work_id"].as_str().into_iter().chain(
                card["upcoming_work_ids"].as_array().into_iter().flatten().filter_map(Value::as_str),
            ) {
                fleet.queues.entry(step.to_owned()).or_insert_with(|| id.to_owned());
            }
            let held = &card["current_work"][0];
            fleet.seats.insert(id.to_owned(), Seat {
                id: id.to_owned(),
                label: card["name"].as_str().unwrap_or(id).to_owned(),
                state: card["state"].as_str().unwrap_or("unknown").to_owned(),
                fault: !card["fault"].is_null(),
                busy_with: held["mission_id"].as_str().zip(held["path"].as_str())
                    .map(|(mission, path)| (mission.to_owned(), path.to_owned())),
            });
        }
        fleet
    }
}

impl Fleet for ScenarioFleet {
    fn seat(&self, agent: &str) -> Option<Seat> { self.seats.get(agent).cloned() }
    fn queued_for(&self, step: &str) -> Option<Seat> {
        self.queues.get(step).and_then(|agent| self.seat(agent))
    }
    fn mission_label(&self, mission: &str) -> String { mission.trim_start_matches("mission/").to_owned() }
    fn person_label(&self, person: &str) -> String { person.trim_start_matches("person/").to_owned() }
}

// These three slices contain frame-shaped collection changes. Fold only the requested
// collection, retaining authored order just like the kit's foldSlice.
fn rows_at(slice: &Slice, field: &str, kind: &str, at_ms: i64) -> Vec<Value> {
    let mut rows = slice.state[field].as_array().expect("collection inventory").clone();
    for event in &slice.timeline {
        if event["at_ms"].as_i64().expect("event offset") > at_ms { break; }
        assert_eq!(event["_tag"], "changes");
        for row in event["upserts"].as_array().expect("changes upserts") {
            if row["kind"] != kind { continue; }
            if let Some(previous) = rows.iter_mut().find(|previous| previous["id"] == row["id"]) {
                previous.clone_from(row);
            } else {
                rows.push(row.clone());
            }
        }
        let removes = event["removes"].as_array().expect("changes removes");
        rows.retain(|row| !removes.contains(&row["id"]));
    }
    rows
}

#[test]
fn synthetic_scenario_mission_presentations_match_daemon() {
    let root = PathBuf::from(test_env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scenarios");
    let mut checked_worlds = 0;
    // Enumerate committed fixture directories, not a hand-maintained world list.
    for world in fs::read_dir(root).unwrap() {
        let world = world.unwrap();
        if !world.file_type().unwrap().is_dir() || world.file_name().to_string_lossy().starts_with('_') { continue; }
        let details = scenarios::load_slice(world.path().join("details.json")).unwrap();
        if details.source["_tag"] != "synthetic" { continue; } // Recorded slices are faithful captures.
        let roster = scenarios::load_slice(world.path().join("roster.json")).unwrap();
        let attention = scenarios::load_slice(world.path().join("attention.json")).unwrap();
        let offsets = std::iter::once(0).chain(details.timeline.iter().map(|event|
            event["at_ms"].as_i64().expect("details offset"))).collect::<BTreeSet<_>>();
        let mut populated = false;
        for offset in offsets {
            let fleet = ScenarioFleet::from_cards(&rows_at(&roster, "agents", "agent", offset));
            // human_attention() supplies open human gates; the client attention slice carries
            // their canonical mission, optional step-run and person IDs.
            let gates = rows_at(&attention, "attention", "attention", offset).into_iter()
                .filter(|item| item["state"] == "open" && item["attention_kind"] == "human-gate")
                .filter_map(|item| Some(Gate {
                    mission: item["mission_id"].as_str()?.to_owned(),
                    step: item["step_run_id"].as_str().map(str::to_owned),
                    person: item["person_id"].as_str().expect("gate person").to_owned(),
                })).collect::<Vec<_>>();
            for row in rows_at(&details, "missions", "mission", offset) {
                let mission: Mission = serde_json::from_value(row).unwrap();
                // This is the same production function called by published_lists::present.
                let expected = serde_json::to_value(present(&mission, &gates, &fleet)).unwrap();
                assert_eq!(mission.presentation.as_ref(), Some(&expected),
                    "{} at {offset}ms: {}", details.world, mission.header.id);
                populated = true;
            }
        }
        checked_worlds += usize::from(populated);
    }
    assert!(checked_worlds > 0, "exercise synthetic missions");
}
