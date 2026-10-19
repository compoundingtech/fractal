//! The current missions list as a published view: every mission's place in the list, and the
//! card of each mission it shows. The refresher in `api::published_lists` folds it; this module
//! holds the rows and the reads that say which missions changed between two cuts.

use super::*;

/// Where one mission sits in the current list, as `mission_collection_page` orders and filters
/// it, and until when that holds with no new claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionKey {
    /// The list's sort time, newest first; a mission with none sorts last.
    pub order: Option<i64>,
    /// The time a page continuation seeks after.
    pub page_time: u128,
    pub visible: bool,
    /// The last moment a recently ended run keeps its mission shown, if that is all that does.
    pub visible_until: Option<u128>,
}

/// One publication of the current missions list.
#[derive(Clone, Default)]
pub struct MissionRows {
    /// Every shown mission's place in the list, by its `mission/` ID.
    pub keys: HashMap<String, MissionKey>,
    /// The shown missions, in list order.
    pub order: Vec<String>,
    /// The card of every shown mission.
    pub cards: HashMap<String, Arc<Value>>,
    /// The runs that wait on a person, which every card's `must_act` reads.
    pub attention_runs: BTreeSet<String>,
    /// When the attention runs were last read: grace periods can move a run in or out of
    /// them with no new claim.
    pub attention_read_at_unix_ms: u128,
    /// When the cards were folded: a worker lease that ends after it changes a card.
    pub folded_at_unix_ms: u128,
    /// The first worker lease to end after the fold, if one does.
    pub next_lease_end_unix_ms: Option<u128>,
    /// Acceptance time of the publication's graph cut, captured off the request path.
    pub time_unix_ms: u128,
    /// The projection frontier at the fold, from [`Store::projection_frontier`].
    pub frontier: u64,
    /// Every leased step's lease end at the fold. A quiet renewal moves one, and the step's
    /// `since` on its card, with no claim.
    pub leases: HashMap<String, u128>,
    /// The open human gates, read with `attention_runs`; every card's presentation reads them.
    pub gates: Vec<st3_ui_model::missions::Gate>,
    /// The agents every card's presentation reads, from the newest published roster. None until
    /// a roster is published, which is not an empty roster: then a word that hinges on an agent's
    /// queue goes unsaid.
    pub roster: Option<Arc<RosterFleet>>,
}

/// The agents a mission presentation reads, from one published roster: each agent's state and
/// the step it holds, and which agent's queue holds each queued step.
#[derive(Debug, Default)]
pub struct RosterFleet {
    /// The roster publication it reads, by `Store::agent_roster_revision`: a roster republished
    /// at the same cut after local activity is a new one.
    pub revision: u64,
    seats: HashMap<String, st3_ui_model::missions::Seat>,
    /// Queued step → the agent whose queue holds it.
    queues: HashMap<String, String>,
    /// Per agent, every step it holds or queues: a change to an agent changes these steps.
    steps: HashMap<String, Vec<String>>,
}

