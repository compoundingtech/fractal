//! Keyed contributions to the shared summary. Publication identity is not a count input.
use super::*;

pub(super) fn shared(
    state: &AppState,
    generation: u64,
    session: &ClientSession,
    snapshot: &ClientSnapshot,
    now: u128,
) -> anyhow::Result<SummaryShared> {
    let store = &state.store;
    let publication = store.agents_publication();
    let mut native;
    let (agents, agents_as_of) = if store.agent_roster_refresher_running() {
        let Some(publication) = publication.as_ref() else {
            store.request_agent_roster_refresh();
            return Err(RosterNotPublished.into());
        };
        let watermark = publication.metadata().status_watermark;
        (publication.rows(), Some(json!({
            "store_index": watermark.store_index,
            "published_at": client_timestamp(u128::from(publication.metadata().materialized_at_ms)),
        })))
    } else {
        native = client_agent_resources_cached(store, false, snapshot.store_index)?;
        overlay_agent_resources(store, &mut native, &client_timestamp(now))?;
        (native.as_slice(), None)
    };
    let active_missions = store.client_summary_counts(now, snapshot.store_index, active)?;
    let machines = published_machines(state, generation, session, snapshot, now)?;
    let peer_deadline = store.summary_machines().and_then(|m| m.valid_until);
    let gateway = client_host_id(&state.node);
    store.with_summary_sources(|sources| {
        maintain(sources, agents, &machines, &gateway, now, peer_deadline);
        Ok(SummaryShared {
            cut: snapshot.store_index,
            evaluated_at_unix_ms: now,
            published_at_unix_ms: client_now_ms(),
            active_missions,
            working_agents: sources.working_agents,
            machines: json!({"connected": sources.machine_counts[0],
                "indirect": sources.machine_counts[1], "offline": sources.machine_counts[2]}),
            agents_as_of,
        })
    })
}

pub(super) fn alerts(store: &Store, rows: &[Value], people: &[Option<String>]) -> anyhow::Result<Vec<usize>> {
    store.with_summary_sources(|sources| {
        maintain_alerts(sources, rows);
        Ok(people.iter().map(|person| match person {
            None => sources.total_alerts,
            Some(person) => sources.person_alerts.get(person).copied().unwrap_or(0),
        }).collect())
    })
}

fn maintain_alerts(sources: &mut crate::store::summary_list::SummarySources, rows: &[Value]) {
    let ids = rows.iter().filter_map(|row| row["id"].as_str()).collect::<BTreeSet<_>>();
    sources.attention.retain(|id, (person, alert)| {
        if ids.contains(id.as_str()) { return true; }
        if *alert {
            sources.total_alerts -= 1;
            if let Some(person) = person {
                *sources.person_alerts.get_mut(person).expect("counted alert person") -= 1;
            }
        }
        sources.work.attention_subject_rebuilds += 1;
        false
    });
    for row in rows {
        let Some(id) = row["id"].as_str() else { continue };
        let person = row["person_id"].as_str();
        let alert = row["alert"] == true;
        if sources.attention.get(id).is_some_and(|(held_person, held_alert)|
            held_person.as_deref() == person && *held_alert == alert) {
            continue;
        }
        if let Some((previous_person, true)) = sources.attention.insert(id.to_owned(), (person.map(str::to_owned), alert)) {
            sources.total_alerts -= 1;
            if let Some(person) = previous_person {
                *sources.person_alerts.get_mut(&person).expect("counted alert person") -= 1;
            }
        }
        if alert {
            sources.total_alerts += 1;
            if let Some(person) = person {
                *sources.person_alerts.entry(person.to_owned()).or_default() += 1;
            }
        }
        sources.work.attention_subject_rebuilds += 1;
    }
    sources.person_alerts.retain(|_, count| *count != 0);
}

fn maintain(
    sources: &mut crate::store::summary_list::SummarySources,
    agents: &[Value],
    machines: &[Value],
    gateway: &str,
    now: u128,
    peer_deadline: Option<u128>,
) {
    let ids = agents.iter().filter_map(|a| a["id"].as_str()).collect::<BTreeSet<_>>();
    sources.agents.retain(|id, was_working| {
        if ids.contains(id.as_str()) { return true; }
        sources.working_agents -= usize::from(*was_working);
        sources.work.agent_subject_rebuilds += 1;
        false
    });
    let mut activity = BTreeMap::<&str, u128>::new();
    for agent in agents {
        let Some(id) = agent["id"].as_str() else { continue };
        let is_working = working(agent);
        if sources.agents.get(id) != Some(&is_working) {
            if let Some(was_working) = sources.agents.insert(id.to_owned(), is_working) {
                sources.working_agents -= usize::from(was_working);
            }
            sources.working_agents += usize::from(is_working);
            sources.work.agent_subject_rebuilds += 1;
        }
        if let (Some(host), Some(at)) = (agent["host_id"].as_str(), timestamp(&agent["last_activity_at"])) {
            activity.entry(host).and_modify(|held| *held = (*held).max(at)).or_insert(at);
        }
    }
    let hosts = machines.iter().filter_map(|m| m["host_id"].as_str()).collect::<BTreeSet<_>>();
    sources.machines.retain(|host, (bucket, _)| {
        if hosts.contains(host.as_str()) { return true; }
        sources.machine_counts[*bucket] -= 1;
        sources.work.machine_subject_rebuilds += 1;
        false
    });
    for machine in machines {
        let host = machine["host_id"].as_str().unwrap_or_default();
        let direct = host == gateway || matches!(machine["state"].as_str(), Some("local" | "reachable"));
        let until = if direct { None } else {
            machine["transports"].as_array().into_iter().flatten()
                .filter_map(|t| timestamp(&t["last_success_at"]))
                .chain(activity.get(host).copied()).max()
                .map(|at| at.saturating_add(300_000)).filter(|until| *until > now)
        };
        let bucket = if direct { 0 } else if until.is_some() { 1 } else { 2 };
        if sources.machines.get(host) != Some(&(bucket, until)) {
            if let Some((previous, _)) = sources.machines.insert(host.to_owned(), (bucket, until)) {
                sources.machine_counts[previous] -= 1;
            }
            sources.machine_counts[bucket] += 1;
            sources.work.machine_subject_rebuilds += 1;
        }
    }
    sources.valid_until = sources.missions.values().filter_map(|m| m.valid_until)
        .chain(sources.machines.values().filter_map(|(_, until)| *until))
        .chain(peer_deadline).min();
}

