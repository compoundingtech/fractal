//! Seat freshness across members (contract C3 of the claims-store breakup): the seat's host
//! pushes when it heard each of its seats to the other members, and each member keeps what its
//! peers pushed, so a card read never waits on a peer.
//!
//! The push is reactive. A commit on this node wakes the pusher; it reads its seats' newest
//! local reports in one statement and sends each capable member only what changed since its last
//! push there, all members at once. Nothing is sent while nothing changes.

use super::*;
use crate::api::client_v0::validation;
use crate::peer::{ClientRelay, ReceivedSeatFreshness, SEAT_FRESHNESS_FEATURE, SeatFreshnessAck, SeatFreshnessPush, SeatHeard};
use crate::store::RelayedFreshness;

/// After a commit wakes the pusher, how long it gathers further commits before one round.
pub const SEAT_FRESHNESS_COALESCE: Duration = Duration::from_secs(10);
/// How long one member may take to accept a push before this round moves on without it.
pub const SEAT_FRESHNESS_ANSWER: Duration = Duration::from_secs(10);
/// How soon a push that failed, or left a seat owed, is tried again, even with no new commit.
const SEAT_FRESHNESS_RETRY: Duration = Duration::from_secs(60);
/// How far ahead of this node's clock a pushed time may be.
pub const SEAT_FRESHNESS_CLOCK_SKEW: Duration = Duration::from_secs(60);
/// The longest wait between retries while seats stay owed.
const SEAT_FRESHNESS_LONGEST_RETRY: Duration = Duration::from_secs(15 * 60);

/// Record a peer's push, through its replication worker, which authenticated `from`. Only seats
/// whose declaration places them on `from` are recorded: a member speaks for its own seats.
pub async fn seat_freshness(
    State(state): State<AppState>,
    Json(received): Json<ReceivedSeatFreshness>,
) -> Result<Json<SeatFreshnessAck>, ApiError> {
    SeatFreshnessPush { seats: received.seats.clone() }
        .validate()
        .map_err(|error| validation(format!("{error:#}")))?;
    let store = state.store.clone();
    blocking_store(move || {
        let hosted = store
            .desired_subjects()?
            .into_iter()
            .filter(|desired| desired.member.as_ref().is_some_and(|member| member.host == received.from))
            .map(|desired| desired.subject)
            .collect::<BTreeSet<_>>();
        // A host's clock may run ahead: a time past this node's now plus a minute is taken as
        // that, so a skewed clock cannot keep a seat looking fresh.
        let latest = smallclaims::store::now_ms() + SEAT_FRESHNESS_CLOCK_SKEW.as_millis();
        let seats = received
            .seats
            .into_iter()
            .filter(|seat| hosted.contains(&seat.agent))
            .map(|seat| (seat.agent, seat.incarnation_id, u128::from(seat.heard_at_unix_ms).min(latest)))
            .collect::<Vec<_>>();
        let recorded = seats.iter().map(|(agent, _, _)| agent.clone()).collect();
        store.extension::<RelayedFreshness>().record(seats);
        Ok(SeatFreshnessAck { recorded })
    })
    .await
    .map(Json)
}

/// The id of `member`'s own newest `daemon.started` when it advertises seat freshness: a member
/// that restarted holds nothing it was pushed, so a new start gets every seat.
pub fn seat_freshness_start(store: &Store, member: &str) -> anyhow::Result<Option<String>> {
    Ok(store
        .latest_claim(&format!("daemon/{member}"), Some("daemon.started"))?
        .filter(|claim| {
            claim.origin == member
                && claim.body.pointer(&format!("/fields/features/{SEAT_FRESHNESS_FEATURE}")).and_then(Value::as_u64) == Some(1)
        })
        .map(|claim| claim.id))
}

