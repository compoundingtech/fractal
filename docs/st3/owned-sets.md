# Owned sets

An owned set gives one publisher responsibility for a complete list of top-level seats, mission
definitions and schedules. A successful publication retires previously live members that are
absent from that list. Membership lives in the ordinary graph, on `owned-set/NAME` subjects with
immutable `owned-set.revised` claims. There is no inventory database alongside the graph.

Ordinary `fractal apply FILE...` publication remains an upsert: omission has no effect. Owned sets are
an explicit choice, made with `fractal apply --set NAME`. A file disappearing from a checkout does
nothing by itself. The publisher must submit a complete, valid bundle. An unreadable input,
invalid declaration, stale fence or unconfirmed mass retirement rejects the entire transaction.

## Publish a complete bundle

Each file starts with `version 2`. The files together declare the complete live membership:

```sh
fractal apply --set garden seats.kdl missions.kdl schedules.kdl \
  --repository acme/garden --ref refs/heads/main \
  --sha 0123456789abcdef0123456789abcdef01234567 \
  --source-sequence 42 --expect-set absent \
  --as person/operator --dry-run
```

Remove `--dry-run` to publish. A dry run prints blockers and exits with failure when any
are present. Add `--check` to execute gates during the preview, with `--workspace DIR` and
`--input NAME=VALUE` as needed; gate answers appear on stderr. Publication checks for broken
mission and step exec gates by default; `--no-gate-check` skips them. Both checks cover only
the missions the preview lists under `gate_check`: those whose exact revision this daemon does
not hold yet. An unchanged or adopted member is not checked again, so a large set's publication
does not rerun its gates; that includes a revision published with `--no-gate-check` or
replicated from another daemon. Use `fractal missions check FILE` to check every gate again.
Loop `until` scripts
are listed as "not checked, they run after each round", including in dry-run checks.
Per-round step scripts still preflight; runtime `until` evaluation remains at each round boundary.

All source flags are required together with `--set`. Without `--set`, publish plain files
with `fractal apply seats.kdl missions.kdl schedules.kdl --as person/operator`.

Initial creation requires `--expect-set absent`; subsequent
publications require the exact selected revision printed by `fractal sets show garden`. The CLI
previews first, then submits captured member heads with the apply. The daemon checks the set
revision, source sequence and every captured member head within the publication transaction.

An existing unmanaged declaration requires `--adopt agent/garden/orchard` (repeat for each
subject). Adoption captures the old heads in the set revision. A member already belonging to
another set is refused. Retirement keeps ownership; v1 has no release or transfer operation.
Reintroducing a retired member through its set makes it live again.

Mission runs, mission-owned seats, resources, observers and subscriptions cannot be set members.
Publish those through their existing routes. A mission definition may contain its normal
run-owned declarations; changing its definition preserves active runs and their pinned revisions.

An agent's nested `exec` and `pty` tasks (`exec/NAMESPACE/AGENT/TASK`) are members carried with
their agent: they are published, adopted and retired together with it, and the agent's `under`
grouping is unchanged. A changed task launch restarts the task; `--rollout when-idle` applies to
agents only. Standalone top-level `exec` and `pty` declarations cannot be set members.

A schedule or subscription in the bundle can pin a mission of the same bundle with
`mission "NAME@ST_REVISION[NAME]"`. The daemon resolves each token to the revision this
publication stores before validating the bundle, so one publication carries both the mission and
its pinned schedule. A token naming a mission outside the bundle, or tokens pinning each other in
a cycle, refuse the publication.

## Declaration diffs and readback

`fractal apply --set NAME ... --dry-run` includes an additive `declaration_diffs` map keyed by
subject for added, changed and retiring members. Each entry has `before`, `after` and `fields`.
`before` is null when the subject has never been published; adoption includes its existing
unmanaged definition. `after` includes the proposed retirement declaration when retiring.
Unchanged members have no entry. `fields` lists the changed JSON pointers in sorted order;
objects are compared by field and arrays as a whole. The empty pointer means the complete
definition was added. The existing classifications, effects, fences, digests and exit codes
are unchanged. The preview uses the existing person-or-agent actor validation.

The values are canonical structured publication definitions: seats and schedules use the
normalized `DesiredSubject` (including its canonical declaration AST and compiled seat member),
and missions use the compiled `MissionSpec`. The mission compiler supplies concurrent run,
revision cutover, completion, retry and duration defaults. These values come from the same
compiler and selected graph definitions as publication; publishers need not reproduce defaults
or render KDL themselves. Derived revision and step definition hashes remain visible.
`fractal apply FILE --dry-run` and the intent preview reuse this same `declaration_diffs`
format. These JSON values are inspection data, not an alternative JSON publication input.

