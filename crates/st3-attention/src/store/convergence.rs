#![allow(unused_imports)]
use super::*;
use crate::graph::parse_test_intent;
use st3_store::store::convergence_shared_attention_tests::{Delivery, FLEET, Node, PERSON, Rng, Simulation, env_number, mission_source};


/// The graph tables whose rows differ between two nodes, for a failing run's report.
fn differing_tables(left: &Node, right: &Node) -> Vec<String> {
    let rows = |node: &Node, table: &str, columns: &[&str], order: &str| {
        let connection = node.store().readers.get();
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h9cfc7a59a062 owner=st3_attention::store::convergence TODO=trace-statement-and-value-representation */ &format!(
                "SELECT json_array({}) FROM {table} ORDER BY {order}",
                columns.join(", ")
            ))
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .unwrap()
    };
    GRAPH_DIGEST_TABLES
        .iter()
        .filter_map(|(label, table, columns, order)| {
            let (left_rows, right_rows) = (
                rows(left, table, columns, order),
                rows(right, table, columns, order),
            );
            (left_rows != right_rows).then(|| {
                format!(
                    "{label}: only {} {:?}; only {} {:?}",
                    left.name,
                    left_rows
                        .difference(&right_rows)
                        .take(3)
                        .collect::<Vec<_>>(),
                    right.name,
                    right_rows
                        .difference(&left_rows)
                        .take(3)
                        .collect::<Vec<_>>()
                )
            })
        })
        .collect()
}

/// One run: random steps with or without faults, then settle.
fn run(seed: u64, steps: u64) -> std::result::Result<(), String> {
    let directory = tempfile::tempdir().unwrap();
    FULL_REPLAYS.with(|replays| replays.set(0));
    let mut simulation = Simulation::new(seed, directory.path());
    let faults = simulation.rng.chance(50);
    for _ in 0..steps {
        simulation.step(faults);
    }
    simulation.settle();
    let report = |simulation: &Simulation, why: &str| {
        let mut text = format!(
            "seed {seed}: {why}\nnodes {:?}\nfaults {:?}\n",
            simulation.digests(),
            simulation.faults
        );
        for left in 0..simulation.nodes.len() {
            for right in left + 1..simulation.nodes.len() {
                for table in differing_tables(&simulation.nodes[left], &simulation.nodes[right]) {
                    text.push_str(&format!("{table}\n"));
                }
            }
        }
        let tail = simulation.log.len().saturating_sub(40);
        text.push_str(&simulation.log[tail..].join("\n"));
        text
    };
    let authorities = simulation
        .digests()
        .into_iter()
        .map(|(_, authority, _)| authority)
        .collect::<BTreeSet<_>>();
    if authorities.len() != 1 {
        return Err(report(&simulation, "the nodes hold different envelopes"));
    }
    // Every claim extends the graph incrementally, or rebuilds the one run tree, desired
    // subject, document or mission it reaches out of order. Only a fault needs the full replay.
    let replays = FULL_REPLAYS.with(std::cell::Cell::get);
    if simulation.faults.is_empty() && replays != 0 {
        return Err(report(
            &simulation,
            &format!("without faults, projections replayed the graph from nothing {replays} times"),
        ));
    }
    let graphs = simulation
        .digests()
        .into_iter()
        .map(|(_, _, graph)| graph)
        .collect::<BTreeSet<_>>();
    if simulation.faults.is_empty() && graphs.len() != 1 {
        return Err(report(
            &simulation,
            "without faults, the same envelopes projected different graphs",
        ));
    }
    Ok(())
}

#[test]
fn nodes_that_sync_hold_the_same_envelopes_and_project_the_same_graph() {
    let steps = env_number("ST3_CONVERGENCE_STEPS", 80);
    let seeds = match std::env::var("ST3_CONVERGENCE_SEED") {
        Ok(seed) => vec![seed.parse().expect("ST3_CONVERGENCE_SEED is a number")],
        Err(_) => {
            // Fixed seeds keep CI repeatable; ST3_CONVERGENCE_RUNS explores more of them.
            (0..env_number("ST3_CONVERGENCE_RUNS", 8))
                .map(|index| 0x5eed_0000 + index)
                .collect::<Vec<_>>()
        }
    };
    // Each run has its own nodes, so runs share nothing and go in parallel.
    let failures = std::thread::scope(|scope| {
        seeds
            .iter()
            .map(|seed| scope.spawn(move || run(*seed, steps)))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|run| match run.join() {
                Ok(result) => result.err(),
                Err(panic) => std::panic::resume_unwind(panic),
            })
            .collect::<Vec<_>>()
    });
    assert!(
        failures.is_empty(),
        "{} of {} runs ended with different digests:\n\n{}",
        failures.len(),
        seeds.len(),
        failures.join("\n\n")
    );
}
