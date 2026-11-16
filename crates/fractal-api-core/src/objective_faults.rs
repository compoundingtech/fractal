//! A missed objective row is a fault for the reviewer its objective names.
//!
//! A declared objective's missed row, the API readiness objective's and the agents list request
//! objective's each open one `operational.failure` on this member's `daemon/NODE` subject,
//! addressed to that reviewer, an agent and never a person; it stays one episode until the row is
//! met again, when an `operational.recovered` ends it. A row with no data neither opens nor ends
//! one. With the reviewer not a live agent declaration, nothing opens.
//!
//! It stays cheap: a check whose rows hold no met or missed objective it is asked about reads
//! nothing, and one that does reads this member's objective faults once through the claims'
//! subject and kind index. It writes only to open or end a fault.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::model::{ClaimInput, FractalError};
use smallclaims::error::internal;
use crate::store::Store;

/// Every objective episode starts with this, so its failures read apart from the daemon's others.
pub const EPISODE_PREFIX: &str = "slo-objective/";

/// What one check did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectiveFaults {
    pub opened: usize,
    pub recovered: usize,
    /// Missed objectives left without a fault because their reviewer is not a live agent.
    pub unowned: usize,
}

/// This member's episode for objective `id`.
pub fn episode(local: &str, id: &str) -> String {
    format!("{EPISODE_PREFIX}{local}/{id}")
}

/// Who a missed objective's fault is addressed to, and how it reads.
pub(crate) struct ObjectiveResponse<'a> {
    pub reviewer: &'a str,
    pub condition: &'a str,
    pub title: &'a str,
}

/// Open a fault for each objective in `objectives` that `rows` shows missed and that has no open
/// fault, and end the open fault of each that `rows` shows met.
pub(crate) fn check_with_response(
    store: &Store,
    rows: &[Value],
    objectives: &BTreeSet<String>,
    response: ObjectiveResponse<'_>,
) -> Result<ObjectiveFaults, FractalError> {
    let mut check = ObjectiveFaults::default();
    let read: Vec<(&str, bool, &Value)> = rows
        .iter()
        .filter_map(|row| {
            let id = row["id"].as_str().filter(|id| objectives.contains(*id))?;
            match row["status"].as_str()? {
                "met" => Some((id, true, row)),
                "missed" => Some((id, false, row)),
                _ => None,
            }
        })
        .collect();
    if read.is_empty() {
        return Ok(check);
    }
    let local = store.origin().to_owned();
    let subject = format!("daemon/{local}");
    let faults = objective_faults(store, &subject).map_err(internal)?;
    // Whether the reviewer is a live agent declaration: read once, and only for a miss.
    let mut live = None;
    for (id, met, row) in read {
        let episode = episode(&local, id);
        let fault = faults.get(&episode);
        let open = fault.map_or(&[][..], |fault| &fault.open[..]);
        if met {
            let reason = format!("The objective {id} is met again: {}", row["current"].as_str().unwrap_or_default());
            for failure in open {
                recover(store, &subject, &episode, failure, &reason)?;
            }
            check.recovered += usize::from(!open.is_empty());
            continue;
        }
        if !open.is_empty() {
            continue;
        }
        if live.is_none() {
            live = Some(store.fault_reviewer_is_declared(response.reviewer).map_err(internal)?);
        }
        if live != Some(true) {
            check.unowned += 1;
            continue;
        }
        let previous = fault.and_then(|fault| fault.latest.as_deref()).unwrap_or("first");
        store.append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("condition".into(), json!(response.condition)),
                ("reviewer".into(), json!(response.reviewer)),
                ("title".into(), json!(format!(
                    "{} objective missed: {id} is {} against {}",
                    response.title,
                    row["current"].as_str().unwrap_or_default(),
                    row["target"].as_str().unwrap_or_default(),
                ))),
                ("reason".into(), json!(format!(
                    "{} This fault ends when the objective is met again.",
                    row["explain"].as_str().unwrap_or_default(),
                ))),
                ("severity".into(), json!("warning")),
                ("targets".into(), json!([subject])),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("{episode}:after:{previous}")),
        })?;
        check.opened += 1;
    }
    Ok(check)
}

