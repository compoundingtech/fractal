//! The attention list a daemon publishes for collection windows: every person's open attention
//! rows, folded off the request path by one refresher (`api::start_attention_list`) and served
//! under the cut they were folded at. One fold serves every person: a window keeps the rows
//! whose `person_id` it selects. A refresh replaces only source subjects and presentation
//! dependencies changed since the newest publication, or admits retained rows at their actual
//! eligibility boundary. Otherwise it republishes the same rows at the newer cut. Everything
//! here is volatile view state, cleared by `forget_views`.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};

/// The views the refresher publishes, in the order it refreshes them.
pub const PUBLISHED_VIEWS: [&str; 4] = ["attention", "glasses", "arrangements", "summary"];

#[derive(Default)]
pub struct ViewHealth {
    /// When its refreshes began failing, in Unix ms; 0 while they succeed.
    pub failing_since: AtomicU64,
    pub withdrawn: AtomicBool,
}

/// The rows of one attention fold and the cut they were folded at.
pub struct AttentionPublication {
    /// The graph index the rows were folded at; windows serving them carry this snapshot.
    pub cut: u64,
    /// The graph projection's health row at the cut. Deferred replication projection catches
    /// up under the same admitted claims, so a different row means the rows may have changed.
    pub projection: Option<(String, u64)>,
    /// The clock the rows were evaluated at: eligibility and grace periods depend on it.
    pub evaluated_at_unix_ms: u128,
    /// The next actual collection eligibility boundary. No periodic clock reevaluates
    /// unchanged source rows.
    pub due_at_unix_ms: Option<u128>,
    /// When these rows were published, in Unix ms: the list's "as of".
    pub published_at_unix_ms: u128,
    /// Every person's rows, most urgent first, as `client_attention_resources_at` orders them,
    /// each with the presentation `fleet` gives it.
    pub rows: Arc<Vec<Value>>,
    /// Subject-keyed durable source folds, retained independently of card presentation.
    pub source_snapshot: Option<Arc<super::attention_snapshot::AttentionSourceSnapshot>>,
    /// Owner-indexed rows in the same order, built only when the refresher changes the rows.
    pub owners: BTreeMap<String, Arc<Vec<Value>>>,
    /// The agents the rows' presentations read, from the newest published roster; `None`
    /// before the first.
    pub fleet: Option<Arc<AttentionFleet>>,
    /// Fleet keys read while presenting each row, including launch preview labels.
    pub presentation_dependencies: BTreeMap<String, AttentionPresentationDependencies>,
    /// The ask deadline policy the rows' deadlines were folded with, from `meta`
    /// (`person_work::ask_deadline_policy`). No claim records it, so a refresh that reads a
    /// different one folds again.
    pub ask_deadline_policy: (u128, u64),
}

/// What the attention presentation reads from an agents roster: each agent's label, the step
/// each holds, and an agent holding a step of each mission. Equal fleets present every row the
/// same, whatever roster revision they came from.
#[derive(Clone, Debug, Default)]
pub struct AttentionFleet {
    pub revision: u64,
    labels: HashMap<String, String>,
    claimants: HashMap<String, String>,
    working: HashMap<String, String>,
    inputs: HashMap<String, FleetAgentInput>,
    step_candidates: HashMap<String, BTreeMap<usize, String>>,
    mission_candidates: HashMap<String, BTreeMap<usize, String>>,
    /// Observation metadata, never presentation authority.
    observed_revision: Arc<AtomicU64>,
}

#[derive(Clone, Debug)]
struct FleetAgentInput {
    position: usize,
    name: String,
    host: Option<String>,
    work: Vec<(Option<String>, Option<String>)>,
}

impl FleetAgentInput {
    fn matches(&self, position: usize, id: &str, card: &Value) -> bool {
        self.position == position
            && self.name == card["name"].as_str().unwrap_or(id.trim_start_matches("agent/"))
            && self.host.as_deref() == card["host_id"].as_str()
            && self.work.iter().map(|(step, mission)| (step.as_deref(), mission.as_deref()))
                .eq(card["current_work"].as_array().into_iter().flatten()
                    .map(|work| (work["id"].as_str(), work["mission_id"].as_str())))
    }