fn timestamp(value: &Value) -> Option<u128> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()
        .and_then(|at| u128::try_from(at.timestamp_millis()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::summary_list::SummarySources;

    #[test]
    fn unchanged_and_unrelated_rows_rebuild_no_contributions() {
        let mut sources = SummarySources::default();
        let mut agents = vec![
            json!({"id":"agent/alder","state":"running","harness_state":"working","fault":null}),
            json!({"id":"agent/birch","state":"running","harness_state":"working","fault":null}),
        ];
        let mut machines = vec![
            json!({"host_id":"host/alder","state":"local"}),
            json!({"host_id":"host/birch","state":"offline"}),
        ];
        maintain(&mut sources, &agents, &machines, "host/alder", 1_000, None);
        assert_eq!(sources.working_agents, 2);
        assert_eq!(sources.machine_counts, [1, 0, 1]);
        let before = sources.work.clone();
        maintain(&mut sources, &agents, &machines, "host/alder", 2_000, None);
        agents[0]["title"] = json!("A different display label");
        machines[0]["revision"] = json!("an unrelated publication revision");
        maintain(&mut sources, &agents, &machines, "host/alder", 3_000, None);
        assert_eq!(sources.work.agent_subject_rebuilds, before.agent_subject_rebuilds);
        assert_eq!(sources.work.machine_subject_rebuilds, before.machine_subject_rebuilds);
        agents[1]["harness_state"] = json!("idle");
        maintain(&mut sources, &agents, &machines, "host/alder", 4_000, None);
        assert_eq!(sources.working_agents, 1);
        assert_eq!(sources.work.agent_subject_rebuilds, before.agent_subject_rebuilds + 1);
        assert_eq!(sources.work.machine_subject_rebuilds, before.machine_subject_rebuilds);
        machines[1]["state"] = json!("reachable");
        maintain(&mut sources, &agents, &machines, "host/alder", 5_000, None);
        assert_eq!(sources.machine_counts, [2, 0, 0]);
        assert_eq!(sources.work.machine_subject_rebuilds, before.machine_subject_rebuilds + 1);
    }

    #[test]
    fn only_the_host_whose_recency_expires_changes_at_its_deadline() {
        let mut sources = SummarySources::default();
        let machines = vec![
            json!({"host_id":"host/alder","state":"unknown","transports":[{"last_success_at":"1970-01-01T00:00:01Z"}]}),
            json!({"host_id":"host/birch","state":"unknown","transports":[{"last_success_at":"1970-01-01T00:00:02Z"}]}),
        ];
        maintain(&mut sources, &[], &machines, "host/gateway", 300_000, None);
        assert_eq!(sources.valid_until, Some(301_000));
        assert_eq!(sources.machine_counts, [0, 2, 0]);
        let before = sources.work.machine_subject_rebuilds;
        maintain(&mut sources, &[], &machines, "host/gateway", 300_999, None);
        assert_eq!(sources.work.machine_subject_rebuilds, before);
        maintain(&mut sources, &[], &machines, "host/gateway", 301_000, None);
        assert_eq!(sources.work.machine_subject_rebuilds, before + 1);
        assert_eq!(sources.machine_counts, [0, 1, 1]);
        assert_eq!(sources.valid_until, Some(302_000));
        maintain(&mut sources, &[], &machines, "host/gateway", 302_000, None);
        assert_eq!(sources.work.machine_subject_rebuilds, before + 2);
        assert_eq!(sources.machine_counts, [0, 0, 2]);
        assert_eq!(sources.valid_until, None);
    }

    #[test]
    fn attention_counts_change_only_for_the_changed_subject_and_person() {
        let mut sources = SummarySources::default();
        let mut rows = vec![
            json!({"id":"attention/alder","person_id":"person/avery","alert":true}),
            json!({"id":"attention/birch","person_id":"person/blair","alert":true}),
        ];
        maintain_alerts(&mut sources, &rows);
        assert_eq!(sources.total_alerts, 2);
        let before = sources.work.attention_subject_rebuilds;
        rows[0]["title"] = json!("Unrelated presentation");
        maintain_alerts(&mut sources, &rows);
        assert_eq!(sources.work.attention_subject_rebuilds, before);
        rows[1]["person_id"] = json!("person/avery");
        maintain_alerts(&mut sources, &rows);
        assert_eq!(sources.work.attention_subject_rebuilds, before + 1);
        assert_eq!(sources.total_alerts, 2);
        assert_eq!(sources.person_alerts["person/avery"], 2);
        assert!(!sources.person_alerts.contains_key("person/blair"));
        rows.pop();
        maintain_alerts(&mut sources, &rows);
        assert_eq!(sources.work.attention_subject_rebuilds, before + 2);
        assert_eq!(sources.total_alerts, 1);
        assert_eq!(sources.person_alerts["person/avery"], 1);
    }
}
