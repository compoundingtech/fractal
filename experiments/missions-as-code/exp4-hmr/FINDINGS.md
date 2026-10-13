# Experiment 4: changing a running mission

Question: what happens to in-flight runs when a mission program is reloaded mid-run, and how do
versions stay replayable?

**Short answer:** a run moves to a new version only if the new version replays the run's
history. Otherwise it stays on the version it has, which stays replayable because versions are
immutable, content-addressed documents that carry their own engine. This works, live through st.

The catch is the rule itself: op names and their order are the compatibility contract. "Insert a
step before where a run is" is the change people make most, and it is the one that needs
discipline (`patched()`).

## The model

- **A version is one bundle.** It holds the program, the SDK, the `decide()` engine and Effect
  itself (~290 KB, `bundle.ts`). Its version is the SHA-256 of the bytes, the same hash st gives
  it as a document: `doc/mac/programs/orchard-ship-change@a0b79605…`. The bundle brings its own
  engine, so a later SDK cannot change how an old version replays. Bundling is deterministic:
  the same source gives the same hash.
- **The journal names the version.** The run's `started` event records it. A move appends
  `upgraded {version, accepted}`, and a replay uses the newest version in the history.
- **A reload is a request in the graph.** `custom.mac.reload.requested` on
  `custom/mac/reload/<run>` names a version. The driver answers with
  `custom.mac.reload.answered`, `moved` or `refused`, and a reason.
- **The check replays the history through the new version** (`upgrade.ts`), with changed
  definitions reported instead of fatal. There are four verdicts:
  - `compatible`: replays exactly. The run moves.
  - `compatible-with-changes`: some finished ops are defined differently now; their recorded
    results stand. The run moves, and the `upgraded` event lists the accepted ops so later
    replays agree.
  - `open-op-changed`: an op the run still waits on is defined differently. Its run was
    started with the old KDL, so moving now would misdescribe it. The request waits until the
    op finishes, then is checked again.
  - `incompatible`: the history does not replay. The request is refused, and the run keeps
    its version to the end.
- **`patched("id")`** answers true for a run that reaches this point under a version that has
  it (and records a marker), and false while replaying a run that passed this point without it.
  It is the escape hatch for changing code behind a run's position.

## The matrix

Runs of v1 were stopped at four points. Each v2 was checked; a run that could move ran to the
end under v2, and one that could not ran to the end under v1. From `node exp4-hmr/matrix.ts`
(the full table, with reasons, is in [RESULTS.table.md](RESULTS.table.md)):

| change | building | in review | fixing round 1 | last CI round |
|---|---|---|---|---|
| **v2-text**: the fix step's goal text | compatible | compatible | **open-op-changed** (waits) | compatible-with-changes |
| **v2-insert**: a scan step after review | compatible, scan runs | compatible, scan runs | **incompatible**, stays v1 | **incompatible**, stays v1 |
| **v2-patched**: the same, behind `patched()` | compatible, scan runs | compatible, scan runs | compatible, no scan | compatible, no scan |
| **v2-remove**: the docs step removed | **incompatible** | **incompatible** | **incompatible** | **incompatible** |
| **v2-rounds**: 5 CI rounds instead of 3 | compatible | compatible | compatible | compatible |

What the matrix says:

- **Changing code ahead of a run is always safe.** The run picks it up.
- **Inserting code behind a run breaks replay.** In the history, the next op after review is
  `round-1/ci`; v2 asks for `security-scan` there.
- **`patched()` makes the insertion safe everywhere.** Only runs that reach the new code
  after the move take the new path.
- **Removing an op the run already scheduled never replays.**
- **A text change** moves finished work and waits on open work.

## Live, through st

[result-live.txt](result-live.txt), from `lab-demo.sh` on the lab instance:

- **hmr-5** (in review): reload to v2-insert → `moved: the history replays exactly`. The driver
  was then SIGKILLed. st restarted it, and it **loaded v2 from the document store** and replayed
  12 events. The new `security-scan` step ran after the review, and the run completed.
- **hmr-6** (fixing round 1): reload to v2-insert →
  `refused: event 17, op round-1/ci: the history scheduled an op this program never asked for here`.
  Then reload to v2-patched → `moved`. The run finished without the scan.
- **An old version stays retrievable by its hash** after the name has moved on:
  `st documents get doc/mac/programs/orchard-ship-change@9d4cea…` returned the exact bytes.

What happens to in-flight runs, in one line each:

- New runs start on whatever version the host mission names.
- A running run moves only when asked and only if the new version replays its history.
- A refused run finishes on its own version, which stays replayable from the graph.

## Where it falls short

1. **Op names are the contract, and the counters are brittle.** Renaming an op, reordering two
   ops, or adding a retry in front of a counted name (`build.2`) breaks replay. Experiment 3
   found the auto-counted timers (`timer`, `timer.2`) meant different things on different
   paths. Timers now have names (`timerName`, `sleep(id)`, `escalateAfter(name, …)`), but
   nothing enforces naming.
2. **`patched()` needs the author to know where runs are.** The most natural change, "add a
   step after review", is refused for every run past review unless it is guarded. A tool that
   diffs two versions' op sequences, using experiment 3's static extractor, could warn at publish
   time; I did not build it.
3. **A changed open op cannot be applied to its running child.** The child run was started from
   the old KDL. st's own revisions restart a changed step; the program could do the same by
   cancelling and rescheduling under a new run ID. It does not: that would discard work
   silently.
4. **Rollout is manual.** A run moves only when asked. "Move every compatible run" is one check
   per run and easy to add, but nobody should run it unattended against `incompatible` runs.
5. **Retention.** Old bundles must outlive every run and audit that references them. st's
   retention rules would need to pin `doc/mac/programs/*` versions that a journal names.
6. **Size.** Each version is about 290 KB, because Effect is inside. Splitting the SDK out would
   save space and reintroduce engine drift: the reason old versions replay is that they do not
   share code with new ones.
7. **Compared with KDL revisions.** st's revision model matches steps by definition hash and
   restarts changed steps and their dependents. It is more forgiving because a KDL mission has
   no "position": order lives in `depends-on`, not in a replayed sequence. A program trades
   that for control flow; the price is this stricter contract.

## Verdict

Hot reload of a running program works and is safe by construction: a run either replays under
the new version or keeps its own. Content-addressed bundles make every version replayable
forever, at about 290 KB each. Day to day, it needs naming discipline and `patched()`, and a
publish-time check would catch most mistakes before a reload is refused.