    fn from_card(position: usize, id: &str, card: &Value) -> Self {
        Self {
            position,
            name: card["name"].as_str().unwrap_or(id.trim_start_matches("agent/")).to_owned(),
            host: card["host_id"].as_str().map(str::to_owned),
            work: card["current_work"].as_array().into_iter().flatten()
                .map(|work| (work["id"].as_str().map(str::to_owned),
                    work["mission_id"].as_str().map(str::to_owned))).collect(),
        }
    }
}

/// Exact roster inputs read by one card. Missing lookups count as dependencies too:
/// introducing a claimant or label must refresh a card that previously had none.
#[derive(Clone, Debug, Default)]
pub struct AttentionPresentationDependencies {
    pub labels: BTreeSet<String>,
    pub claimants: BTreeSet<String>,
    pub working: BTreeSet<String>,
}

impl AttentionPresentationDependencies {
    pub fn changed(&self, before: &AttentionFleet, after: &AttentionFleet) -> bool {
        self.labels.iter().any(|key| before.labels.get(key) != after.labels.get(key))
            || self.claimants.iter().any(|key| before.claimants.get(key) != after.claimants.get(key))
            || self.working.iter().any(|key| before.working.get(key) != after.working.get(key))
    }
}

impl PartialEq for AttentionFleet {
    fn eq(&self, other: &Self) -> bool {
        (&self.labels, &self.claimants, &self.working) == (&other.labels, &other.claimants, &other.working)
    }
}

impl AttentionFleet {
    /// The fleet of a roster's cards, in roster order: the first agent holding a step of a
    /// mission is the one a card about that mission reaches.
    pub fn from_cards(revision: u64, cards: &[Value]) -> Self {
        let mut fleet = AttentionFleet { revision, ..Default::default() };
        fleet.observed_revision.store(revision, AtomicOrdering::Release);
        for (position, card) in cards.iter().enumerate() {
            let Some(id) = card["id"].as_str() else { continue };
            let name = card["name"].as_str().unwrap_or(id.trim_start_matches("agent/"));
            fleet.labels.insert(id.to_owned(), fractal_ui_model::agents::label(name, card["host_id"].as_str()));
            for work in card["current_work"].as_array().into_iter().flatten() {
                if let Some(step) = work["id"].as_str() {
                    fleet.claimants.entry(step.to_owned()).or_insert_with(|| id.to_owned());
                }
                if let Some(mission) = work["mission_id"].as_str() {
                    fleet.working.entry(mission.to_owned()).or_insert_with(|| id.to_owned());
                }
            }
            let input = FleetAgentInput::from_card(position, id, card);
            fleet.add_candidates(id, &input);
            fleet.inputs.insert(id.to_owned(), input);
        }
        fleet
    }