/// Whether every other member can hear this node's seats: each advertises the route, and this
/// node is not a dial-out member, which no other member can dial back. Re-stamps then stay local.
///
/// An advertisement is not delivery: a member counts only once a push from this node reached it
/// and it answered (`reached`, kept across rounds; a member with no direct route, such as a
/// dial-out one, or with a worker that lacks the route, answers nothing and does not count).
pub fn fleet_hears_seats(
    store: &Store,
    node: &str,
    members: &[String],
    dial_out: bool,
    reached: &BTreeMap<String, bool>,
) -> anyhow::Result<bool> {
    if dial_out {
        return Ok(false);
    }
    for member in members.iter().filter(|member| member.as_str() != node) {
        if seat_freshness_start(store, member)?.is_none() || reached.get(member) != Some(&true) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Each seat's incarnation and the time it was heard, by seat.
type HeardBySeat = BTreeMap<String, (String, u128)>;

/// What this node last pushed to one member, and to which of its starts.
struct PushedTo {
    start: String,
    seats: HeardBySeat,
}

/// What this node last pushed to each member.
#[derive(Default)]
pub struct SeatFreshnessPushed {
    members: BTreeMap<String, PushedTo>,
}

/// One round's outcome: the seats each member recorded, the members a push failed for, and those
/// that left a sent seat unrecorded (its declaration had not reached them yet).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeatFreshnessRound {
    pub pushed: BTreeMap<String, usize>,
    pub failed: BTreeSet<String>,
    pub owed: BTreeSet<String>,
    /// Members a push reached and that answered it, whatever they recorded.
    pub reached: BTreeSet<String>,
    /// Members no push can reach: no direct route (a dial-out member), or a worker without
    /// the route.
    pub unreachable: BTreeSet<String>,
}

/// One round: read this node's seats' newest local reports, and push each capable member what
/// changed since the last push to that start of it, all members at once. A member whose start
/// changed gets every seat. A failed push stays owed for the next round.
pub async fn push_seat_freshness(
    store: &Arc<Store>,
    relay: &ClientRelay,
    node: &str,
    members: &[String],
    pushed: &mut SeatFreshnessPushed,
    timeout: Duration,
) -> anyhow::Result<SeatFreshnessRound> {
    let (starts, newest) = {
        let store = Arc::clone(store);
        let node = node.to_owned();
        let members = members.to_vec();
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut starts = BTreeMap::new();
            for member in members.iter().filter(|member| member.as_str() != node) {
                if let Some(start) = seat_freshness_start(&store, member)? {
                    starts.insert(member.clone(), start);
                }
            }
            let seats = store
                .desired_subjects()?
                .into_iter()
                .filter(|desired| {
                    desired.subject.starts_with("agent/")
                        && desired.member.as_ref().is_some_and(|member| member.host == node)
                })
                .map(|desired| desired.subject)
                .collect::<Vec<_>>();
            let newest = store.local_harness_newest_many(&seats.iter().map(String::as_str).collect::<Vec<_>>())?;
            Ok((starts, newest))
        })
        .await??
    };
    pushed.members.retain(|member, _| starts.contains_key(member));
    let mut sends = Vec::new();
    for (member, start) in &starts {
        let previous = pushed
            .members
            .get(member)
            .filter(|to| to.start == *start)
            .map(|to| &to.seats);
        let seats = newest
            .iter()
            .filter(|(agent, heard)| previous.and_then(|seats| seats.get(*agent)) != Some(*heard))
            .map(|(agent, (incarnation, at))| SeatHeard {
                agent: agent.clone(),
                incarnation_id: incarnation.clone(),
                heard_at_unix_ms: u64::try_from(*at).unwrap_or(u64::MAX),
            })
            .collect::<Vec<_>>();
        if seats.is_empty() {
            continue;
        }
        let member = member.clone();
        sends.push(async move {
            let mut recorded = BTreeSet::new();
            let mut result = Ok(Some(()));
            for chunk in seats.chunks(crate::peer::MAX_SEAT_FRESHNESS_SEATS) {
                let push = SeatFreshnessPush { seats: chunk.to_vec() };
                match relay.push_seat_freshness(&member, &push, timeout).await {
                    Ok(Some(ack)) => recorded.extend(ack.recorded),
                    other => {
                        result = other.map(|ack| ack.map(|_| ()));
                        break;
                    }
                }
            }
            // Only what the member recorded is delivered; the rest stays owed for the next round.
            let sent = seats.len();
            let seats = seats.into_iter().filter(|seat| recorded.contains(&seat.agent)).collect::<Vec<_>>();
            let owed = result.is_ok() && seats.len() < sent;
            (member, seats, result, owed)
        });
    }
    let mut round = SeatFreshnessRound::default();
    for (member, seats, result, owed) in futures_util::future::join_all(sends).await {
        if owed {
            round.owed.insert(member.clone());
        }
        match result {
            Ok(Some(_)) => {
                let start = starts[&member].clone();
                let to = pushed
                    .members
                    .entry(member.clone())
                    .or_insert_with(|| PushedTo { start: start.clone(), seats: BTreeMap::new() });
                if to.start != start {
                    *to = PushedTo { start, seats: BTreeMap::new() };
                }
                for seat in &seats {
                    to.seats.insert(seat.agent.clone(), (seat.incarnation_id.clone(), u128::from(seat.heard_at_unix_ms)));
                }
                round.reached.insert(member.clone());
                if !seats.is_empty() {
                    round.pushed.insert(member, seats.len());
                }
            }
            // Its advertisement was ahead of its build, or no route reaches it directly.
            Ok(None) => {
                round.unreachable.insert(member);
            }
            Err(error) => {
                eprintln!("st3: seat freshness to {member}: {error:#}");
                round.failed.insert(member);
            }
        }
    }
    Ok(round)
}