Read an applied definition with the client operation `publication.definition`:

```text
GET /v1/client/publication-definition?subject=mission%2Fgarden%2Fharvest
GET /v1/client/publication-definition?subject=schedule%2Fgarden%2Fdaily
```

The response envelope contains the selected `subject`, `declaration`, `revision` and immutable
claim `token`, with a snapshot from the same read transaction. Owned members follow the selected
set references; active mission runs keep their pinned revisions. Rust `publication_definition`,
Swift `publicationDefinition` and TypeScript `publicationDefinition` also accept agent subjects.
The operation requires both `read.projections` and `read.declarations`, since canonical values
can include environment values and embedded mission declarations. It returns 404 for a missing
definition and rejects unsupported subject kinds or an oversized response without truncation.
The existing redacted agent `subject.definition` operation remains available.

## Retirement

Omitting a seat declares a stop while keeping its conversations and history. Omitting a mission
prevents new runs, including starts pinned to an older revision, while retaining active runs.
Omitting a schedule stops future occurrences and retains work already created. Omitting a
top-level subscription stops its deliveries and retains messages it already delivered.

A seat declared `one-shot` also permits its runtime host to retire that exact member after the
process exits. The daemon records a stop while retaining set ownership and the source bundle.
A new member declaration published through the set starts it again. Other members still require
ordinary source publication to retire.

A publication retiring ten or more members, or at least half the previous live membership,
requires `--confirm-retire DIGEST` from its dry-run preview. The digest binds the exact bundle,
source, prior set revision, member heads, adoption flags and empty-set flag. Any change requires
another preview. Automation must not automatically copy a refused preview's digest and retry.

An intentional empty set additionally requires `--allow-empty`. No input files are accepted only
with that flag; specifying a missing file always fails. Empty membership that retires existing
members still needs the exact retirement confirmation.

## Source ordering and replication

The first publication fixes the repository and full branch ref. Each later publication must have
a greater unsigned source sequence. Equal sequence is an idempotent retry only when both the SHA
and normalized declaration bundle match. A newer source with unchanged declarations records a
receipt and preserves the member tokens, so it does not relaunch seats.

The publisher derives the sequence from the commit's full first-parent depth and verifies ancestry
against its last successful source. It must reject shallow history, non-descendants and rewritten
branches. Fractal checks the numeric fence and graph references; it does not fetch Git or verify that a
publisher derived the sequence honestly. Roll back with a new revert commit and a larger sequence.

Every upgraded replica selects the highest source sequence, irrespective of arrival order or wall
clock time. Membership is complete: a member learned only from a losing branch is also retired
when absent from the winner. Its stop derives from the winning set claim; the next publication
records an explicit stop reference in the retired map. Equal-sequence divergent content is a visible conflict and holds member effects.
Missing previous revisions or member references also hold effects until replication supplies the
dependencies. Managed desired state resolves through the selected set's exact references, so an
older independent declaration cannot override it when partitions heal. Checkpoint replay preserves
set revisions and their member references.

A disconnected publisher can accept locally valid work and cause local effects before learning
of a higher sequence. This protocol provides convergence after healing; it does not provide
exclusive leadership during a partition. Keep the existing serial applier.

Before activating owned sets, upgrade every active fleet member. Publication refuses activation
unless each admitted member's latest own `daemon.started` advertises `features.owned_sets=1`.
A set with agent `exec` or `pty` task members additionally requires `features.owned_set_tasks=1`,
because older daemons refuse replicated revisions that carry such members.
Ordinary publication and existing clients remain usable during the upgrade. An older daemon must
not be introduced into a fleet after set activation.

## Source maps

`fractal apply --set ... --source-map MAP.json` records a `fractal.source-map/v1` document beside
the set commit, in the same transaction. Each mapped subject must be a live member of this
publication; anything else is refused with `invalid-source-map`, and nothing is applied. A file
may omit `text` and name only its `sha256` when the daemon already stores that text; an unknown
sha256 is refused with `source-blob-missing`. Files are capped at 1 MiB and a map at 32 MiB.