impl RosterFleet {
    /// The fleet of `cards`, the roster publication `revision`.
    pub fn from_cards(revision: u64, cards: &[Value]) -> Self {
        let mut fleet = RosterFleet { revision, ..Default::default() };
        for card in cards {
            let Some(id) = card["id"].as_str() else { continue };
            let ids = |field: &str| {
                card[field]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            let queued = card["next_work_id"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .chain(ids("upcoming_work_ids"))
                .collect::<Vec<_>>();
            for step in &queued {
                fleet.queues.entry(step.clone()).or_insert_with(|| id.to_owned());
            }
            let held = &card["current_work"][0];
            fleet.seats.insert(
                id.to_owned(),
                st3_ui_model::missions::Seat {
                    id: id.to_owned(),
                    label: card["name"].as_str().unwrap_or(id).to_owned(),
                    state: card["state"].as_str().unwrap_or("unknown").to_owned(),
                    fault: !card["fault"].is_null(),
                    busy_with: held["mission_id"]
                        .as_str()
                        .zip(held["path"].as_str())
                        .map(|(mission, path)| (mission.to_owned(), path.to_owned())),
                },
            );
            fleet.steps.insert(id.to_owned(), ids("current_work_ids").into_iter().chain(queued).collect());
        }
        fleet
    }

    /// The agents whose state, fault, held step or queue differ from `before`, by agent ID.
    pub fn agents_changed_since(&self, before: &RosterFleet) -> BTreeSet<String> {
        let ids = self.seats.keys().chain(before.seats.keys());
        ids.filter(|id| {
            self.seats.get(*id) != before.seats.get(*id) || self.steps.get(*id) != before.steps.get(*id)
        })
        .cloned()
        .collect()
    }

    /// Every step `agent` holds or queues, here and in `before`.
    pub fn steps_of<'a>(&'a self, before: &'a RosterFleet, agent: &str) -> impl Iterator<Item = &'a String> {
        self.steps.get(agent).into_iter().chain(before.steps.get(agent)).flatten()
    }
}

impl st3_ui_model::missions::Fleet for RosterFleet {
    fn seat(&self, agent: &str) -> Option<st3_ui_model::missions::Seat> {
        self.seats.get(agent).cloned()
    }
    fn queued_for(&self, step: &str) -> Option<st3_ui_model::missions::Seat> {
        self.queues.get(step).and_then(|agent| self.seat(agent))
    }
    fn mission_label(&self, mission: &str) -> String {
        mission.trim_start_matches("mission/").to_owned()
    }
    fn person_label(&self, person: &str) -> String {
        person.trim_start_matches("person/").to_owned()
    }
}

impl MissionRows {
    /// Put the shown missions in list order: newest sort time first, missions without one
    /// last, then by ID, as `mission_collection_page` orders them.
    pub fn sort(&mut self) {
        let mut order = self
            .keys
            .iter()
            .filter(|(_, key)| key.visible)
            .map(|(id, key)| (key.order, id.clone()))
            .collect::<Vec<_>>();
        order.sort_by(|(left_time, left), (right_time, right)| {
            match (left_time, right_time) {
                (Some(left), Some(right)) => right.cmp(left),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| left.cmp(right))
        });
        self.order = order.into_iter().map(|(_, id)| id).collect();
    }

    /// The first time a shown mission leaves the list with no new claim: just after the last
    /// moment its recent end keeps it shown.
    pub fn visible_until(&self) -> Option<u128> {
        self.keys.values().filter_map(|key| key.visible_until).min().map(|until| until + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roster_fleet_names_seats_and_queues_and_says_which_agents_changed() {
        use st3_ui_model::missions::Fleet as _;
        let card = |state: &str, next: Option<&str>| {
            json!({
                "id": "agent/example/keeper", "name": "Keeper", "state": state, "fault": null,
                "current_work_ids": ["step-run/a/held"],
                "current_work": [{"id": "step-run/a/held", "mission_id": "mission/a", "path": "held"}],
                "next_work_id": next, "upcoming_work_ids": ["step-run/b/later"],
            })
        };
        let before = RosterFleet::from_cards(1, &[card("running", Some("step-run/b/next"))]);
        let seat = before.queued_for("step-run/b/next").unwrap();
        assert_eq!((seat.label.as_str(), seat.state.as_str()), ("Keeper", "running"));
        assert_eq!(seat.busy_with, Some(("mission/a".into(), "held".into())));
        assert!(before.queued_for("step-run/b/later").is_some());
        assert!(before.queued_for("step-run/a/held").is_none(), "held is not queued");
        assert!(before.agents_changed_since(&before).is_empty());
        let after = RosterFleet::from_cards(2, &[card("stopped", Some("step-run/b/next"))]);
        let changed = after.agents_changed_since(&before);
        assert_eq!(changed.iter().map(String::as_str).collect::<Vec<_>>(), ["agent/example/keeper"]);
        let steps = after.steps_of(&before, "agent/example/keeper").cloned().collect::<BTreeSet<_>>();
        assert!(["step-run/a/held", "step-run/b/next", "step-run/b/later"].iter().all(|step| steps.contains(*step)));
        // A queue that moved is a change too.
        let moved = RosterFleet::from_cards(3, &[card("running", None)]);
        assert_eq!(moved.agents_changed_since(&before).len(), 1);
    }

    #[test]
    fn missions_sort_newest_first_then_by_id_with_untimed_ones_last() {
        let key = |order| MissionKey { order, page_time: 0, visible: true, visible_until: None };
        let mut rows = MissionRows::default();
        for (id, order) in [("b", Some(5)), ("a", Some(5)), ("c", None), ("d", Some(9))] {
            rows.keys.insert(id.into(), key(order));
        }
        rows.keys.insert("hidden".into(), MissionKey { visible: false, ..key(Some(99)) });
        rows.sort();
        assert_eq!(rows.order, ["d", "a", "b", "c"]);
    }
}
