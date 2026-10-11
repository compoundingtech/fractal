# Hetz conditions evaluator measurement

Operations owns execution, deployment and approval. The builder builds and tests on
hetz2, supplies the exact CI release artifact and analyzes the resulting finite
reads. Do not compile, load-test or run a synthetic flapping workload on hetz.
No measurements have been made on hetz for this replacement PR yet.

## Epochs and duration

Freeze unrelated daemon deployments, configuration changes and planned maintenance
for each window. Keep normal workload running. Record any account reset, fleet
join, checkpoint, backup, collector outage or unusual workload; a affected window
cannot establish the evaluator's steady cost. Keep and report every attempted
window, including disturbed ones. Do not silently retry a bad result.

1. **Baseline:** exact current-main release without conditions. After five minutes
   of warm-up, hold the epoch for 30 minutes: six consecutive five-minute blocks.
2. **Candidate without declarations:** Operations deploys the exact disk-first CI
   archive through its normal forward path. After five minutes of warm-up, hold
   another 30-minute epoch. This measures evaluator scheduling, discovery and
   maintenance when no conditions apply.
3. **Candidate with disk conditions:** after every fleet daemon has been upgraded
   (before any new declaration), Operations publishes the intended native root
   filesystem free-space condition and this replica's database-plus-WAL size
   condition. Keep real thresholds supplied by Operations and normal production
   workload. After five minutes of warm-up, hold another 30-minute epoch. The same
   candidate SHA and normal workload must remain in place. This measures 60 live
   evaluations, including the native disk sampler and database metadata reads.

Requested total: **three 35-minute epochs (105 minutes), plus forward deployment
and the all-member compatibility gate.** Stage 3 can occur later if rollout takes
longer. Operations must record deployment and publication times and exact SHAs.
Do not declare conditions against a mixed fleet. Do not induce a breach on hetz;
transition and collector-retry behavior is tested on hetz2.

## Finite reads

At each epoch's warm-up boundary, and then at minutes 5, 10, 15, 20, 25 and 30
(seven samples per epoch), collect:

- `st doctor --performance --json`, with a ten-second client timeout;
- one read of `GET /v1/client/request-latency` through the existing local socket,
  with a ten-second client timeout (for example `curl --max-time 10 --unix-socket
  "$CONDITIONS_SOCKET" http://localhost/v1/client/request-latency`);
- a read of the daemon's `/proc/PID/stat` and `/proc/PID/io`, timestamped with a
  monotonic clock, plus `CLK_TCK`, PID and process start time; record the exact
  executable SHA at the first and last sample.

Operations supplies the existing socket and daemon PID. Use exactly the same
read pattern in every epoch. Keep complete JSON and exit statuses. These are
**21 performance reads, 21 latency reads and 21 pairs of kernel counter reads**
across the three epochs, not a new recurring monitor. A PID/start-time change or
a missing sample invalidates that block; retain its results. Record the local
read duration so measurement overhead is visible.

## Numbers and decision

For each complete five-minute block, report process CPU cores as
`delta(utime + stime) / CLK_TCK / elapsed_seconds`, and the cumulative kernel
read/write byte deltas. Report the daemon CPU `5m` window alongside that independent
CPU reading. Report the `sql-statement` SLO window's count, p99 and max, and the
read/write/writer-hold transaction windows. Use counts from the complete SLO
window; `/v1/performance` ranks only the top 20 labels and cannot establish total
SQL work. Keep all those ranked rows as attribution evidence. Candidate evaluator
work is charged under the existing `condition/evaluate` task label; absence from
the top 20 does not mean zero cost. The disk worker's cost is included in whole
process CPU even when it runs on another thread.

Publish all six block values per epoch, their median and range, the baseline to
candidate differences and confidence intervals. Predeclare resolution from the
baseline variation and CPU-counter quantization before opening candidate results.
State those numbers, rather than rounding a nonzero change to zero. The requested
claim, "nothing measurable", requires the candidate difference to lie within that
measured resolution for **both CPU and SQL**; a wide interval is inconclusive,
not a pass. A significant extra SQL rate, CPU increase, lost sample, regression in
transaction max/p99 or writer hold above 100 ms keeps the gate closed. No threshold
is loosened after the candidate results arrive. If ordinary workload makes six
blocks inconclusive, Operations decides whether to run a separately documented
longer comparison; the builder does not start more reads on hetz.

The steady-state evaluator performs bounded SQLite reads and cache writes. It
cannot honestly be described as executing zero SQL. This protocol distinguishes
nonzero work below measured resolution from a measurable regression. Local cost
fixtures and CI results are reported separately from production observations.

## Storage evidence

On hetz2, generate a finite long flapping history and export it to a fake OTLP
collector. Once the reviewed local trim API is available, apply it without a
checkpoint quorum on a scratch store and report transition counts and serialized
bytes before/after. Verify restart, head reconstruction, late replay, remote
replication and pending notification delivery after trim. Report the compact
permanent deleted-ID/tombstone set, normal durable messages, pending outbox,
signed transport containers and replicated daily write volume separately.

The local trim dependency is absent on pinned main 8f4f47e0b. The condition-kind
adapter and these trim controls therefore remain a merge gate. A four-state
logical cache is only a bounded stand-in for review and does not demonstrate
physical byte reclamation. A stable-checkpoint rule cannot substitute for the
required local, quorum-free retention.