    /// Advance only changed agents' label and work inputs. The second result counts inputs
    /// updated/removed, not cards scanned. Unrelated card fields retain the exact prior Arc.
    pub fn updated(previous: &Arc<Self>, revision: u64, cards: &[Value]) -> (Arc<Self>, usize) {
        let mut changed = Vec::new();
        let mut count = 0;
        for (position, card) in cards.iter().enumerate() {
            let Some(id) = card["id"].as_str() else { continue };
            count += 1;
            if previous.inputs.get(id).is_none_or(|input| !input.matches(position, id, card)) {
                changed.push((position, id, card));
            }
        }
        if changed.is_empty() && count == previous.inputs.len() {
            previous.observed_revision.store(revision, AtomicOrdering::Release);
            return (Arc::clone(previous), 0);
        }
        let present = cards.iter().filter_map(|card| card["id"].as_str()).collect::<HashSet<_>>();
        let removed = previous.inputs.keys().filter(|id| !present.contains(id.as_str()))
            .map(String::as_str).collect::<Vec<_>>();
        let mut next = (**previous).clone();
        next.revision = revision;
        next.observed_revision = Arc::new(AtomicU64::new(revision));
        let mut steps = BTreeSet::new();
        let mut missions = BTreeSet::new();
        for id in removed.iter().copied().chain(changed.iter().map(|(_, id, _)| *id)) {
            if let Some(input) = next.inputs.remove(id) {
                next.labels.remove(id);
                for (step, mission) in &input.work {
                    if let Some(step) = step {
                        steps.insert(step.clone());
                        if let Some(candidates) = next.step_candidates.get_mut(step) {
                            candidates.remove(&input.position);
                        }
                    }
                    if let Some(mission) = mission {
                        missions.insert(mission.clone());
                        if let Some(candidates) = next.mission_candidates.get_mut(mission) {
                            candidates.remove(&input.position);
                        }
                    }
                }
            }
        }
        let updated = changed.len() + removed.len();
        for (position, id, card) in changed {
            let input = FleetAgentInput::from_card(position, id, card);
            next.labels.insert(id.to_owned(), fractal_ui_model::agents::label(&input.name, input.host.as_deref()));
            for (step, mission) in &input.work {
                steps.extend(step.iter().cloned());
                missions.extend(mission.iter().cloned());
            }
            next.add_candidates(id, &input);
            next.inputs.insert(id.to_owned(), input);
        }
        Self::update_winners(&steps, &mut next.step_candidates, &mut next.claimants);
        Self::update_winners(&missions, &mut next.mission_candidates, &mut next.working);
        if next == **previous {
            previous.observed_revision.store(revision, AtomicOrdering::Release);
            (Arc::clone(previous), updated)
        } else {
            (Arc::new(next), updated)
        }
    }

    fn add_candidates(&mut self, id: &str, input: &FleetAgentInput) {
        for (step, mission) in &input.work {
            if let Some(step) = step {
                self.step_candidates.entry(step.clone()).or_default().insert(input.position, id.to_owned());
            }
            if let Some(mission) = mission {
                self.mission_candidates.entry(mission.clone()).or_default().insert(input.position, id.to_owned());
            }
        }
    }

    fn update_winners(
        affected: &BTreeSet<String>,
        candidates: &mut HashMap<String, BTreeMap<usize, String>>,
        winners: &mut HashMap<String, String>,
    ) {
        for key in affected {
            if let Some((_, winner)) = candidates.get(key).and_then(BTreeMap::first_key_value) {
                if winners.get(key) != Some(winner) {
                    winners.insert(key.clone(), winner.clone());
                }
            } else {
                candidates.remove(key);
                winners.remove(key);
            }
        }
    }

    /// Newest checked roster, including revisions with identical semantic inputs.
    pub fn observed_revision(&self) -> u64 {
        self.observed_revision.load(AtomicOrdering::Acquire)
    }

    pub fn agent_label(&self, agent: &str) -> Option<String> {
        self.labels.get(agent).cloned()
    }

    pub fn claimant(&self, step: &str) -> Option<String> {
        self.claimants.get(step).cloned()
    }

    pub fn working_in(&self, mission: &str) -> Option<String> {
        self.working.get(mission).cloned()
    }
}

impl AttentionPublication {
    pub fn index_owners(rows: &[Value]) -> BTreeMap<String, Arc<Vec<Value>>> {
        let mut owners = BTreeMap::<String, Vec<Value>>::new();
        for row in rows {
            if let Some(person) = row["person_id"].as_str() {
                owners.entry(person.to_owned()).or_default().push(row.clone());
            }
        }
        owners.into_iter().map(|(person, rows)| (person, Arc::new(rows))).collect()
    }
}

/// The refresher's registration, its requests and the newest publication.
#[derive(Default)]
pub struct AttentionList {
    pub refresher: std::sync::OnceLock<Arc<tokio::sync::Notify>>,
    /// When a window last read one of the refresher's views (attention, glasses, arrangements),
    /// in Unix ms. The refresher follows commits only while someone reads.
    pub read_at: AtomicU64,
    pub published: Mutex<Option<Arc<AttentionPublication>>>,
    /// Counts `forget`s. A refresh that began before one may not publish what it read.
    pub forgotten: AtomicU64,
    /// Counts publications, same cut or not, so streams that read an earlier one reread.
    pub revision: tokio::sync::watch::Sender<u64>,
    /// The graph index through which pairing changes were rechecked.
    pub pairings_checked: AtomicU64,
    /// Set when the refresher task ends: no view is served after that.
    pub stopped: AtomicBool,
    pub health: [ViewHealth; PUBLISHED_VIEWS.len()],
    /// Folds by cause: the first claim kind that could change attention, `clock`, `projection`,
    /// `first` or `many`. Republishing the same rows at a newer cut is not a fold.
    pub folds: Mutex<BTreeMap<String, u64>>,
}

