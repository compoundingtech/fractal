//! A missed CI objective is a fault for the fleet's fault agent.
//!
//! The objectives `slo/targets.toml` declares for CI (each `[[duration]]` and `[[count]]`) are
//! read from the rows the SLO history samples once a minute. An objective whose row is missed
//! opens one `operational.failure` on this member's `daemon/NODE` subject, addressed to the fleet
//! fault agent and never to a person; it stays one episode until the row is met again, when an
//! `operational.recovered` ends it. A row with no data, as after a restart before the observer
//! has read any runs, neither opens nor ends one. With no fleet fault agent, nothing opens.
//!
//! It stays cheap: a check whose rows hold no met or missed CI objective reads nothing, and one
//! that does reads this member's objective faults once through the claims' subject and kind
//! index. It writes only to open or end a fault.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::model::{ClaimInput, St3Error};
use smallclaims::error::internal;
use crate::store::Store;

/// Every objective episode starts with this, so its failures read apart from the daemon's others.
pub const EPISODE_PREFIX: &str = "slo-objective/";
pub const CONDITION: &str = "slo-objective-missed";

/// What one check did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectiveFaults {
    pub opened: usize,
    pub recovered: usize,
    /// Missed objectives left without a fault because the fleet has no fault agent.
    pub unowned: usize,
}

/// The CI objectives' ids: each duration's `NAME/pNN` and each count's `NAME`.
pub fn ci_objectives(targets: &crate::slo::Targets) -> BTreeSet<String> {
    targets
        .duration
        .iter()
        .map(|target| target.id())
        .chain(targets.count.iter().map(|target| target.name.clone()))
        .collect()
}

/// This member's episode for objective `id`.
pub fn episode(local: &str, id: &str) -> String {
    format!("{EPISODE_PREFIX}{local}/{id}")
}

/// Check `rows` and report a failure once until a check succeeds again. The SLO history keeps
/// its rows whatever this finds.
pub fn check_and_report(store: &Store, rows: &[Value]) {
    static FAILING: AtomicBool = AtomicBool::new(false);
    match check(store, rows, &ci_objectives(crate::slo::targets())) {
        Ok(_) => FAILING.store(false, Ordering::Relaxed),
        Err(error) => {
            if !FAILING.swap(true, Ordering::Relaxed) {
                eprintln!("st3: checking the CI objectives for faults failed: {error:?}");
            }
        }
    }
}

/// Open a fault for each objective in `objectives` that `rows` shows missed and that has no open
/// fault, and end the open fault of each that `rows` shows met.
pub fn check(
    store: &Store,
    rows: &[Value],
    objectives: &BTreeSet<String>,
) -> Result<ObjectiveFaults, St3Error> {
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
    let mut fault_agent = None;
    for (id, met, row) in read {
        let episode = episode(&local, id);
        let fault = faults.get(&episode);
        let open = fault.map_or(&[][..], |fault| &fault.open[..]);
        if met {
            for failure in open {
                store.append_claim(&ClaimInput {
                    subject: subject.clone(),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(episode)),
                        ("failure".into(), json!(failure)),
                        ("reason".into(), json!(format!("The objective {id} is met again: {}", row["current"].as_str().unwrap_or_default()))),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("{failure}:recovered")),
                })?;
            }
            check.recovered += usize::from(!open.is_empty());
            continue;
        }
        if !open.is_empty() {
            continue;
        }
        if fault_agent.is_none() {
            fault_agent = Some(store.fleet_fault_agent().map_err(internal)?);
        }
        let Some(Some(agent)) = &fault_agent else {
            check.unowned += 1;
            continue;
        };
        let previous = fault.and_then(|fault| fault.latest.as_deref()).unwrap_or("first");
        store.append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("condition".into(), json!(CONDITION)),
                ("reviewer".into(), json!(agent)),
                ("title".into(), json!(format!(
                    "CI objective missed: {id} is {} against {}",
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
