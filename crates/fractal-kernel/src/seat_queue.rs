//! The ordered queue of mission runs for one agent seat.
//!
//! A seat holds at most one claim. A run joins the end of the seat's queue when
//! it first has a step assigned to that seat and leaves when the run is
//! terminal. Each `agent.queue.moved` claim moves one run and is replayed after
//! the joins recorded before it, so a move is one durable record with its own
//! actor and time. Order inside a run still comes from the mission's
//! dependencies; the seat queue only orders runs against each other.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const MOVED_CLAIM: &str = "agent.queue.moved";

/// Compare work actors without changing persisted rows or conflating recognized actor kinds.
pub fn work_actor_matches(candidate: Option<&str>, actor: &str) -> bool {
    let Some(candidate) = candidate.and_then(fractal_schema::canonical_agent_actor) else {
        return false;
    };
    fractal_schema::canonical_agent_actor(actor).as_deref() == Some(candidate.as_ref())
}

#[derive(Clone, Copy)]
enum ActorMatching {
    Literal,
    Aliases,
}

impl ActorMatching {
    fn matches(self, candidate: Option<&str>, actor: &str) -> bool {
        match self {
            Self::Literal => candidate == Some(actor),
            Self::Aliases => work_actor_matches(candidate, actor),
        }
    }

    fn same(self, left: Option<&str>, right: Option<&str>) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => self.matches(Some(left), right),
            (None, None) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    Top,
    Bottom,
    Before,
    After,
}

impl Placement {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "before" => Self::Before,
            "after" => Self::After,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Before => "before",
            Self::After => "after",
        }
    }

    pub fn needs_anchor(self) -> bool {
        matches!(self, Self::Before | Self::After)
    }
}

/// The first graph time a run had a step for the seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueJoin {
    pub run: String,
    pub at_unix_ms: u128,
}

/// One recorded move, in graph order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueMove {
    pub run: String,
    pub placement: Placement,
    pub anchor: Option<String>,
    pub at_unix_ms: u128,
}

/// Rebuild a seat's run order from its joins and its moves.
///
/// Joins apply in time order with the run as a stable tie breaker. Each move
/// applies after every join recorded at or before it. A move that names a run
/// whose join carries a later timestamp (a writer clock ahead of the mover's)
/// joins that run first, because the mover could only name a queued run. A
/// move that names a run with no join is ignored.
pub fn replay(joins: &[QueueJoin], moves: &[QueueMove]) -> Vec<String> {
    let mut joins = joins.to_vec();
    joins.sort_by(|left, right| {
        left.at_unix_ms
            .cmp(&right.at_unix_ms)
            .then_with(|| left.run.cmp(&right.run))
    });
    let known = joins
        .iter()
        .map(|join| join.run.as_str())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::<String>::with_capacity(joins.len());
    let mut pending = joins.iter().peekable();
    for movement in moves {
        while let Some(join) = pending.next_if(|join| join.at_unix_ms <= movement.at_unix_ms) {
            join_run(&mut order, &join.run);
        }
        let named = std::iter::once(movement.run.as_str()).chain(movement.anchor.as_deref());
        for run in named {
            if known.contains(run) {
                join_run(&mut order, run);
            }
        }
        apply_move(&mut order, movement);
    }
    for join in pending {
        join_run(&mut order, &join.run);
    }
    order
}

/// The runs outside `live` that the live runs' order can depend on: the anchors of the live
/// runs' moves, then the anchors of those anchors' own moves, and so on. Every join and move
/// changes only its own run's place, so a run that no kept run is ever placed against cannot
/// change the order of the others. [`replay`] gives the live runs the same order whether or
/// not it knows such a run's join.
pub fn order_anchors(moves: &[QueueMove], live: &BTreeSet<String>) -> BTreeSet<String> {
    let mut anchors = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for movement in moves {
        if let Some(anchor) = movement.anchor.as_deref() {
            anchors.entry(movement.run.as_str()).or_default().push(anchor);
        }
    }
    let mut needed = BTreeSet::new();
    let mut placed = live.iter().map(String::as_str).collect::<Vec<_>>();
    while let Some(run) = placed.pop() {
        for anchor in anchors.get(run).into_iter().flatten() {
            if !live.contains(*anchor) && needed.insert((*anchor).to_owned()) {
                placed.push(anchor);
            }
        }
    }
    needed
}

