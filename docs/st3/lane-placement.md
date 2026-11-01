# Placing missions that do not pin a host (design note)

Status: design, not built. It builds on seat lanes ([seat-lanes.md](seat-lanes.md)): declared
capacity, admission by fit, and step lanes.

## Today

A mission run's seats go where its declarations say: an agent's `host`, or the host the declaration
came from when it names none ([mission-graph-runtime.md](mission-graph-runtime.md)). Each host then
admits its own seats by fit. A run whose seats do not fit its host waits there, in line, with a
reason that names that host's free room, even when another member has room to spare.

## What changes

A mission that pins no host may run on any member with room for it. One member decides, once, when
the run starts; the run then lives where it was placed, as a run does today.

**Pinned or not.** A run is pinned when any agent it declares names a `host`, when it is started with
`--host`, or when a declaration needs this host: a workspace path outside a resource the run declares,
a terminal bound to a person here, a gate that runs a local command. Everything else is free to place.
Pinning stays the default for anything the planner cannot prove portable.

**What each member reports.** Each member's daemon publishes its room for lanes as a replicated
observation, `host.capacity-observed`: the room the formula gives it, what its lanes and the seats
outside lanes hold, the length of its line, and when it measured. It writes one when the numbers move
by more than a step (say 1G or a place in line) and at least every five minutes. The machines API
serves the newest one as `capacity: {state: "reported", …}` in place of today's `unknown`. An
observation older than ten minutes, or from a member that is not current in the membership, does not
count.

**Deciding.** The host the run's declaration came from places it. The run needs the sum of the lanes
its agents declare (the mission's capacity for those that declare none) plus the largest raise any of
its steps asks for. Candidates are the current members that advertise the feature, have lanes on, hold
a fresh observation, and have what the run's resources need (the repository a resource names, for
instance). Among those whose free room fits the run, it takes the one with the most memory left after
placing it; a tie goes to the originating host. When none fits, the run stays on its originating host
and waits there in line, and the reason names the member that came closest ("no member has room for
this run's 28G; the nearest is a build host with 22G free").

**Recording.** The placement is part of the run (`placed_on`, with the observations it read and why),
so every member shows the same answer and a later look at the run can explain it. Nothing moves a run
by itself afterwards.

**Moving.** A run that has waited thirty minutes where another member now has room is flagged with a
suggested move. A person, or an agent acting for one, moves it (`st missions move RUN --to HOST`):
its seats write their handoffs, stop, and start on the new host, in a workspace that host prepares
from the run's resources at the same revision. Automatic moves come later, once moves are proven by
hand.

## Mixed versions and failure

Only members that advertise the feature are candidates, so an older member is never chosen and never
asked. A member that leaves after a run was placed on it keeps the run in the membership's normal
way; the run is moved by hand or ends. Two members cannot place the same run: only the originating
host decides, and the run's creation is one claim. An observation that arrives late can make one
placement worse than it could have been, never wrong: the chosen host still admits the run's seats by
fit and holds them in line if the room is gone.

## Not in this note

Packing several runs together, preempting a running seat for a waiting one, and placing a run's seats
on different members.
