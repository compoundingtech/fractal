use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn append(store: &Store, subject: &str, kind: &str, actor: Option<&str>, fields: Value) {
    store.connection.batched(|transaction| -> anyhow::Result<()> {
        crate::store::append_claim_tx(transaction, store.origin(), subject, kind, actor,
            &json!({"fields": fields}), &[], None)?;
        Ok(())
    }).unwrap().unwrap();
}

fn running(store: &Store, subject: &str) {
    append(store, subject, "runtime.observed", None,
        json!({"status":"running", "runtime_id":subject, "incarnation_id":"one"}));
}

fn short_attempt(store: Arc<Store>) -> smallclaims::read_budget::ReadBudget {
    let pinned = AtomicBool::new(false);
    let snapshots = AtomicU64::new(0);
    let start = Instant::now();
    smallclaims::read_budget::ReadBudget::with_clock_for_test(
        "roster/queue-resume-test", Duration::from_secs(2), Arc::new(move || {
            if store.readers.has_pinned_reader() {
                pinned.store(true, Ordering::SeqCst);
            } else if pinned.swap(false, Ordering::SeqCst) {
                snapshots.fetch_add(1, Ordering::SeqCst);
            }
            start + Duration::from_secs(snapshots.load(Ordering::SeqCst))
        }),
    )
}

fn assert_uncached(store: &Store, history: bool) {
    let (cut, rows, _) = store.published_agent_roster(store.index().unwrap(), history).unwrap();
    assert_eq!(cut, store.index().unwrap());
    let mut expected = store.read_snapshot(|index| {
        let mut rows = client_agent_resources_uncached(store, history, index)?;
        add_agent_todos(store, &mut rows, index)?;
        Ok(rows)
    }).unwrap();
    expected.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str())
        .then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    assert_eq!(*rows, expected);
}

#[test]
fn queue_activity_preserves_completed_membership_pages() {
    for history in [false, true] {
        let store = Arc::new(Store::open_memory("test").unwrap());
        let _wake = store.start_agent_roster_refresher().unwrap();
        running(&store, "agent/base");
        refresh_agent_roster(&store, history).unwrap();
        let previous = store.published_agent_roster(store.index().unwrap(), history).unwrap();
        for n in 0..6 { running(&store, &format!("agent/test-{n:02}")); }
        start(&store, history);
        // Exercise the small page widths reached by adaptive deadline handling.
        let assemblies = store.extension::<Assemblies>();
        assemblies.0.lock().unwrap()[usize::from(history)].as_mut().unwrap().width = 2;
        let mut completed = false;
        let mut progress = 0;
        for attempt in 0..64 {
            let (scanning, cursor, unchanged) = {
                let states = assemblies.0.lock().unwrap();
                let state = states[usize::from(history)].as_ref().unwrap();
                (state.scanning, state.cursor.clone(), state.rows.iter()
                    .filter(|(name, _)| name.as_str() != "agent/test-00")
                    .map(|(name, row)| (name.clone(), row.clone())).collect::<BTreeMap<_, _>>())
            };
            let budget = short_attempt(Arc::clone(&store));
            let result = smallclaims::read_budget::with(Some(budget), || refresh_agent_roster(&store, history));
            assert!(!store.readers.has_pinned_reader());
            if result.is_ok() { completed = true; break; }
            let retained = store.published_agent_roster(store.index().unwrap(), history).unwrap();
            assert!(Arc::ptr_eq(&retained.1, &previous.1));
            {
                let states = assemblies.0.lock().unwrap();
                let state = states[usize::from(history)].as_ref().unwrap();
                assert!(state.cursor >= cursor, "membership must not restart after queue activity (attempt {attempt})");
                for (name, row) in unchanged {
                    assert_eq!(state.rows.get(&name), Some(&row), "unchanged completed rows must survive reconciliation");
                }
            }
            if scanning {
                // Ordinary work activity lands between every membership-page attempt.
                append(&store, "step-run/example/work", "work.progress", Some("agent/test-00"),
                    json!({"summary":format!("page {attempt}")}));
                progress += 1;
            }
        }
        assert!(progress >= 2, "queue churn must overlap several membership pages");
        assert!(completed, "queue activity must not perpetually restart membership");
        assert_uncached(&store, history);
    }
}

#[test]
fn expired_delta_classification_narrows_and_advances_cursor() {
    for history in [false, true] {
        let store = Arc::new(Store::open_memory("test").unwrap());
        let _wake = store.start_agent_roster_refresher().unwrap();
        running(&store, "agent/test");
        running(&store, "agent/unchanged");
        refresh_agent_roster(&store, history).unwrap();
        let mut state = Assembly::new(store.fractal.presented_agents.generation());
        while state.scanning { state.step(&store, history).unwrap(); }
        let floor = state.floor;
        // Each exec claim exercises the classifier's previous-card dependency matching.
        for n in 0..128 { running(&store, &format!("exec/other/child-{n:03}")); }
        running(&store, "exec/test/late-child");
        running(&store, "agent/late-member");
        state.step(&store, history).unwrap(); // Capture the fixed reconciliation target.
        let mut expiries = 0;
        let mut advanced = false;
        let mut completed = false;
        for _ in 0..128 {
            let delta = state.reconcile.as_ref().filter(|page| page.after < page.target)
                .map(|page| page.after);
            let old_width = state.delta_width;
            let result = if delta.is_some() {
                let clock_store = Arc::clone(&store);
                let checks = AtomicU64::new(0);
                let start = Instant::now();
                // Both index queries fit well below 64 progress callbacks. The larger
                // page expires in changed_agent_resources' per-row Rust checks; each
                // retry gets the same allowance, not an extended deadline.
                let budget = smallclaims::read_budget::ReadBudget::with_clock_for_test(
                    "roster/delta-resume-test", Duration::from_millis(64), Arc::new(move || {
                        if clock_store.readers.has_pinned_reader() { checks.fetch_add(1, Ordering::SeqCst); }
                        start + Duration::from_millis(checks.load(Ordering::SeqCst))
                    }),
                );
                smallclaims::read_budget::with(Some(budget), || state.step(&store, history))
            } else { state.step(&store, history) };
            assert!(!store.readers.has_pinned_reader());
            match result {
                Err(error) => {
                    assert!(delta.is_some(), "only the intentionally budgeted delta may expire: {error:#}");
                    assert_eq!(state.reconcile.as_ref().unwrap().after, delta.unwrap(),
                        "failed classifications must not commit a truncated delta");
                    assert!(state.delta_width < old_width, "the failing delta interval must narrow");
                    assert_eq!(state.width, AGENT_ROSTER_WARM_CHUNK, "delta expiry must not shrink unrelated card pages");
                    expiries += 1;
                }
                Ok(Some(_)) => { completed = true; break; }
                Ok(None) => {
                    advanced |= state.reconcile.as_ref().is_some_and(|page| page.after > floor);
                }
            }
        }
        assert!(expiries > 0 && advanced && completed, "narrowed pages must eventually consume the fixed delta");
        let assemblies = store.extension::<Assemblies>();
        assemblies.0.lock().unwrap()[usize::from(history)] = Some(state);
        refresh_agent_roster(&store, history).unwrap();
        assert_uncached(&store, history);
    }
}