fn join_run(order: &mut Vec<String>, run: &str) {
    if !order.iter().any(|queued| queued == run) {
        order.push(run.to_owned());
    }
}

fn apply_move(order: &mut Vec<String>, movement: &QueueMove) {
    let Some(from) = order.iter().position(|run| *run == movement.run) else {
        return;
    };
    let anchor = if movement.placement.needs_anchor() {
        match movement.anchor.as_deref() {
            Some(anchor) if anchor != movement.run && order.iter().any(|run| run == anchor) => {
                Some(anchor)
            }
            _ => return,
        }
    } else {
        None
    };
    let run = order.remove(from);
    let to = match (movement.placement, anchor) {
        (Placement::Top, _) => 0,
        (Placement::Bottom, _) => order.len(),
        (Placement::Before, Some(anchor)) => order
            .iter()
            .position(|queued| queued == anchor)
            .expect("the anchor is queued"),
        (Placement::After, Some(anchor)) => {
            order
                .iter()
                .position(|queued| queued == anchor)
                .expect("the anchor is queued")
                + 1
        }
        (Placement::Before | Placement::After, None) => unreachable!("an anchor was checked"),
    };
    order.insert(to, run);
}

/// A current step as the seat selector sees it.
#[derive(Clone, Copy, Debug)]
pub struct SeatStep<'a> {
    pub subject: &'a str,
    pub run: &'a str,
    pub step: &'a str,
    pub status: &'a str,
    pub assignee: Option<&'a str>,
    pub claimant: Option<&'a str>,
    pub available_to: &'a [String],
    pub created_at_unix_ms: u128,
    /// The seat that held this step before a revision carried it, until that
    /// seat claims it again.
    pub carried_claimant: Option<&'a str>,
}

/// What one seat holds and what it takes next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeatSelection<'a> {
    /// Claimed, working, or verifying steps that occupy the seat.
    pub held: Vec<&'a str>,
    /// Ready steps assigned to the seat, in seat-queue order.
    pub ready: Vec<&'a str>,
}

impl<'a> SeatSelection<'a> {
    /// The seat's next work, whether or not it is free now.
    pub fn next(&self) -> Option<&'a str> {
        self.ready.first().copied()
    }

    /// The step to wake: the next work, only while the seat holds nothing.
    pub fn wake(&self) -> Option<&'a str> {
        if self.held.is_empty() {
            self.next()
        } else {
            None
        }
    }
}

/// The one next-work selector for an agent seat.
///
/// Ready steps sort by their run's queue position, then by creation time and
/// subject inside the run. A run without a ready step for the seat is passed
/// over, so a run waiting on a gate or another seat never blocks the runs after
/// it, and it becomes next again as soon as it has ready work. A nested step
/// whose listed parent step has the same selector is reached through that
/// parent and is not selected separately. Reordering never releases a held
/// step: `held` comes only from claims. A revision does not take a held step's
/// place either: a ready step whose claim by this seat a revision dropped comes
/// first until the seat claims it again.
pub fn select<'a>(agent: &str, steps: &[SeatStep<'a>], run_order: &[String]) -> SeatSelection<'a> {
    select_with_matching(agent, steps, run_order, ActorMatching::Aliases)
}

/// Select mutation-admission work using the exact persisted actor spellings.
pub fn select_literal<'a>(agent: &str, steps: &[SeatStep<'a>], run_order: &[String]) -> SeatSelection<'a> {
    select_with_matching(agent, steps, run_order, ActorMatching::Literal)
}