/// End the open faults of this member's objective rows that `owns` claims and `judged` no longer
/// holds: a row that is no longer evaluated, or that carries no goal, is never met again, so its
/// fault would otherwise stay open. Returns the episodes ended.
pub(crate) fn end_unjudged(store: &Store, owns: impl Fn(&str) -> bool, judged: &BTreeSet<String>) -> Result<usize, FractalError> {
    end_open(store, |id| !judged.contains(id) && owns(id), |id| {
        format!("The objective {id} is no longer judged: its row is gone or carries no goal.")
    })
}

/// The CI objectives `slo/objectives.kdl` shipped before `slo/fractal-ci.kdl` declared them under
/// `fractal/`. Nothing evaluates them any more, so a fault one opened would never be met again.
const RETIRED_CI_OBJECTIVES: [&str; 4] = ["merge-latency/p50", "merge-latency/p95", "ci-critical-path/p95", "flakes"];

/// End this member's open faults of the retired shipped CI objectives. Idempotent: each failure
/// is recovered once. Returns the episodes ended.
pub fn end_retired_ci_objectives(store: &Store) -> Result<usize, FractalError> {
    end_open(store, |id| RETIRED_CI_OBJECTIVES.contains(&id), |id| {
        format!("The objective {id} is retired: Fractal no longer ships it, and slo/fractal-ci.kdl declares its successor under fractal/.")
    })
}

/// End each open objective episode of this member whose id `ends` picks, with `reason`'s reason.
fn end_open(store: &Store, ends: impl Fn(&str) -> bool, reason: impl Fn(&str) -> String) -> Result<usize, FractalError> {
    let local = store.origin().to_owned();
    let subject = format!("daemon/{local}");
    let prefix = episode(&local, "");
    let mut ended = 0;
    for (episode, fault) in objective_faults(store, &subject).map_err(internal)? {
        let Some(id) = episode.strip_prefix(&prefix) else {
            continue;
        };
        if fault.open.is_empty() || !ends(id) {
            continue;
        }
        let reason = reason(id);
        for failure in &fault.open {
            recover(store, &subject, &episode, failure, &reason)?;
        }
        ended += 1;
    }
    Ok(ended)
}

fn recover(store: &Store, subject: &str, episode: &str, failure: &str, reason: &str) -> Result<(), FractalError> {
    store.append_claim(&ClaimInput {
        subject: subject.to_owned(),
        kind: "operational.recovered".into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("episode".into(), json!(episode)),
            ("failure".into(), json!(failure)),
            ("reason".into(), json!(reason)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("{failure}:recovered")),
    })?;
    Ok(())
}

/// One objective's faults on this member: its failures no recovery names, and its newest failure.
#[derive(Debug, Default)]
struct Episode {
    open: Vec<String>,
    latest: Option<String>,
}

fn objective_faults(store: &Store, subject: &str) -> anyhow::Result<BTreeMap<String, Episode>> {
    let connection = store.readers.get();
    let mut query = connection.prepare(
        "SELECT id, kind, json_extract(body,'$.fields.episode'), json_extract(body,'$.fields.failure')
         FROM claims WHERE subject=?1
           AND kind IN ('operational.failure','operational.recovered')
           AND json_extract(body,'$.fields.episode') LIKE ?2
         ORDER BY store_index",
    )?;
    let rows = query
        .query_map(rusqlite::params![subject, format!("{EPISODE_PREFIX}%")], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let recovered: BTreeSet<&str> = rows
        .iter()
        .filter(|(_, kind, ..)| kind == "operational.recovered")
        .filter_map(|(.., failure)| failure.as_deref())
        .collect();
    let mut episodes: BTreeMap<String, Episode> = BTreeMap::new();
    for (id, kind, episode, _) in &rows {
        if kind != "operational.failure" {
            continue;
        }
        let entry = episodes.entry(episode.clone()).or_default();
        entry.latest = Some(id.clone());
        if !recovered.contains(id.as_str()) {
            entry.open.push(id.clone());
        }
    }
    Ok(episodes)
}

#[cfg(test)]
#[path = "objective_faults_tests.rs"]
mod tests;