Each mapped member revision gets one `source.mapped` claim on `<agent|mission>/<id>@<revision>`
naming the set revision, source commit, its files (`blob` is the git blob id, `blob_hash` the
sha256 of its text) and the spans of its KDL nodes. The texts those files name are stored once by
sha256 as claim-referenced blobs, so they replicate and survive checkpoints with the claim. The
first mapped publication of a revision wins; a later map for the same revision is kept, never
refused, so the claim says where the revision was introduced. A noop publication records nothing.
Maps never enter a revision, bundle, preview or set hash. The apply result reports
`source_maps: {recorded, kept, blobs_new, blobs_deduped}` (blob counts are distinct texts).

The apply idempotency check covers the intent, options and actor, not the map: a retry with the
same key and a different map answers the first response and records nothing from the second map.
A revision left unmapped that way gets its map from the next publication that carries one.

`GET /v1/source-maps?subject=ID[&revision=REV][&texts=true]` answers the claim of a revision
(default: the current one), with `texts: {blob_hash: text}` on request; 404 `source-map-missing`
when that revision has none. `GET /v1/source-blobs/<sha256>` answers one stored text.
A `step-run/...` subject answers its mission at the revision its run's generation pinned, plus
`step` and `node`, the step's KDL node path (`mission[ID]/step[STEP]`, nested steps
`.../step[OUTER]/mission[NESTED]/step[INNER]`, gates `.../gate[NAME]`). A node the map does not
list inherits its nearest listed ancestor.

- `GET /v1/source-maps/revisions?subject=ID` lists the revisions with a map, newest first, with
  their commit and set revision, plus the subject's `current` revision.
- `GET /v1/source-maps/where?path=PATH[&repository=OWNER/REPO][&line=N]` reads the maps of every
  live agent and mission revision that name the file. Per repository it answers the newest
  recorded version (`reference`), and per subject whether the file declares it and the nodes with
  a span or frame in the file. `lines` is a span carried into the reference text by a line diff
  (`null` once every line of it changed); with `line`, only nodes covering it are answered.
- `GET /v1/source-maps/declaration?subject=ID&revision=REV` answers the canonical KDL of that
  revision, environment values redacted, for diffs between revisions.

`fractal subject show ID --source`, `fractal source where` and `fractal source diff` read these;
`fractal work claim` adds a `SOURCE` row, and the faults for a broken gate or a step out of time,
and a failing gate's reason, name the declaring line.

## Inspect publication and rollout

```sh
fractal sets ls
fractal sets show garden
fractal sets status garden --sha 0123456789abcdef0123456789abcdef01234567
```

The selected resource reports source, immutable live and retired references, blockers, and each
member's desired token, launched token and running incarnation. A successful `start` action's
`desired_token` proves the declaration that launched that incarnation. Publication alone does not
prove a seat is running. Label-only updates share their existing launch lineage, so a running
seat stays current while its reported launched token retains the original label revision. A commit
query distinguishes a recorded receipt, local visibility,
supersession and current rollout. Remote replica visibility remains unknown in this response.
`fractal sets status <set> --sha <sha>` returns 404 until that SHA has a receipt; this is intended.
Read `fractal sets show <set>` to obtain the current revision for publication fencing.

For native-session continuity, manual rollout and cutover refusals, see
[owned-seat cutover](owned-seat-cutover.md). Use the set publisher to change a managed
declaration or retire it.

Owned sets do not install a repository watcher. Git-backed automation remains a separate,
configured `github.ref` observation, pinned subscription and CI gate feeding the serial applier.
The applier chooses the input files and publishes the complete bundle through this command.

The publication API is `/v1/sets/preview` followed by `/v1/sets/apply`; both take the same typed
request with intent, source, set fence and actor. Apply also needs the preview's member heads and,
when required, its retirement digest. Client-v0 read routes are `/v1/client/sets` and
`/v1/client/sets/NAME`, with optional `?sha=SHA` on the detail route. Rust, Swift and TypeScript
clients expose the additive `owned-set` resource and set list/detail operations.

An optional [`when-idle` rollout policy](owned-seat-cutover.md) drains changed and retiring native seats
and verifies their original conversation on the replacement. Without it, automatic seats retain their
immediate runtime behavior. Manual seats defer cutover with or without the set policy.

## Source enforcement

A set's revision may carry a source policy. The publisher declares it and `--expect-set` fences it
with the rest of the revision. A publication that omits it keeps the selected policy:

```sh
fractal apply --set garden ... --enforce enforce --publisher agent/dotfiles/factory-publisher \
  --namespace 'agent/garden/*' --unmanaged-allow 'agent/garden/scratch-*'
fractal apply --set garden ... --source-policy policy.json
```