fn select_with_matching<'a>(
    agent: &str,
    steps: &[SeatStep<'a>],
    run_order: &[String],
    matching: ActorMatching,
) -> SeatSelection<'a> {
    let held = steps
        .iter()
        .filter(|step| {
            matches!(step.status, "claimed" | "working" | "verifying")
                && (matching.matches(step.claimant, agent) || matching.matches(step.assignee, agent))
                && !submitted_with_ready_nested_work_matching(step, steps, matching)
        })
        .map(|step| step.subject)
        .collect();
    let rank = |run: &str| {
        run_order
            .iter()
            .position(|queued| queued == run)
            .unwrap_or(usize::MAX)
    };
    let mut ready = steps
        .iter()
        .filter(|step| {
            step.status == "ready"
                && matching.matches(step.assignee, agent)
                && !steps.iter().any(|candidate| {
                    nests_under_matching(candidate, step, matching) && candidate.status != "verifying"
                })
        })
        .collect::<Vec<_>>();
    let waits_its_turn = |step: &SeatStep<'_>| !matching.matches(step.carried_claimant, agent);
    ready.sort_by(|left, right| {
        waits_its_turn(left)
            .cmp(&waits_its_turn(right))
            .then_with(|| rank(left.run).cmp(&rank(right.run)))
            .then_with(|| left.created_at_unix_ms.cmp(&right.created_at_unix_ms))
            .then_with(|| left.subject.cmp(right.subject))
    });
    SeatSelection {
        held,
        ready: ready.into_iter().map(|step| step.subject).collect(),
    }
}

/// True when a listed ancestor step in the same run has the same selector and
/// has not been submitted. A submitted parent's work message was consumed, so
/// its nested work is no longer reached through it.
pub fn reached_through_parent(step: &SeatStep<'_>, steps: &[SeatStep<'_>]) -> bool {
    steps
        .iter()
        .any(|candidate| nests_under(candidate, step) && candidate.status != "verifying")
}

/// True when `step` is nested under the listed `ancestor` in the same run and
/// both have the same selector.
pub fn nests_under(ancestor: &SeatStep<'_>, step: &SeatStep<'_>) -> bool {
    nests_under_matching(ancestor, step, ActorMatching::Aliases)
}

/// Compare nested selectors for mutation admission without merging actor aliases.
pub fn nests_under_literal(ancestor: &SeatStep<'_>, step: &SeatStep<'_>) -> bool {
    nests_under_matching(ancestor, step, ActorMatching::Literal)
}

fn nests_under_matching(ancestor: &SeatStep<'_>, step: &SeatStep<'_>, matching: ActorMatching) -> bool {
    ancestor.run == step.run
        && matching.same(ancestor.assignee, step.assignee)
        && ancestor.available_to.len() == step.available_to.len()
        && ancestor.available_to.iter().zip(step.available_to).all(|(left, right)| {
            matching.matches(Some(left), right)
        })
        && ancestor.step.len() < step.step.len()
        && step.step.starts_with(ancestor.step)
        && step.step.as_bytes().get(ancestor.step.len()) == Some(&b'/')
}

/// An agent can submit a parent while its own nested steps are still ready.
/// The parent then waits in `verifying` for work that nothing will prompt: the
/// inherited parent message was consumed, and the seat looks occupied. Such a
/// parent frees the seat so the ready nested step gets its own wake.
pub fn submitted_with_ready_nested_work(parent: &SeatStep<'_>, steps: &[SeatStep<'_>]) -> bool {
    submitted_with_ready_nested_work_matching(parent, steps, ActorMatching::Aliases)
}

