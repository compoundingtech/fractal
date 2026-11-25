#![allow(unused_imports)]
use super::*;


#[test]
fn agents_window_partial_projection_supplies_its_queue_deadline() {
    let store = Store::open_memory("node").unwrap();
    let entry = |index, history, covered, deadline| runtime::AgentResourcesEntry {
        index, local: 0, history, covered, valid_until_unix_ms: deadline,
        idle_residency: crate::config::idle_residency_policy(),
        items: Arc::new(Vec::new()), published_at_unix_ms: 0,
    };
    let Ok(mut cache) = store.fractal.agent_resources_cache.lock() else {
        panic!("fixture cache was poisoned before deadline setup");
    };
    cache.push_back(entry(0, false, None, Some(100)));
    cache.push_back(entry(0, false, Some(BTreeSet::from(["agent/node.one".into()])), Some(200)));
    // Newer history rows and another cut must not replace this window's deadline.
    cache.push_back(entry(0, true, None, Some(300)));
    cache.push_back(entry(1, false, None, Some(400)));
    drop(cache);
    assert_eq!(store.agent_roster_valid_until(0, None), Some(200));
    assert_eq!(store.agent_roster_valid_until(1, None), Some(400));
    assert_eq!(store.agent_roster_valid_until(2, None), None);
    let Ok(mut cache) = store.fractal.agent_resources_cache.lock() else {
        panic!("fixture cache was poisoned before clearing the lease deadline");
    };
    cache.push_back(entry(0, false, Some(BTreeSet::new()), None));
    drop(cache);
    assert_eq!(store.agent_roster_valid_until(0, None), None,
        "a current projection with no live leases must not inherit an older expiry");
}

#[test]
fn changed_idle_policy_refolds_roster_and_exact_cards_at_the_same_claim_cut() {
    let store = Store::open_memory("node").unwrap();
    let index = store.index().unwrap();
    let effective = crate::config::idle_residency_policy();
    // Settings are unchanged: only the activation epoch differs after reactivation.
    let previous = crate::config::IdleResidencyPolicy {
        enabled_at_unix_ms: Some(effective.enabled_at_unix_ms.unwrap_or_default() + 1),
        ..effective
    };
    for exact in [false, true] {
        let resources = if exact {
            &store.fractal.exact_agent_resources_cache
        } else {
            &store.fractal.agent_resources_cache
        };
        resources.lock().unwrap().push_back(runtime::AgentResourcesEntry {
            index,
            local: 0,
            history: false,
            idle_residency: previous,
            covered: exact.then(|| BTreeSet::from(["agent/example".into()])),
            valid_until_unix_ms: None,
            items: Arc::new(vec![serde_json::json!({"id":"agent/example", "name":"Example"})]),
            published_at_unix_ms: now_ms(),
        });
        let build = |changed: Option<(&BTreeSet<String>, &[Value])>| {
            if exact {
                let (selected, old) = changed.expect("exact cold fold selects its agent");
                assert_eq!(selected, &BTreeSet::from(["agent/example".into()]));
                assert!(old.is_empty(), "old-policy cards must not be a delta source");
            } else {
                assert!(changed.is_none(), "a policy change must rebuild the entire roster");
            }
            Ok(vec![serde_json::json!({"id":"agent/example", "name":"Example", "policy":"current"})])
        };
        let cards = if exact {
            store.cached_exact_agent_resources(index, false, "agent/example", build).unwrap()
        } else {
            store.cached_agent_resources(index, false, build).unwrap()
        };
        assert_eq!(cards[0]["policy"], "current");
        assert_eq!(store.index().unwrap(), index, "no claim write is needed for a policy refresh");
        assert_eq!(resources.lock().unwrap().back().unwrap().idle_residency, effective);
    }
}