`policy.json` holds `{"enforce": "off"|"audit"|"enforce", "publisher": [...], "namespaces": [...],
"unmanaged_allow": [...]}`. Each omitted field keeps the selected value, and explicit flags override
the file. A policy equal to `off` with empty lists is stored as absent. Namespace and allow globs
claim top-level `agent/`, `schedule/`, `subscription/` or `mission/` subjects only: an agent's
`exec`/`pty` tasks ride with their agent, and other `exec/` and `pty/` subjects are runtime
(mission-run tasks, owned terminals), so such a glob is refused with `invalid-source-policy`.

- `off`, or no policy, is the behaviour above: a plain declaration of a member is refused with
  `set-managed-subject`, and nothing else is checked. There is no force path.
- `enforce` refuses every ad hoc desired-state write to a live member: plain `fractal apply`
  (definitions, schedules and subscriptions), `agents stop`, `agents start`, mission publication,
  `missions retire`, `agents hold`, `agents suspend` and `agents resume`. It also refuses creation of
  a new subject matching `namespaces` but not `unmanaged_allow`. The refusal names the verb, the set,
  its source, and the escape.
- `audit` refuses nothing that `off` accepts. Each write that `enforce` would refuse proceeds and
  appends `source.audited` (verb, actor, set revision and the refusal) in the same transaction. A
  plain declaration of a member remains refused because it cannot take effect over the set's own
  member reference.

Terminal `agents retire` of a live member of an `enforce` set is refused with `set-managed-subject`
through every path, including client-v0. It cannot be forced: omit the member from the set
publication, then retire it. An `audit` set accepts the retirement, records it as `source.audited`
and also as `source.debt.recorded` with the verb `retire`.

No publication can undo a terminal retirement. A publication that still declares a retired agent,
whatever the set's policy, keeps it as a retired member instead of failing with
`recipient-retired`: its preview reports the change `retired` with the effect `keep terminally
retired by ACTOR: REASON; drop it from the source`, and the rest of the bundle applies. The retire
debt stays open while the source declares the member. A publication that omits the member reports
the same change with the effect `keep terminally retired by ACTOR: REASON`. That is not a retirement
by the publication, so it does not count toward the `--confirm-retire` threshold. That publication
closes the debt as `adopted`.

`--force --reason TEXT` accepts the write on an `audit` or `enforce` set. The reason must be
nonempty. Persons and agents may force (`fractal apply`, `agents stop|start|hold|suspend|resume`,
`missions retire`). The write takes effect, and `source.debt.recorded` on the member names the set
revision, its source, the verb, the reason, the forced claim (or, for a hold, suspend or resume, the
runtime request) and the changed pointers. The debt, like a `source.audited` record, is appended in
the transaction that makes the change: a write whose debt cannot be recorded does not happen. A
forced declaration (including a mission) overrides the set's member reference until the set
publishes again.

The next publication with a new source sequence converges every member and closes all of the set's
open debt, except retire debt on a member it still declares. Its preview lists `reverted_debts`, and
apply returns `source_debts_reverted`. Each entry has the outcome `reverted` (the source differs, or
a forced hold or suspension is still in place), `adopted` (the source now declares the forced value
or created subject, or no longer declares a retired member), or `closed` (a runtime
change that no longer stands, such as a forced resume or a hold someone replaced). A reverted change
shows the effect `revert ad hoc change by ACTOR: REASON`. In its own transaction the publication
writes one `source.debt.closed` per debt (debt claim, outcome, closing set revision, publisher),
releases a forced delivery hold that is still the seat's latest, and resumes a forced suspension
that is still the seat's latest suspend or resume: it appends the resume request `agents resume`
would, on the seat's current host, as the publisher. A forced creation that the source still omits
is stopped and kept as a retired member.

```sh
fractal source status            # policies, open debt, debt each current revision closed
fractal source status --audit    # also the newest 200 audit records
```

When the selected or requested policy names `publisher`, only those actors may publish the set
(`source-publisher-refused`). Replicated admission does not re-check this because writers are not
signed yet. Treat the publisher list as a local guard, not authority against a compromised peer.

A revision carrying a policy, or a set with subscription members, requires every active fleet
member to advertise `features.source_enforcement=1`. Older daemons reject such revisions and
ignore debt overrides.

Not routed through source: messages, asks and answers, work claim and completion, run cancel,
mission run starts, a seat's own `restart-when-idle`, and restarting another agent (a runtime
action with no desired value). Fractal has no native `startable-by` grant yet, so grant-based
starts are not enforced.