fn submitted_with_ready_nested_work_matching(
    parent: &SeatStep<'_>,
    steps: &[SeatStep<'_>],
    matching: ActorMatching,
) -> bool {
    parent.status == "verifying"
        && steps
            .iter()
            .any(|step| step.status == "ready" && nests_under_matching(parent, step, matching))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(run: &str, at: u128) -> QueueJoin {
        QueueJoin {
            run: run.into(),
            at_unix_ms: at,
        }
    }

    fn moved(run: &str, placement: Placement, anchor: Option<&str>, at: u128) -> QueueMove {
        QueueMove {
            run: run.into(),
            placement,
            anchor: anchor.map(str::to_owned),
            at_unix_ms: at,
        }
    }

    #[test]
    fn joins_keep_start_order_and_later_runs_join_the_end() {
        let joins = [join("c", 30), join("a", 10), join("b", 20)];
        assert_eq!(replay(&joins, &[]), ["a", "b", "c"]);
        let moves = [moved("a", Placement::Bottom, None, 25)];
        assert_eq!(
            replay(&joins, &moves),
            ["b", "a", "c"],
            "a run that joins after a move still joins the end"
        );
    }

    #[test]
    fn each_placement_moves_one_run() {
        let joins = [join("a", 1), join("b", 2), join("c", 3), join("d", 4)];
        let cases = [
            (moved("c", Placement::Top, None, 5), ["c", "a", "b", "d"]),
            (moved("a", Placement::Bottom, None, 5), ["b", "c", "d", "a"]),
            (
                moved("d", Placement::Before, Some("b"), 5),
                ["a", "d", "b", "c"],
            ),
            (
                moved("a", Placement::After, Some("c"), 5),
                ["b", "c", "a", "d"],
            ),
        ];
        for (movement, expected) in cases {
            assert_eq!(replay(&joins, &[movement]), expected);
        }
    }

    #[test]
    fn a_move_joins_named_runs_whose_writer_clock_ran_ahead() {
        let joins = [join("a", 10), join("b", 50)];
        let moves = [moved("b", Placement::Top, None, 20)];
        assert_eq!(replay(&joins, &moves), ["b", "a"]);
    }

    #[test]
    fn unusable_moves_are_ignored() {
        let joins = [join("a", 1), join("b", 2)];
        let moves = [
            moved("missing", Placement::Top, None, 3),
            moved("b", Placement::Before, Some("missing"), 4),
            moved("b", Placement::After, None, 5),
            moved("b", Placement::Before, Some("b"), 6),
        ];
        assert_eq!(replay(&joins, &moves), ["a", "b"]);
    }

    #[test]
    fn order_anchors_follow_anchors_from_the_live_runs_only() {
        let live = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let moves = [
            moved("ended-1", Placement::Top, None, 1),
            moved("ended-2", Placement::After, Some("a"), 2),
            moved("a", Placement::Before, Some("ended-3"), 3),
            moved("ended-3", Placement::After, Some("ended-4"), 4),
            moved("ended-4", Placement::Before, Some("b"), 5),
            moved("ended-5", Placement::Before, Some("ended-6"), 6),
        ];
        assert_eq!(
            order_anchors(&moves, &live),
            BTreeSet::from(["ended-3".to_owned(), "ended-4".to_owned()])
        );
    }

    #[test]
    fn the_live_order_is_the_same_without_the_runs_no_live_run_is_placed_against() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        let runs = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let placements = [
            Placement::Top,
            Placement::Bottom,
            Placement::Before,
            Placement::After,
        ];
        for case in 0..5_000 {
            let mut joins = Vec::new();
            for run in runs {
                if next(5) != 0 {
                    joins.push(join(run, u128::from(next(40))));
                }
            }
            let mut at = 0;
            let moves = (0..next(14))
                .map(|_| {
                    at += u128::from(next(6));
                    let run = runs[next(8) as usize];
                    let anchor = (next(4) != 0).then(|| runs[next(8) as usize]);
                    moved(run, placements[next(4) as usize], anchor, at)
                })
                .collect::<Vec<_>>();
            let live = joins
                .iter()
                .filter(|_| next(2) == 0)
                .map(|join| join.run.clone())
                .collect::<BTreeSet<_>>();
            let live_order = |joins: &[QueueJoin]| {
                replay(joins, &moves)
                    .into_iter()
                    .filter(|run| live.contains(run))
                    .collect::<Vec<_>>()
            };
            let anchors = order_anchors(&moves, &live);
            let kept = joins
                .iter()
                .filter(|join| live.contains(&join.run) || anchors.contains(&join.run))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                live_order(&kept),
                live_order(&joins),
                "case {case}: joins {joins:?}, moves {moves:?}, live {live:?}"
            );
        }
    }

    #[test]
    fn mixed_actor_spellings_share_ready_held_carried_and_nested_selection() {
        let available = vec!["node/worker".to_owned()];
        let canonical_available = vec!["agent/node/worker".to_owned()];
        let first = SeatStep {
            subject: "step-run/first/build", run: "mission-run/first", step: "build",
            status: "ready", assignee: Some("node/worker"), claimant: None,
            available_to: &available, created_at_unix_ms: 10, carried_claimant: None,
        };
        let second = SeatStep {
            subject: "step-run/second/build", run: "mission-run/second",
            assignee: Some("agent/node/worker"), created_at_unix_ms: 20, ..first
        };
        let order = vec![first.run.to_owned(), second.run.to_owned()];
        assert_eq!(
            select_literal("agent/node/worker", &[second, first], &order).ready,
            [second.subject],
        );
        for actor in ["node/worker", "agent/node/worker"] {
            assert_eq!(select(actor, &[second, first], &order).ready, [first.subject, second.subject]);
            let carried = SeatStep { carried_claimant: Some("node/worker"), ..second };
            assert_eq!(select(actor, &[first, carried], &order).next(), Some(second.subject));
            let held = SeatStep { status: "working", claimant: Some("node/worker"), ..second };
            assert_eq!(select(actor, &[first, held], &order).held, [second.subject]);
        }
        let nested = SeatStep {
            subject: "step-run/first/build/check", step: "build/check",
            assignee: Some("agent/node/worker"), available_to: &canonical_available, ..first
        };
        assert!(reached_through_parent(&nested, &[first, nested]));
        assert_eq!(
            select_literal("agent/node/worker", &[first, nested], &order).ready,
            [nested.subject],
            "an alias-assigned parent must not hide literally assigned nested work",
        );
        let submitted = SeatStep { status: "verifying", ..first };
        assert!(submitted_with_ready_nested_work(&submitted, &[submitted, nested]));
        for other in ["daemon/node/worker", "person/test", "host/node/worker"] {
            assert!(select(other, &[first, second], &order).ready.is_empty());
            assert!(!nests_under(&SeatStep { assignee: Some(other), ..first }, &nested));
        }
    }

    #[test]
    fn nested_steps_are_reached_through_their_listed_parent_only() {
        let none = Vec::new();
        let step = |subject: &'static str, path: &'static str| SeatStep {
            subject,
            run: "mission-run/one",
            step: path,
            status: "ready",
            assignee: Some("agent/worker"),
            claimant: None,
            available_to: &none,
            created_at_unix_ms: 1,
            carried_claimant: None,
        };
        let steps = [
            step("step-run/one/build", "build"),
            step("step-run/one/build/check", "build/check"),
            step("step-run/one/builder", "builder"),
        ];
        assert!(!reached_through_parent(&steps[0], &steps));
        assert!(reached_through_parent(&steps[1], &steps));
        assert!(
            !reached_through_parent(&steps[2], &steps),
            "a sibling that shares a name prefix is not nested"
        );
        let selection = select("agent/worker", &steps, &["mission-run/one".into()]);
        assert_eq!(
            selection.ready,
            ["step-run/one/build", "step-run/one/builder"]
        );
    }

    #[test]
    fn a_submitted_parent_frees_the_seat_and_its_nested_step_keeps_queue_order() {
        let none = Vec::new();
        let step =
            |subject: &'static str, run: &'static str, path: &'static str, status| SeatStep {
                subject,
                run,
                step: path,
                status,
                assignee: Some("agent/worker"),
                claimant: None,
                available_to: &none,
                created_at_unix_ms: 1,
                carried_claimant: None,
            };
        let order = ["mission-run/one".to_owned(), "mission-run/two".to_owned()];
        let mut steps = [
            step("step-run/one/build", "mission-run/one", "build", "ready"),
            step("step-run/two/fix", "mission-run/two", "fix", "verifying"),
            step(
                "step-run/two/fix/check",
                "mission-run/two",
                "fix/check",
                "ready",
            ),
        ];
        let selection = select("agent/worker", &steps, &order);
        assert!(selection.held.is_empty());
        assert_eq!(
            selection.ready,
            ["step-run/one/build", "step-run/two/fix/check"],
            "the nested step is selected as its own run's ready work"
        );
        assert_eq!(selection.wake(), Some("step-run/one/build"));

        steps[2].status = "completed";
        let selection = select("agent/worker", &steps, &order);
        assert_eq!(
            selection.held,
            ["step-run/two/fix"],
            "a submitted parent with no ready nested step still holds the seat"
        );
        assert_eq!(selection.wake(), None);
    }
}
