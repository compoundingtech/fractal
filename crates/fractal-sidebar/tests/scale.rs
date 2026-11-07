//! Opt-in timings follow seat_queue_perf: --release --test scale -- --ignored --nocapture.
use fractal_sidebar::{edits::{Base, Edits, Effect, Event, Identity}, model::*,
    paging::{Scope, Snapshot}, plan::{MoveIntent, V2Refusal, plan_move, plan_v2},
    projection::{build, build_v2}, widget::{self, Kind, Presentation, Row, Styles}};
use ratatui::{Terminal, backend::TestBackend, style::Style};
use serde_json::json;
use std::{hint::black_box, time::{Duration, Instant}};

fn fixture(count: usize, depth: usize) -> (ArrangementInput, Vec<MembershipInput>) {
    let ids: Vec<_> = (0..depth).map(|i| format!("00000000-0000-7000-8000-{i:012x}")).collect();
    let mut folders = serde_json::Map::new();
    let mut parents = std::collections::BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        let parent = i.checked_sub(1).map(|p| ids[p].clone());
        folders.insert(id.clone(), json!({"name":{"value":"Folder","revision":"r"},
            "position":{"value":{"parent":parent,"key":"a0"},"revision":"r"},"tombstone":null}));
        parents.insert(id.clone(), parent);
    }
    let a = ArrangementInput { subject: "arrangement/person/test/00000000-0000-7000-8000-000000010000".into(),
        owner: "person/test".into(), revision: "r".into(), deleted: false,
        body: serde_json::from_value(json!({"version":2,"name":{"value":"Sidebar","revision":"r"},"folders":folders})).unwrap(),
        resolved: Some(ResolvedInput { parents, folders: Default::default() }) };
    let members = (0..count).map(|i| MembershipInput { container: a.subject.clone(), member: format!("agent/test/{i:05}"),
        revision: "r".into(), position: fractal_schema::ordered_membership::Position { bucket: ids.last().cloned(), key: "a0".into() } }).collect();
    (a, members)
}
fn intent(members: &[MembershipInput]) -> MoveIntent {
    MoveIntent { node: NodeRef::Subject { subject: members.last().unwrap().member.clone() },
        parent: members[0].position.bucket.clone().map_or(Parent::Root {}, |id| Parent::Folder { id }),
        before: Some(NodeRef::Subject { subject: members[0].member.clone() }) }
}
fn terminal() -> Terminal<TestBackend> { Terminal::new(TestBackend::new(38, 24)).unwrap() }
fn draw(terminal: &mut Terminal<TestBackend>, tree: &SidebarTree, selected: usize) {
    // Flatten once, borrowing node identities/labels; never scan the catalog per visible row.
    let mut stack: Vec<_> = tree.roots.iter().rev().collect();
    let mut rows = Vec::with_capacity(tree.nodes.len());
    while let Some(node) = stack.pop() {
        let key = node_key(node);
        let (id, row) = tree.nodes.get_key_value(&key).unwrap();
        let label = match node { NodeRef::Subject { subject } => subject, NodeRef::Folder { id } => id };
        rows.push(Row { id, label, kind: match node { NodeRef::Folder { .. } => Kind::Folder, _ => Kind::Resource { available: true } } });
        stack.extend(row.children.iter().rev());
    }
    let styles = Styles { background: Style::default(), dim: Style::default(), label: Style::default(),
        text: Style::default(), accent: Style::default(), selected: Style::default() };
    terminal.draw(|frame| { widget::render(frame.buffer_mut(), ratatui::layout::Rect::new(0, 0, 38, 24),
        &rows, &Presentation { filter: "", selected, focused: true }, &styles); }).unwrap();
}
#[test]
fn scale_ties_deep_nesting_and_next_frame_overlay() {
    for (count, depth) in [(1001, 1), (4096, 1), (1001, fractal_schema::arrangements::MAX_FOLDERS)] {
        let (a, members) = fixture(count, depth);
        let tree = build_v2(&a, &members).unwrap();
        assert_eq!(tree.nodes.len(), count + depth);
        let mut reversed = members.clone(); reversed.reverse();
        assert_eq!(build_v2(&a, &reversed).unwrap(), tree);
        let scope = Scope { owner: a.owner.clone(), container: a.subject.clone(), endpoint: "local".into(), host: "host".into(), projection: "1".into() };
        let base = Base { scope: scope.clone(), revision: a.revision.clone(), ready: true,
            snapshot: Snapshot { id: "snapshot".into(), host_id: "host".into(), store_index: 1, projection_version: "1".into(), created_at: "now".into(), published_at: None } };
        let identity = |n| Identity { actor: a.owner.clone(), action_id: format!("action-{n}"), idempotency_key: format!("key-{n}") };
        let move_intent = intent(&members);
        let mut edits = Edits::new(scope, move_intent.clone());
        assert_eq!(edits.reduce(Event::Plan { base, plan: plan_v2(&a, &members, &move_intent).unwrap(), folder: identity(1), membership: identity(2) }), vec![Effect::PersistPending]);
        let mut terminal = terminal(); draw(&mut terminal, &tree, depth);
        let pending = edits.project(&tree).unwrap();
        draw(&mut terminal, &pending, depth); // Exactly the next frame, before persistence or dispatch.
        let parent = &pending.nodes[&node_key(&members_node(&members[0]))].shown_under;
        let Parent::Folder { id } = parent else { panic!("folder") };
        assert_eq!(pending.nodes[&format!("folder:{id}")].children[0], move_intent.node);
        let cells: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(cells.contains(&members.last().unwrap().member), "{cells}");
        assert_eq!(tree.nodes[&format!("folder:{id}")].children[0], members_node(&members[0]));
    }
}
fn members_node(member: &MembershipInput) -> NodeRef { NodeRef::Subject { subject: member.member.clone() } }
#[test]
fn atomic_rekey_and_mebibyte_refuse_before_any_dispatch() {
    let (a, mut members) = fixture(2049, 0);
    members.last_mut().unwrap().position.key = "b00".into();
    let mut movement = intent(&members); movement.before = Some(members_node(&members[1024]));
    assert_eq!(plan_v2(&a, &members, &movement), Err(V2Refusal::TooManyOperations { count: 1025 }));
    let (a, mut members) = fixture(3, 0);
    members[2].member.push_str(&"x".repeat(fractal_schema::ordered_membership::MAX_EDIT_BYTES));
    assert!(matches!(plan_v2(&a, &members, &intent(&members)), Err(V2Refusal::TooManyBytes { bytes }) if bytes > 1024 * 1024));
    // No plan exists to hand to Edits::reduce, hence no Send/persistence/partial batch.
}
#[test]
fn complete_scale_paging_ignores_unrelated_indexes_and_keeps_stale_base_on_loss() {
    use fractal_sidebar::paging::{self, Frontier, Page, Paging, Status};
    let (a, members) = fixture(4096, 0);
    let scope = Scope { owner: a.owner, container: a.subject, endpoint: "local".into(), host: "host".into(), projection: "1".into() };
    let mut state = Paging::new(None);
    let effects = state.reduce(paging::Event::Refresh(scope.clone(), 1));
    let [paging::Effect::SubscribeAndRead(token)] = effects.as_slice() else { panic!("first page") };
    let token = token.clone();
    for (index, chunk) in members.chunks(200).enumerate() {
        let more = (index + 1) * 200 < members.len();
        let effects = state.reduce(paging::Event::Page(token.clone(), Page {
            frontier: Frontier { epoch: "epoch".into(), changed_index: 1, live_count: 4096 },
            // Unrelated writes advance every page's snapshot, never the collection frontier.
            snapshot: Snapshot { id: format!("snapshot-{index}"), host_id: "host".into(), store_index: 100 + index as u64,
                projection_version: "1".into(), created_at: "now".into(), published_at: None },
            members: chunk.to_vec(), has_more: more, next: more.then(|| format!("cursor-{index}")),
        }));
        if more { assert!(matches!(effects.as_slice(), [paging::Effect::Read(_, _)])); assert!(state.complete().is_none()); }
        else { assert_eq!(effects, vec![paging::Effect::PersistBase]); }
    }
    assert_eq!(state.status, Status::Ready);
    let complete = state.complete().unwrap().clone(); assert_eq!(complete.members, members);
    state.reduce(paging::Event::Refresh(scope, 2));
    assert!(state.stale()); assert_eq!(state.complete(), Some(&complete));
    state.reduce(paging::Event::Disconnect);
    assert_eq!(state.status, Status::Disconnected); assert_eq!(state.complete(), Some(&complete));
    assert!(state.reduce(paging::Event::Expired(token)).is_empty());
    assert_eq!(state.status, Status::Disconnected);
}
fn p95(mut samples: Vec<Duration>) -> Duration { samples.sort_unstable(); samples[(samples.len() * 95).div_ceil(100) - 1] }
#[allow(clippy::assertions_on_constants, reason = "ignored latency gate must compile in debug but reject debug measurements at runtime")]
#[test]
#[ignore = "explicit release latency gate; run on a stated idle runner with --nocapture"]
fn latency_projection_planning_render_and_same_data_v1_baseline() {
    assert!(!cfg!(debug_assertions), "measure --release only");
    println!("runner={} arch={} os={} viewport=38x24 samples=100 warmup=10", std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()), std::env::consts::ARCH, std::env::consts::OS);
    let mut failures = Vec::new();
    for count in [1001, 4096] {
        let (a, members) = fixture(count, 1); let movement = intent(&members);
        let catalog: Vec<_> = members.iter().map(|m| m.member.clone()).collect();
        let mut v1 = a.clone(); let mut body = serde_json::to_value(&a.body).unwrap(); body["version"] = json!(1);
        body["placements"] = json!(members.iter().map(|m| (m.member.clone(), json!({"value":{"folder":m.position.bucket,"key":m.position.key},"revision":"r"}))).collect::<std::collections::BTreeMap<_, _>>());
        v1.body = serde_json::from_value(body).unwrap();
        v1.resolved.as_mut().unwrap().folders = members.iter().map(|m| (m.member.clone(), m.position.bucket.clone())).collect();
        let mut results = Vec::new();
        for version in [1, 2] {
            let mut work = Vec::new(); let mut render = Vec::new(); let mut terminal = terminal();
            for round in 0..110 {
                let start = Instant::now();
                let tree = if version == 1 { let tree = build(&v1, &catalog).unwrap(); black_box(plan_move(&tree, &movement).unwrap()); tree }
                    else { let tree = build_v2(&a, &members).unwrap(); black_box(plan_v2(&a, &members, &movement).unwrap()); tree };
                let elapsed = start.elapsed(); let start = Instant::now(); draw(&mut terminal, black_box(&tree), 0);
                if round >= 10 { work.push(elapsed); render.push(start.elapsed()); }
            }
            let pair = (p95(work), p95(render));
            println!("members={count} v{version} projection_planning_p95_ms={:.3} render_p95_ms={:.3}", pair.0.as_secs_f64()*1000.0, pair.1.as_secs_f64()*1000.0);
            results.push(pair);
        }
        for (passed, gate) in [(results[1].0 <= Duration::from_millis(4), "4ms projection/planning"),
            (results[1].1 <= Duration::from_millis(8), "8ms render"),
            (results[1].0 <= results[0].0 * 2, "2x v1 projection/planning"),
            (results[1].1 <= results[0].1 * 2, "2x v1 render")] {
            if !passed { failures.push(format!("{count} members: {gate}")); }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
