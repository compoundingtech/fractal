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
/// How soon a push that failed is tried again, even with no new commit.
const SEAT_FRESHNESS_RETRY: Duration = Duration::from_secs(60);

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
        let seats = received
            .seats
            .into_iter()
            .filter(|seat| hosted.contains(&seat.agent))
            .map(|seat| (seat.agent, seat.incarnation_id, u128::from(seat.heard_at_unix_ms)))
            .collect::<Vec<_>>();
        let recorded = seats.len();
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

/// What this node last pushed to each member, and to which of its starts.
#[derive(Default)]
pub struct SeatFreshnessPushed {
    members: BTreeMap<String, (String, BTreeMap<String, (String, u128)>)>,
}

/// One round's outcome: the seats pushed to each member, and the members a push failed for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeatFreshnessRound {
    pub pushed: BTreeMap<String, usize>,
    pub failed: BTreeSet<String>,
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
            .filter(|(pushed_start, _)| pushed_start == start)
            .map(|(_, seats)| seats);
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
            let mut result = Ok(Some(SeatFreshnessAck::default()));
            for chunk in seats.chunks(crate::peer::MAX_SEAT_FRESHNESS_SEATS) {
                let push = SeatFreshnessPush { seats: chunk.to_vec() };
                result = relay.push_seat_freshness(&member, &push, timeout).await;
                if !matches!(result, Ok(Some(_))) {
                    break;
                }
            }
            (member, seats, result)
        });
    }
    let mut round = SeatFreshnessRound::default();
    for (member, seats, result) in futures_util::future::join_all(sends).await {
        match result {
            Ok(Some(_)) => {
                let start = starts[&member].clone();
                let entry = pushed.members.entry(member.clone()).or_insert_with(|| (start.clone(), BTreeMap::new()));
                if entry.0 != start {
                    *entry = (start, BTreeMap::new());
                }
                for seat in &seats {
                    entry.1.insert(seat.agent.clone(), (seat.incarnation_id.clone(), u128::from(seat.heard_at_unix_ms)));
                }
                round.pushed.insert(member, seats.len());
            }
            // Its advertisement was ahead of its build, or no route reaches it directly.
            Ok(None) => {}
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

/// Push this node's seats' freshness to the other members whenever it commits, in rounds at
/// least [`SEAT_FRESHNESS_COALESCE`] apart; a round with a failed push is retried after
/// [`SEAT_FRESHNESS_RETRY`] even with no new commit.
pub fn start_seat_freshness_push(state: &AppState) {
    let Some(relay) = state.client_relay.clone() else {
        return;
    };
    let store = state.store.clone();
    let node = state.node.clone();
    let wake = Arc::new(tokio::sync::Notify::new());
    let observer = {
        let wake = Arc::clone(&wake);
        store.observe_commits(move |_| wake.notify_one())
    };
    tokio::spawn(crate::task_cpu::account("seat_freshness.push", async move {
        let _observer = observer;
        let mut pushed = SeatFreshnessPushed::default();
        let mut retry = false;
        loop {
            if retry {
                tokio::select! {
                    () = wake.notified() => {}
                    () = tokio::time::sleep(SEAT_FRESHNESS_RETRY) => {}
                }
            } else {
                wake.notified().await;
            }
            tokio::time::sleep(SEAT_FRESHNESS_COALESCE).await;
            let members = seat_freshness_members(&relay);
            match push_seat_freshness(&store, &relay, &node, &members, &mut pushed, SEAT_FRESHNESS_ANSWER).await {
                Ok(round) => retry = !round.failed.is_empty(),
                Err(error) => {
                    eprintln!("st3: seat freshness push failed: {error:#}");
                    retry = true;
                }
            }
        }
    }));
}