impl AttentionList {
    pub fn forget(&self) {
        let mut published = self.published.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        let forgotten = published.take().is_some();
        drop(published);
        if forgotten {
            self.revision.send_modify(|revision| *revision += 1);
        }
    }
}

impl Store {
    /// Follows the revision of attention publications, so a stream that read an earlier one
    /// rereads the newer.
    #[cfg(any(test, feature = "test-support"))]
    pub fn subscribe_attention_list(&self) -> tokio::sync::watch::Receiver<u64> {
        self.fractal.attention_list.revision.subscribe()
    }

}

#[cfg(test)]
#[path = "attention_list/tests.rs"]
mod tests;

#[cfg(test)]
mod incremental_fleet_tests {
    use super::*;

    fn cards() -> Vec<Value> {
        vec![
            json!({"id":"agent/oak","name":"Oak","host_id":"host/a",
                "current_work":[{"id":"step-run/shared","mission_id":"mission/shared"}]}),
            json!({"id":"agent/elm","name":"Elm","host_id":"host/b",
                "current_work":[{"id":"step-run/shared","mission_id":"mission/shared"}]}),
            json!({"id":"agent/ash","name":"Ash","current_work":[]}),
        ]
    }

    #[test]
    fn unchanged_fields_reuse_fleet_and_advance_observed_revision() {
        let mut cards = cards();
        let original = Arc::new(AttentionFleet::from_cards(1, &cards));
        cards[0]["state"] = json!("waiting");
        cards[1]["current_work"][0]["title"] = json!("A different work title");
        let (same, updates) = AttentionFleet::updated(&original, 2, &cards);
        assert!(Arc::ptr_eq(&same, &original));
        assert_eq!(updates, 0);
        assert_eq!(same.observed_revision(), 2);
        cards[1]["name"] = json!("New Elm");
        let (changed, updates) = AttentionFleet::updated(&same, 3, &cards);
        assert_eq!(updates, 1, "only the renamed agent input is rebuilt");
        assert!(!Arc::ptr_eq(&changed, &same));
        assert_eq!(*changed, AttentionFleet::from_cards(3, &cards));
        assert_eq!(original.agent_label("agent/elm"),
            Some(fractal_ui_model::agents::label("Elm", Some("host/b"))));
        assert_eq!(changed.claimant("step-run/shared").as_deref(), Some("agent/oak"));
    }

    #[test]
    fn winner_indexes_preserve_first_roster_agent_after_reorder_and_removal() {
        let mut cards = cards();
        let initial = Arc::new(AttentionFleet::from_cards(1, &cards));
        cards.swap(0, 1);
        let (reordered, updates) = AttentionFleet::updated(&initial, 2, &cards);
        assert_eq!(updates, 2);
        assert_eq!(reordered.claimant("step-run/shared").as_deref(), Some("agent/elm"));
        assert_eq!(reordered.working_in("mission/shared").as_deref(), Some("agent/elm"));
        assert_eq!(*reordered, AttentionFleet::from_cards(2, &cards));
        cards.remove(0);
        let (removed, _) = AttentionFleet::updated(&reordered, 3, &cards);
        assert_eq!(removed.claimant("step-run/shared").as_deref(), Some("agent/oak"));
        assert_eq!(*removed, AttentionFleet::from_cards(3, &cards));
        cards[0]["current_work"] = json!([]);
        let (idle, updates) = AttentionFleet::updated(&removed, 4, &cards);
        assert_eq!(updates, 1);
        assert_eq!(idle.claimant("step-run/shared"), None);
        assert_eq!(idle.working_in("mission/shared"), None);
        assert_eq!(*idle, AttentionFleet::from_cards(4, &cards));
    }
}
