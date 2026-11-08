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
        assert_eq!(edits.reduce(Event::Plan { base, plan: plan_v2(&tree, &move_intent).unwrap(), folder: identity(1), membership: identity(2) }), vec![Effect::PersistPending]);
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
    assert_eq!(plan_v2(&build_v2(&a, &members).unwrap(), &movement), Err(V2Refusal::TooManyOperations { count: 1025 }));
    let (a, mut members) = fixture(3, 0);
    members[2].member.push_str(&"x".repeat(fractal_schema::ordered_membership::MAX_EDIT_BYTES));
    assert!(matches!(plan_v2(&build_v2(&a, &members).unwrap(), &intent(&members)), Err(V2Refusal::TooManyBytes { bytes }) if bytes > 1024 * 1024));
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
fn revisions(a: &ArrangementInput) -> fractal_sidebar::cache::Revisions {
    fractal_sidebar::cache::Revisions { scope: Scope { owner: a.owner.clone(), container: a.subject.clone(),
        endpoint: "local".into(), host: "host".into(), projection: "1".into() },
        membership: fractal_sidebar::paging::Frontier { epoch: "epoch".into(), changed_index: 1, live_count: 4096 },
        catalog: "catalog-1".into(), presentation: "presentation-1".into() }
}
// Same v1 validation and key-insertion functions as resource_sidebar; normalize positions
// solely to give both baselines the identical pending-tree update and shared painter.
fn v1_plan(tree: &SidebarTree, movement: &MoveIntent) -> fractal_sidebar::plan::V2Plan {
    plan_move(tree, movement).unwrap();
    let siblings = &tree.nodes[&node_key(&NodeRef::Folder { id: match &movement.parent { Parent::Folder { id } => id.clone(), _ => unreachable!() } })].children;
    let keys: Vec<_> = siblings.iter().filter(|node| **node != movement.node)
        .map(|node| tree.nodes[&node_key(node)].key.as_deref().unwrap()).collect();
    let (key, rekeys) = fractal_sidebar::keys::insertion(&keys, 0).unwrap(); assert!(rekeys.is_empty());
    let NodeRef::Subject { subject } = &movement.node else { unreachable!() };
    let Parent::Folder { id } = &movement.parent else { unreachable!() };
    fractal_sidebar::plan::V2Plan { folders: vec![], memberships: vec![fractal_schema::ordered_membership::Operation::Place {
        member: subject.clone(), bucket: Some(id.clone()), key }] }
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
            let arrangement = if version == 1 { &v1 } else { &a }; let revisions = revisions(arrangement);
            let mut cold = Vec::new(); let mut work = Vec::new(); let mut render = Vec::new(); let mut terminal = terminal();
            for _ in 0..100 {
                let start = Instant::now();
                let tree = if version == 1 { build(arrangement, &catalog).unwrap() } else { build_v2(arrangement, &members).unwrap() };
                cold.push(start.elapsed()); black_box(tree);
            }
            cold.sort_unstable();
            println!("members={count} v{version} cold_projection_p50_ms={:.3} cold_projection_p95_ms={:.3}", cold[49].as_secs_f64()*1000.0, p95(cold).as_secs_f64()*1000.0);
            let mut cache = fractal_sidebar::cache::ProjectionCache::default();
            cache.project(&revisions, arrangement, &members, &catalog).unwrap();
            for round in 0..110 {
                let start = Instant::now();
                let tree = cache.project(&revisions, arrangement, &members, &catalog).unwrap();
                let plan = if version == 1 { v1_plan(tree, &movement) } else { plan_v2(tree, &movement).unwrap() };
                let mut edits = Edits::new(revisions.scope.clone(), movement.clone());
                let identity = |n| Identity { actor: a.owner.clone(), action_id: format!("action-{n}"), idempotency_key: format!("key-{n}") };
                edits.reduce(Event::Plan { base: Base { scope: revisions.scope.clone(), revision: a.revision.clone(), ready: true,
                    snapshot: Snapshot { id: "snapshot".into(), host_id: "host".into(), store_index: 1, projection_version: "1".into(), created_at: "now".into(), published_at: None } },
                    plan, folder: identity(1), membership: identity(2) });
                let pending = edits.project(tree).unwrap();
                let elapsed = start.elapsed(); let start = Instant::now(); draw(&mut terminal, black_box(&pending), 0);
                if round >= 10 { work.push(elapsed); render.push(start.elapsed()); }
            }
            let pair = (p95(work), p95(render));
            println!("members={count} v{version} warm_cached_projection_planning_pending_p95_ms={:.3} render_p95_ms={:.3}", pair.0.as_secs_f64()*1000.0, pair.1.as_secs_f64()*1000.0);
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
#[test]
fn projection_cache_keys_every_revision_but_not_unrelated_snapshots() {
    use fractal_sidebar::cache::ProjectionCache;
    let (a, members) = fixture(3, 1); let key = revisions(&a);
    // Poisoned rows expose a rebuild without adding cache counters to the production API.
    let mut poison = members.clone(); poison.push(members[0].clone());
    let mut cache = ProjectionCache::default();
    let expected = cache.project(&key, &a, &members, &[]).unwrap().clone();
    for _snapshot_index in 100..110 { assert_eq!(cache.project(&key, &a, &poison, &[]), Ok(&expected)); }
    for dimension in 0..11 {
        let mut changed = key.clone();
        match dimension {
            0 => changed.scope.owner.push('x'), 1 => changed.scope.container.push('x'),
            2 => changed.scope.endpoint.push('x'), 3 => changed.scope.host.push('x'), 4 => changed.scope.projection.push('x'),
            5 => changed.membership.epoch.push('x'), 6 => changed.membership.changed_index += 1, 7 => changed.membership.live_count += 1,
            8 => changed.catalog.push('x'), 9 => changed.presentation.push('x'), _ => {}
        }
        let mut arrangement = a.clone(); if dimension == 10 { arrangement.revision.push('x'); }
        assert_eq!(cache.project(&changed, &arrangement, &poison, &[]), Err(ProjectionError::InvalidResolution), "dimension {dimension}");
        assert_eq!(cache.project(&key, &a, &members, &[]), Ok(&expected));
    }
    let mut v1 = a.clone(); let mut body = serde_json::to_value(&a.body).unwrap();
    body["version"] = json!(1); body["placements"] = json!({}); v1.body = serde_json::from_value(body).unwrap();
    assert_eq!(cache.project(&key, &v1, &[], &[]).unwrap().nodes.len(), 1, "body version invalidates at the same revision");
    assert_eq!(cache.project(&key, &a, &members, &[]), Ok(&expected), "v2 replaces v1 entry");
    let mut unavailable = a.clone(); unavailable.deleted = true;
    assert_eq!(cache.project(&key, &unavailable, &members, &[]), Err(ProjectionError::ArrangementUnavailable));
    unavailable.deleted = false; unavailable.resolved = None;
    assert_eq!(cache.project(&key, &unavailable, &members, &[]), Err(ProjectionError::NotReady));
}
#[test]
fn cache_rechecks_live_folder_resolution_at_the_same_revision() {
    use fractal_sidebar::cache::ProjectionCache;
    let (a, members) = fixture(3, 2); let key = revisions(&a);
    let ids: Vec<_> = a.resolved.as_ref().unwrap().parents.keys().cloned().collect();
    let mut cache = ProjectionCache::default();
    for missing in &ids {
        cache.project(&key, &a, &members, &[]).unwrap();
        let mut partial = a.clone(); let parents = &mut partial.resolved.as_mut().unwrap().parents;
        parents.remove(missing);
        *parents.values_mut().next().unwrap() = Some("unknown".into());
        assert_eq!(cache.project(&key, &partial, &members, &[]), Err(ProjectionError::NotReady));
        assert_eq!(build_v2(&partial, &members), Err(ProjectionError::NotReady), "full projection owns precedence");
        let mut poison = members.clone(); poison.push(members[0].clone());
        assert_eq!(cache.project(&key, &a, &poison, &[]), Err(ProjectionError::InvalidResolution), "incomplete resolution cleared the entry");
    }
    cache.project(&key, &a, &members, &[]).unwrap();
    let mut changed = a.clone(); changed.resolved.as_mut().unwrap().parents.insert(ids[1].clone(), None);
    assert_eq!(cache.project(&key, &changed, &members, &[]), Ok(&build_v2(&changed, &members).unwrap()), "changed valid parent rebuilds");
    changed.resolved.as_mut().unwrap().parents.insert(ids[0].clone(), Some("unknown".into()));
    assert_eq!(cache.project(&key, &changed, &members, &[]), Err(ProjectionError::InvalidResolution));
    assert_eq!(cache.project(&key, &a, &members, &[]), Ok(&build_v2(&a, &members).unwrap()), "same-revision recovery rebuilds");
}
#[test]
fn cache_rechecks_v1_placement_resolution_at_the_same_revision() {
    use fractal_sidebar::cache::ProjectionCache;
    let (mut a, members) = fixture(3, 2); let key = revisions(&a);
    let catalog: Vec<_> = members.iter().map(|m| m.member.clone()).collect();
    let mut body = serde_json::to_value(&a.body).unwrap(); body["version"] = json!(1);
    body["placements"] = json!(members.iter().map(|m| (m.member.clone(), json!({"value":{"folder":m.position.bucket,"key":m.position.key},"revision":"r"}))).collect::<std::collections::BTreeMap<_, _>>());
    a.body = serde_json::from_value(body).unwrap();
    a.resolved.as_mut().unwrap().folders = members.iter().map(|m| (m.member.clone(), m.position.bucket.clone())).collect();
    let mut cache = ProjectionCache::default();
    let expected = cache.project(&key, &a, &[], &catalog).unwrap().clone();
    for missing in &catalog {
        cache.project(&key, &a, &[], &catalog).unwrap();
        let mut partial = a.clone();
        partial.resolved.as_mut().unwrap().folders.remove(missing);
        *partial.resolved.as_mut().unwrap().parents.values_mut().next().unwrap() = Some("unknown".into());
        assert_eq!(cache.project(&key, &partial, &[], &catalog), Err(ProjectionError::NotReady));
        assert_eq!(build(&partial, &catalog), Err(ProjectionError::NotReady), "full projection owns precedence");
        let mut invalid = a.clone();
        *invalid.resolved.as_mut().unwrap().parents.values_mut().next().unwrap() = Some("unknown".into());
        assert_eq!(cache.project(&key, &invalid, &[], &catalog), Err(ProjectionError::InvalidResolution), "partial resolution cleared the entry");
    }
    cache.project(&key, &a, &[], &catalog).unwrap();
    let mut changed = a.clone();
    changed.resolved.as_mut().unwrap().folders.insert(catalog[0].clone(), None);
    let moved = build(&changed, &catalog).unwrap(); assert_ne!(moved, expected);
    assert_eq!(cache.project(&key, &changed, &[], &catalog), Ok(&moved), "changed destination rebuilds");
    changed.resolved.as_mut().unwrap().folders.insert(catalog[0].clone(), Some("unknown".into()));
    assert_eq!(cache.project(&key, &changed, &[], &catalog), Err(ProjectionError::InvalidResolution));
    assert_eq!(cache.project(&key, &a, &[], &catalog), Ok(&expected), "same-revision recovery rebuilds");
}
