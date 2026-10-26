# Why a same-build pair stayed `projection_comparison_waiting`

Status: cause named, fix in the same change. Names below are invented: `alder`, `birch`, `cedar`,
`dune` are four members of one fleet that run the same build.

## What was seen

On a fleet of four members running one build, `st replication status` read
`projection_comparison_waiting: true` for every pair of one member (`alder`) in both directions,
for hours, while the other three pairs among `birch`, `cedar` and `dune` read `false`.

- `schema_digest` was identical on every peer entry, so the registries did not differ.
- `waiting_claims`, `unsealed` local batches and `unhealthy_projections` were all 0, and
  `last_error` was empty. One member had one `invalid` record, and its other pairs settled, so
  that record was not the cause.
- `graph_digest` differed between members whenever it was sampled. That is expected for nodes
  whose envelope sets differ at that instant, and it is not evidence of an impure projection:
  `st replication diff` against a peer, which compares the last signed response, reported
  `graph.equal: true` with `coverage: all-shared-projections`, and the `sync` block of the same
  status showed `peer_only_envelopes: 0`, `local_only_envelopes: 0` and a recent
  `graph_compared_at_unix_ms` with `diverged: false`.

So the comparison did complete, and it found the graphs equal, for the pairs that still read
waiting. The status was wrong, not the projections.

## Cause

`replication_status_sealed` decides `projection_comparison_waiting` by comparing two inventory
digests taken at different times:

- the peer's inventory digest, as the peer advertised it at its **last exchange** with this node
  (`meta.peer_projection_inventory:<peer>`), and
- this node's inventory digest **now**.

Only when they are equal does it call the pair comparable. A busy member receives envelopes from
all of its peers in bursts, and each burst moves its own inventory. Between two exchanges with one
peer the stored copy of that peer's inventory is therefore already older than the local one, so
the digests differ and the pair reads waiting. The exchange itself did the right thing: at the
exchange where both nodes held the same envelopes, `receive_replication_exchange` compared the
projections and recorded the result in memory (`graph_compared_at_unix_ms`, `diverged`). The
status never used that result.

How the evidence fits:

- Sampling the stored inventory digests on the affected member every three seconds showed three
  different values at once, one per peer, and the digests stayed unchanged through quiet
  stretches of ten seconds or more. At most one of them could equal the member's own digest, and
  nothing re-ran the comparison until the next burst. `alder` toward `birch`, `cedar` and `dune`
  was true because no more than one peer could be current at a time, and one pair read false at
  moments when its peer happened to be the freshest.
- The reverse direction (`birch` toward `alder`) is the same arithmetic seen from the other end:
  `alder` is a peer whose stored digest at `birch` is stale whenever `birch` has moved on since
  the last exchange.
- Whether a pair read false was therefore a race between exchange cadence and envelope arrival,
  not a property of the pair. Pairs whose members happened to exchange just after a quiet moment
  settled; pairs of the member that received the most bursts did not settle in any sample.
- Restarting a member did not clear it: the stored digests are rebuilt by the next exchange, which
  is stale by the same arithmetic.

It is a status and scheduling defect, not an impure projection: the same claims projected to the
same digests on both sides every time the comparison was made.

## Fix

- At each exchange where the comparison is made (inventories equal, nothing received, no
  signatures held, projection not deferred, same registry), remember the result for that peer:
  when, the registry digest, and the tables that differed.
- The status reports that result while it is at most five minutes old, the registry still matches
  and it found every comparable projection equal. A newer exchange whose inventories differ no
  longer undoes it. A comparison that found tables differing is not carried over a later exchange
  with different inventories, because the nodes may have converged since and a difference is only
  named from an exchange that can identify it; that pair reads waiting and says which tables the
  last aligned exchange found differing. A pair with no aligned exchange in five minutes reads
  waiting too.
- Local conditions that make a comparison premature still apply immediately: unsealed local
  batches, claims waiting for their kinds, a deferred projection, a different registry.
- The status exports what the reported comparison found (`projection_graphs_equal`) and when it was
  made (`projection_compared_at_unix_ms`). `st replication diff` and the peer lines of
  `st replication status` read those, not the equality of the peer's last digests with this node's
  live ones, and show the comparison's age ("graphs compared 3m ago").
- Every waiting pair now says why in `projection_comparison_waiting_reason`, and
  `st replication status` and `st replication diff` print it:
  - claims wait for a build that knows their kinds,
  - this node has unsealed local batches,
  - the projection is deferred,
  - the peer has not reported its registry, or runs a different one (both digests, shortened),
  - no exchange in the last 300 seconds found the same envelopes on both nodes, with the envelope
    counts the last exchange measured.

## Proof

- `crates/smallclaims/tests/projection_comparison.rs`:
  `a_comparison_made_at_an_aligned_exchange_survives_envelopes_that_arrive_afterwards` fails on the
  previous status (`projection_comparison_waiting: true` after an unrelated peer delivered one
  envelope) and passes now.
- `crates/smallclaims/src/store.rs`, `aligned_comparison_tests`:
  `a_pair_without_a_recent_aligned_exchange_names_why_it_waits` covers the reason text and the
  five-minute expiry.
- The two earlier tests in `projection_comparison.rs` still pass: an older advertised inventory
  stays pending until an aligned exchange, and unsealed local claims wait.

## After the deploy

Every same-build pair reads `projection_comparison_waiting: false` within one aligned exchange of
the daemon starting, and any pair that still waits names its reason in the same status output.
