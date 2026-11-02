//! The attention list a daemon publishes for collection windows: every person's open attention
//! rows, folded off the request path by one refresher (`api::start_attention_list`) and served
//! under the cut they were folded at. One fold serves every person: a window keeps the rows
//! whose `person_id` it selects. A refresh folds again only when a claim admitted since the
//! newest publication can change what attention reads, the projection moved under the same
//! claims, or the clock passed the publication's period; otherwise it republishes the same rows
//! at the newer cut. Everything here is volatile cache state, cleared by `forget_views`.

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
    /// The earliest acceptance time after `evaluated_at_unix_ms` among the claims this list's
    /// folds and deltas read: a claim dated in the future that becomes eligible then. The
    /// rows are evaluated again at that time, not only at the clock period.
    pub due_at_unix_ms: Option<u128>,
    /// Claims after this index may still be dated after the clock the rows were evaluated at;
    /// `None` when none is.
    pub future_after: Option<u64>,
    /// When these rows were published, in Unix ms: the list's "as of".
    pub published_at_unix_ms: u128,
    /// Every person's rows, most urgent first, as `client_attention_resources_at` orders them,
    /// each with the presentation `fleet` gives it.
    pub rows: Arc<Vec<Value>>,
    /// Owner-indexed rows in the same order, built only when the refresher changes the rows.
    pub owners: BTreeMap<String, Arc<Vec<Value>>>,
    /// The agents the rows' presentations read, from the newest published roster; `None`
    /// before the first.
    pub fleet: Option<Arc<AttentionFleet>>,
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
        for card in cards {
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
        }
        fleet
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
