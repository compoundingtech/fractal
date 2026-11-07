//! The kit's committed fleet-mid-refactor world, decoded before any screen opens.

use super::{adapt, view::*};
use crate::model::{Collection, Model};
use anyhow::{Context, Result, ensure};
use fractal_client::{Resource, Snapshot, scenarios::{self, DecodeMode, WireValue}};
use std::collections::BTreeMap;

#[cfg(test)]
pub use super::demo_test_fixture::{world as test_world, late_mail};

const SLICES: &[(&str, &str)] = &[
    ("roster", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/roster.json")),
    ("details", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/details.json")),
    ("attention", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/attention.json")),
    ("conversation", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/conversation.json")),
    ("terminal", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/terminal.json")),
    ("sync", include_str!("../../../../fixtures/scenarios/fleet-mid-refactor/sync.json")),
];

pub(super) struct DemoWorld {
    pub world: World,
    pub terminals: BTreeMap<String, fractal_client::TerminalScreen>,
    pub arrangements: Vec<fractal_client::Arrangement>,
}

pub(super) fn load() -> Result<DemoWorld> {
    load_at(SLICES, chrono::Utc::now().timestamp_millis())
}

fn load_at(sources: &[(&str, &str)], now: i64) -> Result<DemoWorld> {
    let mut model = Model::default();
    model.actor = "person/robin".into();
    let mut extras = adapt::Extras {
        live: true, missions_followed: true, attention_live: true, agents_live: true,
        inventory_supported: Some(true), ..Default::default()
    };
    let snapshot = Snapshot {
        id: "snapshot/scenario-demo".into(), host_id: "host/meadow".into(),
        store_index: 1, projection_version: "scenario-demo".into(),
        created_at: chrono::DateTime::from_timestamp_millis(now).context("invalid demo clock")?.to_rfc3339(),
        published_at: None,
    };
    for collection in [&mut model.now, &mut model.messages, &mut model.launches,
        &mut model.missions, &mut model.work, &mut model.agents, &mut model.sessions,
        &mut model.runtimes, &mut model.machines, &mut model.devices] {
        collection.snapshot = Some(snapshot.clone());
    }
    let mut conversations: BTreeMap<String, Vec<fractal_client::TimelineEntry>> = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    let mut arrangements = Vec::new();
    for &(name, source) in sources {
        let slice = scenarios::parse_slice(source)
            .and_then(|slice| {
                ensure!(slice.world == "fleet-mid-refactor" && slice.slice == name,
                    "unexpected world or slice");
                ensure!(slice.decode == DecodeMode::Strict, "demo requires strict decode");
                slice.rebased(now)
            }).with_context(|| format!("demo scenario fleet-mid-refactor/{name}.json"))?;
        let decoded = slice.decode()
            .with_context(|| format!("demo scenario fleet-mid-refactor/{name}.json strict decode failed"))?;
        for item in decoded {
            // Validate replay too, but the initial screen is the slice's initial state.
            if !item.pointer.starts_with("/state/") || item.pointer.starts_with("/state/recorded_history/") { continue; }
            match item.value {
                WireValue::Resource(resource) => {
                    let collection: Option<&mut Collection> = match resource.as_ref() {
                        Resource::Agent(_) => Some(&mut model.agents),
                        Resource::Session(_) => Some(&mut model.sessions),
                        Resource::Runtime(_) => Some(&mut model.runtimes),
                        Resource::Machine(_) => Some(&mut model.machines),
                        Resource::Device(_) => Some(&mut model.devices),
                        Resource::Mission(_) => Some(&mut model.missions),
                        Resource::Work(_) => Some(&mut model.work),
                        Resource::Attention(_) => Some(&mut model.now),
                        Resource::Message(_) => Some(&mut model.messages),
                        Resource::Launch(_) | Resource::LaunchVariant(_) => Some(&mut model.launches),
                        Resource::Arrangement(row) => { arrangements.push(row.clone()); None },
                        _ => None,
                    };
                    if let Some(collection) = collection { collection.items.push(*resource); }
                }
                WireValue::TimelineEntry(entry) => {
                    let index = item.pointer.split('/').nth(3).context("missing thread index")?.parse::<usize>()?;
                    let agent = slice.state["threads"][index]["agent"].as_str().context("missing thread agent")?;
                    conversations.entry(agent.into()).or_default().push(entry);
                }
                WireValue::TerminalScreen(screen) => {
                    let index = item.pointer.split('/').nth(3).context("missing terminal index")?.parse::<usize>()?;
                    let owner = slice.state["terminals"][index]["owner"].as_str().context("missing terminal owner")?;
                    terminals.insert(owner.into(), screen);
                }
                _ => {}
            }
        }
    }
    let person = "person/robin";
    let names = adapt::names(&model, person);
    for (agent, entries) in conversations {
        extras.conversations.insert(agent, Load::Ready(adapt::conversation(&entries, &names)));
    }
    let mut world = adapt::world_at(&model, person, &extras, &snapshot.created_at);
    world.worktrees = Load::Failed("The scenario kit does not model worktrees".into());
    Ok(DemoWorld { world, terminals, arrangements })
}

/// Preview-only PR 17 fixture. Memberships are a complete invented server result,
/// not a substitute for the still-unwired live collection-v1 reader.
pub(super) fn sidebar_v2(world: &mut World) -> Result<(fractal_client::Arrangement, Vec<fractal_sidebar::model::MembershipInput>)> {
    use fractal_sidebar::model::MembershipInput;
    use serde_json::json;
    let subject = format!("arrangement/person/robin/{}", fractal_client::SIDEBAR_ARRANGEMENT_UUID);
    let folder_ids: Vec<String> = (0..5).map(|i| format!("00000000-0000-7000-8000-{i:012}")).collect();
    let names = ["Product", "Sidebar v2", "Delivery", "Operations", "Research"];
    let mut folders = serde_json::Map::new();
    let mut parents = BTreeMap::new();
    for (i, id) in folder_ids.iter().enumerate() {
        let parent = match i { 1 | 2 => Some(folder_ids[0].clone()), _ => None };
        folders.insert(id.clone(), json!({
            "name": {"value": names[i], "revision": "preview-1"},
            "position": {"value": {"parent": parent, "key": format!("a{i}")}, "revision": "preview-1"},
            "tombstone": null
        }));
        parents.insert(id.clone(), parent);
    }
    let arrangement = serde_json::from_value(json!({
        "id": subject, "revision": "preview-1", "updated_at": "2026-11-07T12:00:00Z",
        "owner": "person/robin", "deleted": false,
        "body": {"version": 2, "name": {"value": "Sidebar", "revision": "preview-1"}, "folders": folders},
        "resolved": {"parents": parents, "folders": {}}
    })).context("Invalid v2 preview arrangement")?;
    let agent_template = world.agents.items().first().context("Demo has no agent template")?.clone();
    let mission_template = world.missions.items().first().context("Demo has no mission template")?.clone();
    let mut agents = world.agents.items().to_vec();
    let mut missions = world.missions.items().to_vec();
    let mut members = Vec::new();
    let tasks = [
        "Projection", "Ordered placements", "Folder nesting", "Keyboard navigation",
        "Retirement", "Membership paging", "Sidebar review", "Write journal",
        "Terminal handoff", "Delivery audit", "Release plan", "Device pairing",
        "Fleet inventory", "Observability", "Research notes",
    ];
    for i in 0..30 {
        let member = if i % 3 == 0 {
            let mut row = mission_template.clone();
            row.id = format!("mission/preview/sidebar-{i:02}");
            row.title = format!("{} mission", tasks[i / 2]);
            let id = row.id.clone();
            missions.push(row);
            id
        } else {
            let mut row = agent_template.clone();
            row.id = format!("agent/preview/sidebar-{i:02}");
            row.name = format!("{} agent", tasks[i / 2]);
            row.mission = None;
            row.parent = None;
            row.terminal = false;
            row.subagents.clear();
            row.activity = "Invented sidebar preview".into();
            row.state = if i == 29 { AgentState::Retired } else if i % 2 == 0 { AgentState::Working } else { AgentState::Idle };
            let id = row.id.clone();
            agents.push(row);
            id
        };
        // Retired members are absent from the complete membership set while the
        // catalog still contains them. The production projection must hide this row.
        if i == 29 { continue; }
        members.push(MembershipInput {
            container: subject.clone(), member, revision: "preview-1".into(),
            position: fractal_schema::ordered_membership::Position {
                bucket: Some(folder_ids[match i { 0..=8 => 1, 9..=14 => 2, 15..=23 => 3, _ => 4 }].clone()),
                key: format!("a{}", char::from(b'A' + i as u8)),
            },
        });
    }
    world.agents = Load::Ready(agents);
    world.missions = Load::Ready(missions);
    Ok((arrangement, members))
}

/// Product callers use `load` to report errors before entering terminal raw mode.
#[cfg(test)]
pub fn world() -> World {
    load().unwrap_or_else(|error| panic!("Cannot open scenario demo: {error:#}")).world
}

#[cfg(test)]
pub fn loading() -> World {
    loading_from(world())
}

pub(super) fn loading_from(mut world: World) -> World {
    world.link = Link::Connecting;
    world.attention = Load::Loading;
    world.agents = Load::Loading;
    world.missions = Load::Loading;
    world.machines = Load::Loading;
    world.devices = Load::Loading;
    world.worktrees = Load::Loading;
    world.usage = Load::Loading;
    world.clients = Load::Loading;
    world.slo = Load::Loading;
    world.conversations.clear();
    world
}

pub fn terminal(owner: &str) -> Vec<ratatui::text::Line<'static>> {
    match load().unwrap_or_else(|error| panic!("Cannot open scenario demo: {error:#}")).terminals.remove(owner) {
        Some(screen) => screen.lines.into_iter().map(|line| ratatui::text::Line::raw(line.text)).collect(),
        None => vec![ratatui::text::Line::raw("This scenario has no recorded terminal for this subject.")],
    }
}

#[cfg(test)]
pub(super) fn contract_world() -> World {
    let now = chrono::DateTime::parse_from_rfc3339(scenarios::ANCHOR).unwrap().timestamp_millis();
    let mut world = load_at(SLICES, now).unwrap().world;
    // Local clock labels are presentation, not shared data (as in conversation_of).
    for conversation in world.conversations.values_mut() {
        if let Load::Ready(entries) = conversation {
            for entry in entries { entry.at.clear(); }
        }
    }
    world
}

/// Cover every input, including inventories without a standalone TUI screen yet.
#[cfg(test)]
pub(super) fn contract_inputs() -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let hashes: BTreeMap<_, _> = SLICES.iter().map(|(name, json)| {
        (*name, format!("{:x}", Sha256::digest(json.as_bytes())))
    }).collect();
    serde_json::json!({ "world": "fleet-mid-refactor", "sha256": hashes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_v2_preview_has_nested_ordered_members_and_hides_retired() {
        let mut world = load().unwrap().world;
        let (arrangement, members) = sidebar_v2(&mut world).unwrap();
        assert!(matches!(arrangement.body, fractal_client::ArrangementBody::V2(_)));
        assert_eq!(members.len(), 29);
        assert!(members.windows(2).all(|pair| pair[0].position.key < pair[1].position.key));
        assert_eq!(arrangement.resolved.as_ref().unwrap().parents.values().filter(|parent| parent.is_some()).count(), 2);
        let retired = world.agents.items().iter().find(|agent| agent.id == "agent/preview/sidebar-29").unwrap();
        assert_eq!(retired.state, AgentState::Retired);
        assert!(!members.iter().any(|member| member.member == retired.id));
    }

    #[test]
    fn demo_rebases_inventory_instants_but_keeps_relative_ages() {
        let first = load_at(SLICES, 1_700_000_000_000).unwrap().world;
        let later = load_at(SLICES, 1_700_000_060_000).unwrap().world;
        let expires = |world: &World| chrono::DateTime::parse_from_rfc3339(&world.devices.items()[0].expires)
            .unwrap().timestamp_millis();
        assert_eq!(expires(&later) - expires(&first), 60_000);
        assert_eq!(first.agents.items()[0].activity, later.agents.items()[0].activity);
    }

    #[test]
    fn demo_rejects_a_strict_decode_failure_before_adaptation() {
        let mut corrupted: serde_json::Value = serde_json::from_str(SLICES[0].1).unwrap();
        corrupted["state"]["agents"][0]["state"] = "not-a-client-v0-state".into();
        let json = serde_json::to_string(&corrupted).unwrap();
        let error = load_at(&[("roster", &json)], 0).err().expect("strict decode must fail");
        let message = format!("{error:#}");
        assert!(message.contains("fleet-mid-refactor/roster.json strict decode failed"), "{message}");
        assert!(message.contains("/state/agents/0"), "{message}");
    }

    #[test]
    fn demo_initial_state_uses_the_generated_roster_and_recorded_terminal() {
        let demo = load().unwrap();
        assert_eq!(demo.world.agents.items().len(), 4);
        assert_eq!(demo.world.devices.items().len(), 2);
        assert!(matches!(&demo.world.worktrees, Load::Failed(message)
            if message == "The scenario kit does not model worktrees"));
        assert_eq!(demo.world.missions.items().len(), 1);
        let mission = &demo.world.missions.items()[0];
        assert_eq!(mission.word, Word::Stalled);
        assert_eq!(mission.agents.len(), 3);
        let steps: Vec<_> = mission.steps.iter().map(|step| {
            (step.name.as_str(), step.state, step.owner.as_deref())
        }).collect();
        assert_eq!(steps, [
            ("step-1", StepState::Done, Some("Atlas Builder")),
            ("step-2", StepState::Working, Some("Atlas Migrator")),
            ("step-5", StepState::Ready, Some("Atlas Migrator")),
            ("step-3", StepState::Waiting, Some("Atlas Builder")),
            ("step-4", StepState::Pending, Some("Atlas Reviewer")),
        ]);
        let conversation = demo.world.conversations["agent/example/atlas/builder"].ready().unwrap();
        assert!(conversation.iter().any(|entry| matches!(&entry.body, Body::User(text) if text.contains("worker typecheck"))));
        assert!(conversation.iter().any(|entry| matches!(&entry.body, Body::Assistant(text) if text.contains("asked Robin"))));
        assert!(!conversation.iter().any(|entry| matches!(&entry.body, Body::Mail { from, .. } if from == "Fractal")));
        // Like the live TUI, Now shows the selected person's attention, not Ada's.
        assert_eq!(demo.world.attention.items().len(), 1);
        assert_eq!(demo.world.conversations.len(), 2);
        let screen = &demo.terminals["agent/example/atlas/builder"];
        assert!(screen.lines.iter().any(|line| line.text.contains("typecheck")));
    }
}