/// The members this node pushes to: current members, and the configured peers of a fleet that
/// predates membership.
pub fn seat_freshness_members(relay: &ClientRelay) -> Vec<String> {
    relay
        .fleet_view()
        .members
        .iter()
        .filter(|member| member.state == "current")
        .map(|member| member.name.clone())
        .chain(relay.peers.iter().map(|peer| peer.name.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// How the pusher paces itself: the coalescing after a commit, the bound on each member's
/// answer, and the retry of a failed or owed push, which doubles up to its longest while seats
/// stay owed so a member that never records one is not asked every minute for ever.
#[derive(Clone, Copy, Debug)]
pub struct SeatFreshnessPace {
    pub coalesce: Duration,
    pub answer: Duration,
    pub retry: Duration,
    pub longest_retry: Duration,
}

impl Default for SeatFreshnessPace {
    fn default() -> Self {
        Self {
            coalesce: SEAT_FRESHNESS_COALESCE,
            answer: SEAT_FRESHNESS_ANSWER,
            retry: SEAT_FRESHNESS_RETRY,
            longest_retry: SEAT_FRESHNESS_LONGEST_RETRY,
        }
    }
}

/// Push this node's seats' freshness to the other members whenever it commits, in rounds at
/// least [`SEAT_FRESHNESS_COALESCE`] apart. A round that failed a member, or left a seat owed, is
/// retried after [`SEAT_FRESHNESS_RETRY`] even with no new commit, then less often.
pub fn start_seat_freshness_push(state: &AppState) {
    let Some(relay) = state.client_relay.clone() else {
        // Not in a fleet: no other member reads this node's re-stamps.
        if let Err(error) = state.store.set_fleet_hears_seats(true) {
            eprintln!("st3: seat freshness: {error:#}");
        }
        return;
    };
    spawn_seat_freshness_push(state.store.clone(), relay, state.node.clone(), SeatFreshnessPace::default());
}

/// The pusher [`start_seat_freshness_push`] starts, at `pace`.
pub fn spawn_seat_freshness_push(store: Arc<Store>, relay: ClientRelay, node: String, pace: SeatFreshnessPace) {
    let wake = Arc::new(tokio::sync::Notify::new());
    let observer = {
        let wake = Arc::clone(&wake);
        store.observe_commits(move |_| wake.notify_one())
    };
    tokio::spawn(crate::task_cpu::account("seat_freshness.push", async move {
        let _observer = observer;
        let mut pushed = SeatFreshnessPushed::default();
        // Whether this node's last push to each member reached it.
        let mut reached = BTreeMap::<String, bool>::new();
        let mut retry: Option<Duration> = None;
        // The first round needs no commit: what this node heard before it started is owed.
        let mut first = true;
        loop {
            match retry {
                _ if first => first = false,
                Some(after) => {
                    tokio::select! {
                        () = wake.notified() => {}
                        () = tokio::time::sleep(after) => {}
                    }
                }
                None => wake.notified().await,
            }
            tokio::time::sleep(pace.coalesce).await;
            let members = seat_freshness_members(&relay);
            let again = match push_seat_freshness(&store, &relay, &node, &members, &mut pushed, pace.answer).await {
                Ok(round) => {
                    for member in &round.reached {
                        reached.insert(member.clone(), true);
                    }
                    // A member no push reaches, or whose push failed, cannot hear this node's seats
                    // until a push reaches it again.
                    for member in round.unreachable.iter().chain(&round.failed) {
                        reached.insert(member.clone(), false);
                    }
                    !round.failed.is_empty() || !round.owed.is_empty()
                }
                Err(error) => {
                    eprintln!("st3: seat freshness push failed: {error:#}");
                    true
                }
            };
            // Membership, advertisements and starts arrive as commits too, so each round settles,
            // from what its pushes reached, whether every other member can hear this node's seats.
            let dial_out = relay.fleet_view().members.iter().any(|member| member.name == node && member.mode == "dial-out");
            let flag = {
                let (store, node, members, reached) = (Arc::clone(&store), node.clone(), members.clone(), reached.clone());
                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    store.set_fleet_hears_seats(fleet_hears_seats(&store, &node, &members, dial_out, &reached)?)
                })
                .await
            };
            if let Err(error) = flag.map_err(anyhow::Error::from).and_then(|result| result) {
                eprintln!("st3: seat freshness: {error:#}");
            }
            retry = again.then(|| retry.map_or(pace.retry, |after| (after * 2).min(pace.longest_retry)));
        }
    }));
}
