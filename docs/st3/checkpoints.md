# Checkpoints and trimming

A fleet's replicated claim log only grows, because every member holds every claim. Most of what it
holds is old observation: a seat's state a minute ago, a diagnostic that has since repeated, a
reading that a later reading includes. Each member trims that history from its own copy, on its
own, without changing the graph or what any reader returns. Members do not agree on what to trim
and do not wait for each other. What one member deleted travels to its peers as retired claim
IDs, so no member sends it back.

This document explains what a trim drops and how it deletes it, with examples. It describes
`crates/smallclaims/src/store/checkpoint_trim.rs` (the daily cut, the trim and its budget),
`crates/smallclaims/src/store/checkpoint_staging.rs` (the capture and plan on disk),
`crates/fractal-schema/retention.toml` (the retention policy, which names the rules for
Fractal's own claim kinds; see [Retention](retention.md)) and
`crates/fractal-store/src/store/checkpoint_rules.rs` (the planner functions those rules name).
Operating notes are in [Fleet replication](replication.md#checkpoints). Read this before changing a
rule or the trim.

In one sentence: **a member drops a claim only when a later claim of the same slot, which it
keeps, replaces it for every reader.**

## The daily cut

A cut is named for a UTC day, such as `checkpoint/2026-10-01`. It covers every envelope the member
accepted before the start of that day, and it is due two days later, so every claim in it is at
least two days old. `newest_due_cut` in `checkpoint.rs` says which cut is due.

The daemon runs a trim step every ten minutes on a background thread that never holds the store's
writer for long. A step:

1. sweeps signature and verdict rows an earlier trim left (below) and rebuilds what an earlier
   trim deferred;
2. does nothing more while the member is catching up with a peer, since what it holds before the
   cut is still changing;
3. finishes a cut a crash or the daily budget left `trimming`;
4. trims the newest due cut, once. A cut moves only forward: one already `trimming`, `trimmed`
   or set aside is done, and a cut waits while an earlier one is still `trimming` or the day's
   budget is spent.

A cut is captured (the envelopes before the cut that its rules read, staged to disk), planned (the
rules and guards below mark what goes), checked, and trimmed (tombstones, then deletions).

```toml
[trim]
local = true                 # the default; false turns trimming off on this member
notify = "agent/alder.ops"   # optional: who gets a stopped trim or an answers difference
daily_claims = 100000        # optional: the most claims a trim deletes in a day
```

`local = false` stops the step after its sweep: the member trims nothing and keeps everything.
The `[checkpoint]` section still loads but is ignored; the daemon logs "[checkpoint] is ignored:
every member trims on its own ([trim])".

## What a rule may drop

A trim does not decide what is old. The rules in the retention policy do, and a rule exists only
for a kind that was audited against every reader of that kind. Everything else is kept.

A member drops only the kinds in `LOCAL_TRIM_KINDS`, each with a test that every reader answers the
same from the full and the trimmed log: today `observer.observed`, `harness.observed` and
`resource.observed`. A kind joins that list only with such a test. The capture also stages the
kinds judging those reads (`LOCAL_TRIM_CONTEXT_KINDS`: `runtime.observed` and `harness.diagnostic`,
which the status-history rule folds with a seat's observations), and keeps them as it keeps a
protected claim. The policy has rules for more kinds (the table below); until a kind joins the
list, its rule plans nothing that is deleted.

### Slots and witnesses

A **slot** is the part of a claim that one reader answer depends on, such as (subject, incarnation)
for a harness observation, or (subject, request) for a subscription deferral. Within a slot the
claims form a sequence in **canonical order**: accepted time, writer, batch sequence, position in
the batch, claim ID. The order never depends on when a claim reached a member, and removing a claim
or record never renumbers a survivor's position (see `claim_positions` in schema.md).

For each slot the rule names the claims to keep. Everything else in the slot is a candidate. A
candidate is dropped only if it has a **witness**: a later claim of the slot that is kept and sets
every field the candidate sets. Most readers fold a kind last-writer-wins, field by field, or take
the newest claim, so a later claim with every field makes the earlier one invisible. That holds
whatever arrives afterwards: a late claim that sorts before, between or after the two can change
what the fold ends with only by replacing the witness, which is also what it would have done to the
dropped claim. A kind that clears all of its fields on every update (a state transition) is
witnessed by any later kept claim of its slot.

The harness question rule compacts only redundant observations a day after resolution.
The newest observation and resolution remain indefinitely, so the compact record and receipt
continue to answer readers and prevent re-observation from reopening the question.

Every rule of every kind except `loop.state` and the harness question rule also keeps the newest
claim carrying each field, so a field set only once still has its last value.

### Guards

These claims are never dropped, whatever a rule says:

- a claim whose actor is a person (`person/*`);
- a claim of a kind that is not plain append or state transition, because those kinds' validation
  reads them (`Once`, `OncePerActor`, `OncePerAttempt`);
- a claim whose record is not valid: invalid, unknown or repaired, or the replacement of a repair;
- a claim a projection row references by ID: desired state, mission definitions and revisions,
  document bindings;
- a claim another claim cites as evidence, that a mission run pins as an input, or that a mission
  request names as its discovery (looked up in the citation cache, `local_trim_citations`, which
  each cut first catches up with the claims that arrived since the last);
- a claim held in two envelopes, and a claim sharing an operation with a kept claim, so an
  operation is wholly kept or wholly dropped;
- every claim in each writer's newest envelope before the cut, because sequence numbers and heads
  read it;
- a `harness.observed` claim without an `incarnation_id`, because an older reader falls back to
  arrival order for it.

An envelope is signed as a unit, so it is deleted only when every claim in it is dropped. When some
of its claims are kept, the envelope stays whole, with its payload and signatures, so peers can
still fetch and verify it; a dropped claim inside it loses only this member's own copies (its claim
row, record, signature and verdict rows; its stored position stays, as every position does), and
admitting the envelope again restores neither the claim nor its record.

The planner runs the witness check and these guards to a fixed point, since keeping one claim can
remove a witness another drop relied on. The drop set is a pure function of the captured claims
and the rules: it reads no clock and no local observation.

### The rules

The `[[rules]]` of `crates/fractal-schema/retention.toml` are the canonical list. Each renders one
line of the rules description (`rules_description` in `checkpoint_rules.rs`), followed by the
engine's own terms. Its hash, with the rule engine version, is the rules digest; it names the
staging file and the citation cache, so a build with other rules never reuses either. Members may
run different rules: each trims its own copy with its own build's. In summary (the kinds a member
drops today are marked):

| Kind | Slot | Kept |
|---|---|---|
| `harness.observed` (dropped) | subject, incarnation | the first claim; the first `ready`, `working` or `idle`, and the first of those that is not a credential state; the newest; the newest non-`working` claim and every `working` claim after it; the newest carrier of each optional field; the start of the current native credential episode and the credential report that ended the one before (rules version 24); the newest `working` claim and the newest `ended` claim (rules version 25) |
| observed seat status history (`harness.observed` transitions and runtime resets) | seat | the last 200 transitions and runtime resets within seven days before the cut, ignoring stamped heartbeats; for each incarnation the transition before its first visible one, even when older; the start of the current state |
| `harness.timeline` | subject, incarnation | the newest; only claims dated at least five days before the cut go |
| `loop.state` | subject | the first and last claim of each run of equal (status, round); the first claim carrying the loop's items |
| `subscription.mission-deferred` | subject, request | every claim while the request is open; once a start, failure, cancellation or release closes it, the newest |
| `observer.observed` (dropped), `daemon.diagnostic`, `transport.observed` | subject (and code, or origin) | the newest |
| `resource.observed` written by an observer (dropped) | subject | the newest; the newest of each `attribution_only` value; the earliest that names an opener; for a pull request, every observation back to and including the newest one that is not open outside the merge queue, or the newest 400 when there is none, and any accepted at the same time as that one (the merge-queue check walks back that far); every observation of a resource any of them describes in flat facts (rules version 23) |
| `runtime.action.*` with no actor, `render.applied`, `runtime.readiness-deadline-reached` | subject (and action, incarnation, status) | the newest; only claims dated at least five days before the cut go |
| `sekret.called`, `sekret.exited`, `sekret.refused`, `sekret.changed` written before they became local observations | subject | the newest; only claims dated at least five days before the cut go |
| `harness.limits` | subject | the newest; the newest reading the account fold accepts, and any accepted at the same time; every reading measured within an hour of the newest reading of its account group (driver, account, declared account, weekly or five-hour only) across every seat |
| `harness.todo.observed` | subject | the newest |
| `runtime.reconcile-decision` | subject, decision key | the newest of each key; the reconciler asks whether a key was recorded, reads the newest member-reconcile decision and the subject's newest |
| `harness-question.observed` | subject | every observation while pending; newest observation indefinitely once resolved at least a day before the cut; the resolution is durable (see [Harness questions](harness-questions.md)) |
| `harness.usage`, response rollups | subject, incarnation, model, account, run, step, host | the last snapshot of each UTC hour in the seven days before the cut, the newest snapshot before that window, and the newest of all |
| `harness.usage`, session cumulative | subject, incarnation | the newest and the last claim of the largest total |
| `harness.usage`, context occupancy | subject, incarnation | the newest |

Trimming never resets a seat's `since` or clears a held credential refusal: the start of the current
state and of the current credential episode are always kept. The client read separately enforces its own
seven-day / 200-item bound.

Kinds the rules do not name are never dropped: missions, runs, steps, work claims, gates, messages,
attention, membership, desired state, `checkpoint.*` claims and every durable fact. The same goes
for a person's claim of any kind.

**First-native-launch history is durable.** `custom.agent.first-native-launch` receipts,
`custom.agent.first-native-launch-acknowledged` acknowledgements, `harness.session-file` bindings
and `harness.diagnostic` notices with code `first-native-launch-incomplete` have no drop rule, so
every claim stays: a receipt fences the one-time launch opportunity, an acknowledgement records
explicit recovery rather than erasing that fence, and a binding preserves the native session to
continue. `first_native_launch_receipt_binding_and_acknowledgement_survive_checkpoint_trim` in
`crates/fractal-seats/src/store/checkpoint_tests.rs` applies the real tombstone and deletion path
and checks that native continuation still selects the bound session.

**Why the "five days" rows.** These kinds are local observations that the owning member's own log
already forgets after seven days. A claim is dropped only when it is at least five days older than
the cut, and a cut is due two days after it, so the claim is at least seven days old by then.

**Why `harness.limits` keeps an hour per account.** The account fold reads, for each account
group, every reading measured within an hour of the group's newest, and picks the highest weekly
percentage in the newest reset window among them; it also reads each seat's newest reading to know
which seats use the account. A group's newest measurement only moves forward, so a reading more
than an hour older than the newest the cut holds is never read again, whatever arrives later.

**Why `work.renewed` is not in the table.** It is the case that taught the rules what "witness"
costs. A late step claim can land between any two lease renewals, so no renewal has a witness that
holds for every arrival order, and the step-timing fold needs every renewal. Every renewal stays.

## Worked examples

The names here are invented. Each example's outcome is checked against the planner by a test in
`crates/fractal-store/src/store/checkpoint_tests.rs`: `the_documented_examples_drop_what_the_document_says`
for the harness observation, the renewals and the guard, and
`usage_rollups_keep_hourly_ends_in_the_window_and_one_baseline_before_it` for the usage rollup.
The usage and diagnostic examples show what their rules plan; a member does not delete those
kinds until they join `LOCAL_TRIM_KINDS`.

### A harness observation

A seat `agent/alder.worker` runs one incarnation, `inc-1`, and its harness reports nine
observations. Each carries the same fields, as a real one does:

| # | State | Kept? | Why |
|---|---|---|---|
| 1 | `starting` | kept | the first claim of every incarnation, so the set of incarnations is exact |
| 2 | `ready` | kept | the first `ready`, `working` or `idle`: "was this incarnation ever ready?" asks for it |
| 3 | `working` | **dropped** | after claim 7 is kept, claim 3 can never be the first `working` after the newest other state |
| 4 | `idle` | **dropped** | replaced by claim 7, which is a later non-`working` claim |
| 5 | `working` | **dropped** | as claim 3 |
| 6 | `working` | **dropped** | as claim 3 |
| 7 | `idle` | kept | the newest non-`working` claim: "working since" is the first `working` after it |
| 8 | `working` | kept | the first `working` after claim 7 |
| 9 | `working` | kept | the newest claim, and the newest carrier of every optional field |

Four claims go, and the five that stay answer every question the nine did. Claim 8 and claim 9 both
stay because a late `idle` observation, written by another machine, could arrive and sort between
them. Then the first `working` after the newest non-`working` claim would be claim 9, not claim 8,
and the kept set still answers it. The rule keeps every `working` after the newest other state for
exactly this reason.

Had claim 4 been the only claim that carried a `reason` field, it would stay too, as the newest
carrier of that field.

### A usage rollup

Response rollups are cumulative snapshots: each carries the series' running total, and a usage query
for a period subtracts the snapshot at its start from the one at its end. Take one series (one
account on one step) with these snapshots, where the window is the seven days before the cut:

| Snapshot (running total) | When | Kept? |
|---|---|---|
| 100 | window start − 3 h | dropped |
| 200 | window start − 2 h | dropped |
| 300 | window start − 1 h | **kept**: the newest before the window, the baseline for a period that starts at its edge |
| 400 | window start + 10 min | dropped |
| 500 | window start + 20 min | dropped |
| 600 | window start + 50 min | **kept**: the last of its UTC hour |
| 700 | window start + 65 min | dropped |
| 800 | window start + 90 min | **kept**: the last of its hour, and the newest of all, so the series' total is exact |

Five claims would go. A period that ends on any hour boundary in the window, or at the edge of it,
reads the same value, and the lifetime total is unchanged. Every response is also exported to
OpenTelemetry when `[observations.otlp]` is set, and that export is the history beyond the window.
Another account on the same step is its own slot and keeps its own newest snapshot. A legacy
per-response claim has no rule and stays, since every usage read sums those.

### A kind that is never dropped

Step `step-run/example/build` renews its lease three times: `work.renewed` at times 1, 2 and 3, each
extending the expiry. Each renewal supersedes the one before it, so it looks like a dropped claim.
It is not, because `work.renewed` has no rule: the step-timing fold closes an interval when the next
event arrives after the current expiry, and whether a late claim lands inside or outside an interval
depends on every renewal. All three stay, and so does everything else about the step.

### A claim a guard protects

A daemon writes three `daemon.diagnostic` claims with code `slow-request` on `daemon/alder`; a
person wrote the first, `person/avery`. The rule keeps the newest per (subject, code), so the second
looks droppable, and it is: the third replaces it. The first is a person's claim, which is never
dropped, so all of the first and third stay. Only the second goes. If a later claim cited the
second as evidence, the evidence guard would keep that one too.

## Two checks before and during a trim

There is no proof copy. Two checks stand in for it, and either one stops the trim:

- **Before anything is recorded**, a plan that would drop the newest claim of any subject and kind
  is refused whole. Every rule keeps the newest claim of its slot, and no slot is coarser than
  (subject, kind), so this finds only a planner defect. The member records a
  `local-trim-would-drop-newest` diagnostic naming the claim.
- **While deleting**, every chunk checks, in its own transaction, whether the graph generation
  moved; if it did, it compares the projection digests before and after the chunk. A change rolls
  the chunk back and records a `checkpoint-trim-graph-changed` diagnostic.

Either stop marks the cut `graph-changed`, opens an operational fault (`local-trim-stopped`) on the
member's `daemon/NODE` subject for the agent `[trim] notify` names, and halts trimming on that
member until `fractal replication checkpoint resume --reason TEXT` is run, by a person or by the
`[trim] notify` agent (`--as agent/...`), and no other agent. The resume sets the stopped cut aside,
the next day's cut goes on, and the fault ends. A stop has found a bug in a rule or the planner:
turn the diagnostic into a failing test, then fix the rule.

A third check runs across members: the daily answers check compares what each member answers for
the claims older than the cut ([Answers across members](replication.md#answers-across-members)).

## Trimming

A cut is trimmed in two steps, and a crash anywhere is safe.

1. **Tombstones, in chunks.** The member marks the cut `recording`, then for every dropped
   envelope and claim records a tombstone: writer, sequence, hash and, for a claim, its ID,
   subject, kind, actor, predecessors, operation and request digest. Each transaction stops
   recording after 20 ms (`TRIM_CHUNK_BUDGET`). When every tombstone is recorded it marks the
   cut `trimming`.
2. **Deletions, in chunks.** The member walks only that cut's own tombstones, keeping its place in
   `meta` so a restart goes on from it. For each dropped envelope it deletes its claims, their
   events, records, operations, the envelope's signatures and the envelope row; for a dropped claim
   in a kept envelope, the claim's own rows. It deletes each claim's signature and cached verdict
   rows too (`claim_signatures`, `claim_verdicts` and the verdict queues). Its verdict links stay
   until a requeue meets them or the daily link sweep finds the claim gone.
   After the last chunk it marks the cut `trimmed` and moves the committed index forward by one, so
   snapshots and page cursors taken before the trim expire through the existing "snapshot changed"
   path. A cut that dropped nothing is trimmed at once.

A crash while the cut is `recording` has deleted nothing. The next step finds the cut not yet
trimmed, forgets the tombstones it recorded, and captures and plans it again. A crash while it is
`trimming` leaves every tombstone recorded; the next step deletes what is left.

The signature and verdict rows are local caches outside every digest. Each step sweeps any such
rows an earlier trim left, walking the claim tombstones in order one claim at a time, in
transactions that stop after 20 ms (`SWEEP_CHUNK_BUDGET`), then records
`checkpoint_trimmed_claim_rows_swept = done` in `meta` so later steps read one row.

### Tombstones and the retired ledger

A tombstone and the envelope it stands for have one identity, so the inventory lists the same
identities before and after a trim. A tombstoned envelope is never sent, and one that arrives is
not stored. A claim ID with a tombstone still counts as existing for evidence checks; an ancestry
walk goes on through its predecessors; and a retried request for a dropped claim is answered with
the claim's ID rather than written a second time (`retired_operation_receipts`,
`retired_claim_ancestry`).

The retired ledger (`retired_claim_ids`, `retired_envelopes`) is the member's authority for what it
deleted, once its import of older checkpoint rows has finished. Becoming the authority judges the
ledger once: an identity that only a cut that never committed recorded was not a deletion and
leaves it. From then on an identity enters the ledger only as it is deleted, in the deleting
transaction, so a cut that stops before deleting, or a chunk that rolls back, retires nothing.
Admission then refuses a retired envelope or claim from any peer, whatever its build; the
inventory lists a retired envelope as held without payload, so no peer sends it; and the member
serves no retired envelope or claim.

Retired claim IDs travel between members in the claim-range summary (`retired`; see
[Fleet replication](replication.md)). A member that learns a retired ID it lacks records it as
retired too, so it never admits, asks for or serves that claim again, even when it comes from a
member that still holds it or runs an older build; a copy it still holds stays until its own trim
drops it. An older build neither sends nor reads `retired`.

**What does not change.** The graph and every reader's answer at the current index. A read at an
older snapshot, or of a dropped claim's history, answers from the kept claims.

### Backups

A claim backup writes the newest trimmed cut's tombstones as its `checkpoint` record, with the
permanent ledgers, in the unchanged archive format. Restore records the archived tombstones and
retired-ledger rows as they are. See [Backups](backups.md).

## The trim budget

A trim shares the store with live writes. It never makes one wait long:

- Recording and deleting each stop starting new steps after 20 ms (`TRIM_CHUNK_BUDGET`), and a
  chunk deletes at most 2,000 envelopes. Chunks use background writer admission and rest outside
  the writer between commits for four times the chunk's hold, at least 100 ms and at most 5 s.
- A member deletes at most `[trim] daily_claims` claims a day (100,000 unless set; about 1.7 h of
  paced deletion on a 53 GB member). A cut that drops more stops at that count, stays `trimming`,
  and goes on the next day where it stopped; the next cut begins only after it finishes, with what
  the day has left. The day's count is kept in `meta` (`local_trim_day`) in the deleting
  transaction, so a restart neither repeats nor exceeds it.
- Past 65,536 WAL frames (256 MiB of 4 KiB pages) a trim stops deleting and restarts the WAL,
  waiting up to 200 ms per attempt for older readers and trying for 30 s; if the WAL will not
  restart, the run stops and the next step goes on from there.
- While it deletes, the views refold and the replication snapshot is patched at most once a minute,
  and the snapshot also whenever 4,096 deleted envelopes are held; when the run stops, however it
  stops, the views are forgotten once and the snapshot patched once more. What a chunk's deletes
  deferred is rebuilt, a budget's worth at a time, when the run stops.
- A chunk that holds the writer over 1 s logs one line with its phase, counts, body bytes and its
  longest stages.

Every foreign key column leads an index, because deleting a claim looks up every row that
references it. `every_foreign_key_column_leads_an_index` in `crates/fractal-store/src/store/tests.rs`
fails otherwise, so add the index in the same change as the foreign key.
`a_trim_never_makes_a_write_wait_long` in `crates/fractal-store/src/store/tombstones_tests.rs` trims a
production-sized history with every row made slow and fails without the budget, and
`production_history_has_bounded_journal_writes` bounds that setup's journal writes.
`crates/fractal-store/src/store/local_trim_tests.rs` covers the daily cap, a restart part way through
a capped cut, the WAL bound and the stops.

## The working set

Capture, planning and trimming work from staged data on disk, and a step holds a fixed working set
plus the claims of one slot, whatever the size of the history. The `checkpoint_memory` regression
runs a full pass at two history sizes four times apart and requires each measured stage to stay
within 8 MiB and grow by less than half when the history quadruples. A restart resumes the saved
capture rather than reading the history again. The capture is described in
[Checkpoint capture](checkpoint-capture.md).

## Older members

A member on an older build that still runs shared checkpoints counts this member as a participant,
and this member never seals, so the older member's shared checkpoints stop becoming stable, as
with any `[trim] local` member. A few days after one became due, the older member asks a person to
excuse this member (`fractal replication checkpoint excuse` on that build) or to upgrade it. The
`checkpoint.sealed`, `checkpoint.verified` and `checkpoint.excused` kinds stay registered and
admitted, since old history and older members carry them, but no new ones are written. This member
advertises no checkpoint in its inventory, so an older member adopts nothing from it.

## Where the code lives

| What | Where |
|---|---|
| Due time and names | `crates/smallclaims/src/store/checkpoint.rs` |
| The daily cut, tombstones, trim, budget and pacing | `crates/smallclaims/src/store/checkpoint_trim.rs` |
| The staging file: capture, cursor, plan marks, tombstone pages | `crates/smallclaims/src/store/checkpoint_staging.rs` |
| The scoped capture | `crates/smallclaims/src/store/checkpoint_staging/scoped.rs` |
| Disk admission for the capture | `crates/smallclaims/src/store/checkpoint_space.rs` |
| The citation cache | `crates/smallclaims/src/store/local_trim_citations.rs` |
| The retired ledger | `crates/smallclaims/src/store/retired.rs` |
| The rules, each kind's class and window | `crates/fractal-schema/retention.toml` |
| The guards, the planner | `crates/fractal-store/src/store/checkpoint_rules.rs` |
| The planner over a staging file | `crates/fractal-store/src/store/checkpoint_rules/staged.rs` |
| A stopped trim's fault and resume | `crates/fractal-store/src/store/local_trim_fault.rs` |
| A step's memory budget | `crates/fractal/tests/checkpoint_memory.rs` |
| Rule examples as tests | `crates/fractal-store/src/store/checkpoint_tests.rs` |
| Local trim tests | `crates/fractal-store/src/store/local_trim_tests.rs` |
| Trim, tombstone and replication tests | `crates/fractal-store/src/store/tombstones_tests.rs` |

A new rule changes the rules digest. A new kind is dropped only once it joins `LOCAL_TRIM_KINDS`
with a test that every reader answers the same from the full and the trimmed log. See
[Retention](retention.md) for how to add one.
