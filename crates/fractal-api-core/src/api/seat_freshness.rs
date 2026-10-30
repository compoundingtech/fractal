//! Seat freshness across members (contract C3 of the claims-store breakup): the seat's host
//! answers when it last heard each of its seats, and every member asks each capable host in the
//! background, so a card read never waits on a peer.

use super::*;
use crate::peer::{ClientRelay, SEAT_FRESHNESS_FEATURE, SeatFreshnessReply, SeatFreshnessRequest, SeatHeard};
use crate::store::RelayedFreshness;
use crate::api::client_v0::validation;

/// How often a member asks each capable host about its seats.
pub const SEAT_FRESHNESS_POLL: Duration = Duration::from_secs(30);
/// The longest one host may take to answer before the member moves on.
const SEAT_FRESHNESS_ANSWER: Duration = Duration::from_secs(10);

/// This node's answer for its replication worker: when it last heard each named seat, from its
/// local observation log. A seat it holds no local report of is left out.
pub async fn seat_freshness(
    State(state): State<AppState>,
    Json(request): Json<SeatFreshnessRequest>,
) -> Result<Json<SeatFreshnessReply>, ApiError> {
    request.validate().map_err(|error| validation(format!("{error:#}")))?;
    let store = state.store.clone();
    blocking_store(move || {
        let mut seats = Vec::new();
        for agent in request.agents {
            if let Some((incarnation_id, at)) = store.local_harness_newest(&agent)? {
                seats.push(SeatHeard {
                    agent,
                    incarnation_id,
                    heard_at_unix_ms: u64::try_from(at).unwrap_or(u64::MAX),
                });
            }
        }
        Ok(SeatFreshnessReply { seats })
    })
    .await
    .map(Json)
}

/// Whether `member`'s own newest `daemon.started` advertises the seat freshness route.
pub fn advertises_seat_freshness(store: &Store, member: &str) -> anyhow::Result<bool> {
    Ok(store
        .latest_claim(&format!("daemon/{member}"), Some("daemon.started"))?
        .is_some_and(|claim| {
            claim.origin == member
                && claim.body.pointer(&format!("/fields/features/{SEAT_FRESHNESS_FEATURE}")).and_then(Value::as_u64) == Some(1)
        }))
}

/// The seats each other member's declarations place on it, for the members that advertise the
/// seat freshness route.
pub fn seat_freshness_targets(store: &Store, node: &str, members: &[String]) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    let mut targets = BTreeMap::<String, Vec<String>>::new();
    for member in members.iter().filter(|member| member.as_str() != node) {
        if advertises_seat_freshness(store, member)? {
            targets.insert(member.clone(), Vec::new());
        }
    }
    for desired in store.desired_subjects()? {
        if let Some(seats) = desired.member.as_ref().and_then(|member| targets.get_mut(&member.host))
            && desired.subject.starts_with("agent/")
        {
            seats.push(desired.subject);
        }
    }
    targets.retain(|_, seats| !seats.is_empty());
    Ok(targets)
}

/// Whether every other member can ask this node about its seats: each advertises the route, and
/// this node is not a dial-out member, which no other member can dial. Re-stamps then stay local.
pub fn fleet_hears_seats(store: &Store, node: &str, members: &[String], dial_out: bool) -> anyhow::Result<bool> {
    if dial_out {
        return Ok(false);
    }
    for member in members.iter().filter(|member| member.as_str() != node) {
        if !advertises_seat_freshness(store, member)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Ask each capable host about its seats once, and keep what each answers. Record whether every
/// other member can ask this node in turn.
pub async fn poll_seat_freshness(store: &Arc<Store>, relay: &ClientRelay, node: &str) -> anyhow::Result<()> {
    let view = relay.fleet_view();
    let dial_out = view.members.iter().any(|member| member.name == node && member.mode == "dial-out");
    // Current members, and the configured peers of a fleet that predates membership.
    let members = view
        .members
        .iter()
        .filter(|member| member.state == "current")
        .map(|member| member.name.clone())
        .chain(relay.peers.iter().map(|peer| peer.name.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let targets = {
        let store = Arc::clone(store);
        let node = node.to_owned();
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            store.set_fleet_hears_seats(fleet_hears_seats(&store, &node, &members, dial_out)?)?;
            seat_freshness_targets(&store, &node, &members)
        })
        .await??
    };
    for (member, chunk) in targets
        .into_iter()
        .flat_map(|(member, seats)| {
            seats
                .chunks(crate::peer::MAX_SEAT_FRESHNESS_AGENTS)
                .map(|chunk| (member.clone(), chunk.to_vec()))
                .collect::<Vec<_>>()
        })
    {
        let request = SeatFreshnessRequest { agents: chunk };
        match tokio::time::timeout(SEAT_FRESHNESS_ANSWER, relay.seat_freshness(&member, &request)).await {
            Ok(Ok(Some(reply))) => store.extension::<RelayedFreshness>().record(
                reply
                    .seats
                    .into_iter()
                    .filter(|seat| request.agents.contains(&seat.agent))
                    .map(|seat| (seat.agent, seat.incarnation_id, u128::from(seat.heard_at_unix_ms))),
                smallclaims::store::now_ms(),
            ),
            // Its advertisement was ahead of its build, or no route reaches it directly.
            Ok(Ok(None)) => {}
            Ok(Err(error)) => eprintln!("st3: seat freshness from {member}: {error:#}"),
            Err(_) => eprintln!("st3: seat freshness from {member}: no answer in {SEAT_FRESHNESS_ANSWER:?}"),
        }
    }
    Ok(())
}

/// Ask every capable host about its seats every [`SEAT_FRESHNESS_POLL`], in the background.
pub fn start_seat_freshness_poll(state: &AppState) {
    let Some(relay) = state.client_relay.clone() else {
        // Not in a fleet: no other member reads this node's re-stamps.
        if let Err(error) = state.store.set_fleet_hears_seats(true) {
            eprintln!("st3: seat freshness: {error:#}");
        }
        return;
    };
    let store = state.store.clone();
    let node = state.node.clone();
    tokio::spawn(crate::task_cpu::account("seat_freshness.poll", async move {
        loop {
            if let Err(error) = poll_seat_freshness(&store, &relay, &node).await {
                eprintln!("st3: seat freshness poll failed: {error:#}");
            }
            tokio::time::sleep(SEAT_FRESHNESS_POLL).await;
        }
    }));
}
