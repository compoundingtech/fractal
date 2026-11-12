use super::*;

const MAX_REQUEST_REFS: usize = 128;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AttentionDeclaration {
    Undeclared,
    Declared { asks: u64 },
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct AttentionRequestReference {
    pub claim_id: String,
    pub subject: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AttentionBudgetReport {
    pub mission: String,
    pub person: String,
    pub window_start_exclusive_unix_ms: u128,
    pub window_end_inclusive_unix_ms: u128,
    pub local_claim_index: u64,
    pub declared: AttentionDeclaration,
    pub charged: usize,
    pub inclusive: usize,
    pub not_charged: usize,
    pub unattributed_for_person: usize,
    pub fleet_complete: bool,
    pub request_refs: Vec<AttentionRequestReference>,
    pub request_refs_complete: bool,
}

impl Store {
    /// Data only, at one retained-local snapshot. Each run keeps its initial budget.
    pub fn attention_budget_report(
        &self,
        mission: &str,
        person: &str,
        at: u128,
    ) -> Result<AttentionBudgetReport> {
        anyhow::ensure!(
            fractal_schema::accountable::person(person),
            "expected a native person subject"
        );
        self.read_snapshot_unindexed(|| {
            let connection = self.readers.get();
            let resolved = identities::resolve_identity_conn(
                &connection,
                identities::IdentityKind::Mission,
                mission,
            )?;
            let storage = resolved
                .identity
                .strip_prefix("mission/")
                .context("invalid mission identity")?;
            report(&connection, storage, &resolved.canonical_id, person, at)
        })
    }
}

fn declaration(
    connection: &Connection,
    mission: &str,
    revision: &str,
    person: &str,
) -> Result<AttentionDeclaration> {
    let body: String = connection
        .query_row(
            "SELECT body FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
            params![mission, revision],
            |row| row.get(0),
        )
        .context("attention budget requires the retained mission revision")?;
    let spec: Value = serde_json::from_str(&body)?;
    let asks = spec["declared_budgets"]["attention"]
        .as_array()
        .and_then(|budgets| budgets.iter().find(|budget| budget["person"] == person))
        .and_then(|budget| budget["asks"].as_u64());
    Ok(asks.map_or(AttentionDeclaration::Undeclared, |asks| {
        AttentionDeclaration::Declared { asks }
    }))
}

// Count already verified each origin step and run. Classify once per originating
// run, not once per ask. Walk each ancestor's own initial revision, never the
// root revision or a later generation. The same 64-vertex bound applies here.
const PINNED_REVISIONS: &str = "WITH RECURSIVE ancestry(id, mission_id, initial_revision, root_run_id, parent_step_run, depth) AS (
 SELECT id, mission_id, initial_revision, root_run_id, parent_step_run, 0 FROM mission_runs WHERE id=?1
 UNION ALL
 SELECT r.id, r.mission_id, r.initial_revision, r.root_run_id, r.parent_step_run, a.depth+1
 FROM ancestry a JOIN step_runs s ON s.subject=a.parent_step_run
 JOIN mission_runs r ON r.id=s.run_id AND r.root_run_id=a.root_run_id WHERE a.depth<63
) SELECT mission_id, initial_revision,
         EXISTS(SELECT 1 FROM ancestry a JOIN step_runs s ON s.subject=a.parent_step_run
                JOIN mission_runs r ON r.id=s.run_id AND r.root_run_id=a.root_run_id
                WHERE a.depth=63)
  FROM ancestry ORDER BY depth";

/// Charging policy: every declaring ancestor, pinned to that run's initial revision.
pub(super) fn run_charges_mission(
    connection: &Connection,
    run: &str,
    mission: &str,
    person: &str,
    revisions: &mut BTreeMap<String, bool>,
) -> Result<bool> {
    let mut ancestry = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h48f56e683553 owner=fractal_store::store::attention_budget::report TODO=trace-statement-and-value-representation */ PINNED_REVISIONS)?;
    for ancestor in ancestry.query_map([run], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
    })? {
        let (ancestor, revision, overflow) = ancestor?;
        anyhow::ensure!(!overflow, "attention ancestry limit of 64 vertices exceeded");
        if ancestor != mission {
            continue;
        }
        let declares = match revisions.entry(revision) {
            std::collections::btree_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let declares = matches!(
                    declaration(connection, mission, entry.key(), person)?,
                    AttentionDeclaration::Declared { .. }
                );
                entry.insert(declares);
                declares
            }
        };
        if declares {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn report(
    connection: &Connection,
    storage: &str,
    mission: &str,
    person: &str,
    at: u128,
) -> Result<AttentionBudgetReport> {
    let revision: String = connection
        .query_row(
            "SELECT revision FROM mission_definitions WHERE mission_id=?1",
            [storage],
            |row| row.get(0),
        )
        .context("attention budget requires a current mission revision")?;
    let declared = declaration(connection, storage, &revision, person)?;
    let local_claim_index = connection.query_row(
        "SELECT COALESCE(MAX(store_index),0) FROM claims",
        [],
        |row| row.get(0),
    )?;
    let mut references = BTreeMap::new();
    let counts = count_with_refs(connection, storage, person, at, |ask, _, _| {
        references.insert(
            (ask.accepted_at_unix_ms, ask.id.clone()),
            AttentionRequestReference {
                claim_id: ask.id.clone(),
                subject: ask.subject.clone(),
            },
        );
        if references.len() > MAX_REQUEST_REFS {
            references.pop_first();
        }
    })?;
    let mut charged = 0;
    let mut revisions = BTreeMap::new();
    for (run, requests) in &counts.requests_by_originating_run {
        let run = run.strip_prefix("mission-run/").context("invalid origin run")?;
        if run_charges_mission(connection, run, storage, person, &mut revisions)? {
            charged += requests;
        }
    }
    Ok(AttentionBudgetReport {
        mission: mission.into(),
        person: person.into(),
        window_start_exclusive_unix_ms: counts.window_start_exclusive_unix_ms,
        window_end_inclusive_unix_ms: counts.window_end_inclusive_unix_ms,
        local_claim_index,
        declared,
        charged,
        inclusive: counts.requests,
        not_charged: counts.requests - charged,
        unattributed_for_person: counts.unattributed_for_person,
        fleet_complete: false,
        request_refs_complete: references.len() == counts.requests,
        request_refs: references.into_values().rev().collect(),
    })
}
