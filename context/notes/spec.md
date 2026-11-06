# Notes spec

This document specifies st notes: short authored texts that a person or an
agent attaches to one st subject, shown in stui, the Fractal TUI and Fractal
web, and delivered as context to the agents that the target resolves to. Terms
are defined in [ontology.md](./ontology.md).

## Status

Draft. `vision.md` and `requirements.md` do not exist yet. They become
authoritative only after Johannes confirms them in the interview. The
candidate requirements below are input for that interview, not constraints.

### Implementation status

The source implementation follows Johannes's 2026-10-15 interview
(DQ3/DQ5): writes store `agent`, API and CLI inputs have no audience choice,
and reads and digests include every current note regardless of stored
audience. Composer audience controls and audience-specific badges are
removed; the schema's open audience enum stays.

The directive/subject-note CLI split is implemented in source:
`directive set|clear|show` is available through the shared `fractal`/`st`/`st3`
CLI. Bare `note` and `note set|clear` print a one-line deprecation notice.

DQ8 dedicated-only writes are implemented: generic claim admission refuses
`note.set`; the note API retains local target/assignment checks, and peers
trust the origin's check without changing replicated admission.

The minimal session subject (DQ2) is registered. Notes use session targets,
not a `session` field. Local note admission checks the parent runtime's exact
running incarnation; current reads and digests stop including those notes on
session end or replacement. `--on session/…` targets one directly, while
`--on agent/… --session current` resolves the authoritative running incarnation
without falling back to an agent-wide note.

The web implements both entry points: detail-view Notes with **Add note**,
plus the selected-row `n` popover, using one composer. The composition Button
exposes React Aria's ref contract for popover anchoring and focus restoration.

Generated clients and schema documentation use session targets and omit the
removed note `session` field. Rich update codecs preserve omitted fields
(unchanged), explicit null (clear), and supplied values, as declared by the
schema's `x-st-preserve-omission` field annotation.

Serial verification on 2026-11-02 covered implementation head
`e0bec6178683129304f3616cab8f2133a71e676b`; the Rust notes and DQ8
tests ran at `2314ef74d852dd7159289aaf7f0480c3a6f05b05`, before the
subsequent codec-generator and Button typing fixes. All builds ran through
resource admission; elapsed times exclude admission queue waits.

| Run | Result | Peak RSS (KiB) | Elapsed |
| --- | --- | ---: | ---: |
| Client generation | Pass | 1,077,832 | 10.16 s |
| Schema Markdown generation | Pass | 1,066,892 | 20.69 s |
| Client generation `--check` | Pass | 720,736 | 4.62 s |
| Eight-package `cargo test … note` | 73 passed (plus 2 child-process tests) | 5,892,168 | 7 m 37.73 s |
| Schema `subject_notes::tests` | 6 passed | 1,068,940 | 17.61 s |
| Exact participant dedicated-operation test | 1 passed | 1,123,952 | 2.88 s |
| Exact note API authority roundtrip | 1 passed | 3,216,196 | 4 m 27.66 s |
| Corrected codec generation | Pass | 713,048 | 7.36 s |
| Codegen regression suite | 28 passed | 1,069,552 | 6.02 s |
| Canonical clippy ratchet and generated freshness | Pass; 109 existing unique warnings | 1,064,004 | 3 m 31.21 s |
| TypeScript client contract/schema tests | 116 passed | 1,173,980 | 5.04 s |
| UI kit typecheck | Pass | 1,803,824 | 23.30 s |
| Web typecheck | Pass | 2,070,000 | 40.43 s |
| Web notes/live-source tests | 82 passed in 2 files | 1,166,604 | 6.79 s |
| UI Storybook build | Pass | 2,176,332 | 31.95 s |
| Both notes play families | 96/96 cases across 24 stories passed | 1,630,652 | 40.17 s |
| Web entry-point negative control | Expected exit 1; 4/4 cases rejected | 1,169,524 | 9.08 s |

Verification repaired two behavioral integration defects: rich patch decoding
previously turned omitted fields into explicit clears, and the composition
Button's public props omitted its underlying ref. The codegen regression suite
also required the existing eight-case pairing scope enum to follow the schema's
one-case-per-line invariant. No tests or gates were weakened.

The kit/UX review verification on 2026-11-03 uses target-labelled note
landmarks (`Notes for {display label}`), including the composer's existing
safe fallback label. Feed-state fixtures give each displayed target a
distinct label. The kit declares the ES2024 library used by story promise
resolvers, and partial-feed fixtures explicitly construct an observed feed.
Play isolation finishes pagehide persistence writes before clearing origin
storage; its regression includes a pagehide storage writer. The quiet byte
hint is checked for clipped assistive-only geometry, not DOM visibility
(which does not account for CSS clipping).

| Review verification run | Result | Peak RSS (KiB) | Elapsed |
| --- | --- | ---: | ---: |
| Initial UI typecheck | Failed: ES2024 promise resolver library missing | 1,819,740 | 25.86 s |
| Corrected UI typecheck | Pass | 1,798,044 | 29.26 s |
| Initial web typecheck | Failed: partial fixture spread retained the feed union | 2,051,852 | 34.37 s |
| Corrected web typecheck | Pass | 2,060,708 | 40.71 s |
| Notes/live-source Vitest | 82 passed in 2 files | 1,166,268 | 9.51 s |
| Kit scenario story checks (including notes) | 814 passed in 1 file | 2,262,608 | 2 m 26.23 s |
| Initial Storybook build | Pass; 3,321 modules | 2,238,448 | 31.62 s |
| Initial notes play gate | 96/124 passed; hint matcher and storage reset failures | 1,620,304 | 1 m 24.16 s |
| Pagehide isolation browser regression | 1 passed | 1,168,772 | 11.80 s |
| Corrected Storybook build | Pass; 3,321 modules | 2,181,664 | 52.91 s |
| Second notes play gate | 112/124 passed; repeated feed-state landmark names | 1,631,832 | 49.32 s |
| Target-labelled web typecheck | Pass | 2,061,784 | 53.26 s |
| Target-labelled Storybook build | Pass; 3,321 modules | 2,172,604 | 29.31 s |
| Final notes play gate | 124/124 cases across 31 stories passed | 1,629,792 | 53.52 s |

The final play suite covers all 31 matching stories at both schemes and
widths (124 cases), with accessibility checks enabled and no omitted cases.
The following controls all used `negativeControl:true`; every required
control rejected all four scheme/width cases with exit 1. The touch-only
Explore control exited 0 as expected: its injected defect affects keyboard
paths, which that story does not exercise.

| Negative control (story suffix) | Result | Peak RSS (KiB) | Elapsed |
| --- | --- | ---: | ---: |
| Note badge `all-states` | Exit 1; 4/4 rejected | 1,634,144 | 7.98 s |
| Note composer `all-states` | Exit 1; 4/4 rejected | 1,628,608 | 9.96 s |
| Note list `all-states` | Exit 1; 4/4 rejected | 1,616,184 | 13.14 s |
| Explore inline quick composer `all-states` | Exit 1; 4/4 rejected | 1,631,852 | 9.85 s |
| Explore keyboard N popover `all-states` | Exit 1; 4/4 rejected | 1,625,428 | 6.98 s |
| Web entry points `all-states` | Exit 1; 4/4 rejected | 1,598,980 | 8.82 s |
| Web entry points `touch-light-440` | Exit 1; 4/4 rejected | 1,629,192 | 7.26 s |
| Web entry points `feed-states-light-440` | Exit 1; 4/4 rejected | 1,624,236 | 6.76 s |
| Explore keyboard N popover `touch-light-440` | Exit 0; 4/4 passed (keyboard-only injection) | 1,620,020 | 8.52 s |

The final UI evidence covers implementation head
`e4b8f3649d07e0f3d9a58ff9e84a7be1b184d74c`. The source tests passed
before the subsequent story/isolation and landmark fixes; the final web
typecheck, Storybook build, full play suite and controls cover those fixes.
Times include shell startup but exclude admission queue waits. No dependency
installation was needed.

Re-review fixes #15–#17 were verified serially on the build host on 2026-11-03 at
`e5b40bc507441e3ecdb8283cc1733ca0583db25d`. Closing the disclosure restores
**Add note** while retaining the draft; per-target create state survives pane
unmounts, and a settled create does not steal focus from the transcript.
Unavailable drafts remain editable with submit disabled. The selected-row menu
names **Add note…** and describes its conversation-only N shortcut.
All requested checks passed without further source changes or weakened tests.

| Re-review verification run | Result | Peak RSS (KiB) | Elapsed |
| --- | --- | ---: | ---: |
| UI kit typecheck | Pass; 0 errors | 1,795,836 | 23.99 s |
| Web typecheck | Pass; 0 errors | 2,061,432 | 37.79 s |
| Notes/live-source Vitest | 82/82 passed in 2 files | 1,164,808 | 8.26 s |
| Kit scenario story checks | 814/814 passed in 1 file | 2,311,124 | 2 m 45.47 s |
| Storybook build | Pass; 3,321 modules | 2,274,780 | 31.26 s |
| Complete notes play gate | 124/124 passed across 31 stories | 1,175,656 | 1 m 14.15 s |

The complete gate used `^fractal-(plugins|explore)-notes-`, accessibility on,
and no omissions; receipts are in the build checkout's `tmp/notes-2867-rr-play`.
Each requested negative control used `negativeControl:true`, rejected all
four scheme/width cases, and exited 1:

| Re-review negative control (story suffix) | Exit / rejected cases | Peak RSS (KiB) | Elapsed |
| --- | --- | ---: | ---: |
| Note composer `all-states` | 1 / 4 of 4 | 1,169,700 | 10.74 s |
| Explore keyboard N popover `all-states` | 1 / 4 of 4 | 1,163,492 | 16.16 s |
| Web entry points `all-states` | 1 / 4 of 4 | 1,178,620 | 13.65 s |
| Web entry points `feed-states-light-440` | 1 / 4 of 4 | 1,167,536 | 8.94 s |
| Web entry points `touch-light-440` | 1 / 4 of 4 | 1,167,732 | 10.43 s |

Measurements include the remote Nix shell startup, exclude admission waits,
and report GNU time's maximum resident set size. Every remote run pulled the
branch fast-forward first and used nice/gate-slot admission; all ran before
15:00Z. No builds ran on the editing host, and no commits were made on the build host.

## Scope

Defines: the note resource, its claim, its projection, the HTTP API, the CLI,
agent-context delivery, and the shared client schema for stui, the Fractal TUI
and Fractal web.

Does not define: git and pull-request annotations (owned by
`agent/dotfiles/annotations`); the UX variants (see [Quick-note UX variants](./ux-variants.md)); the existing person directive note, which stays as
specified in `docs/st3/schema.md`.

## Candidate requirements (pending interview)

- **CR01 Low input overhead:** Adding a note to the selected agent or mission
  takes one key or click, the text, and one confirm. No form, no required
  field other than the text.
- **CR02 Visible where the target is shown:** Clients subscribe to the `notes`
  window and derive note counts and newest-note previews alongside targets,
  without adding note fields to agent or mission collection rows.
- **CR03 Agent context:** Every current note reaches each resolved agent at
  its next turn and at every session start, including the first incarnation,
  through a notes digest.
- **CR04 Inform only:** A note and its daemon-origin digest never assign work,
  approve a gate, wake an agent by themselves, or grant authority.
- **CR05 Shared schema:** stui, the Fractal TUI and Fractal web read notes
  through one generated client schema; no client defines its own note shape.
- **CR06 Short plain text:** Note content is plain text, at most 1024 UTF-8
  bytes. Metadata supplies the structure; there are no tags.
- **CR07 Reactive:** Clients see a new note through their existing collection
  subscription, not through polling.

## Model

```
note/<author-family>/<author-id...>/<UUIDv7>  subject family "note" (new)
  claim note.set   (StateTransition, Durable, WritePolicy: SubjectEncodedAuthor)
    target     SubjectReference   families: agent, session, mission, mission-run, step-run
    text       string | null      nonblank, ≤ 1024 bytes; null = archive
    audience   open enum          known values: "self", "agent"
    expires_at string?            UTC RFC3339, like the directive note
  actor  = author (person/… or agent/…)
  accepted_at = time
```

A note is its own subject so that each note has a stable identity, its own
history, and can be referenced from messages, mission evidence or later
annotations. Examples: `note/person/example/0192…` and
`note/agent/<org>/<name>/0192…`. The author is encoded in the subject and is
immutable. The new `note` family has `client_writable=true`; the existing
`arrangement` family has `client_writable=false`.

`audience` retains the existing codegen open-enum convention
(`crates/fractal-client-codegen/src/rich.rs`), not a closed
string enum (`rich.rs:593–596`). Known wire values remain `self` and `agent`,
but all new notes use `agent`, regardless of author. Audience is not a user
choice or an active privacy boundary: every current note, including an
existing `self` note, is context for its resolved agents. The open enum
preserves room for stricter levels later; it does not introduce them now.
Readers render unrecognized values as a neutral label, never 'Unknown',
and do not exclude them from agent context on the basis of audience.

Text has no tag field, tag parsing or tag picker. Author, target, timestamps
and archived state supply the structure (DQ5).

Wire shape (client-v0, generated into Rust, TypeScript and Swift):

```json
{
  "id": "note/person/example/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d",
  "target": "session/agent/example/worker/1234%3A2026-10-14T15%3A57%3A40.016Z",
  "author": "person/example",
  "text": "this agent session should be retired soon",
  "audience": "agent",
  "created_at": "2026-10-14T16:20:00Z",
  "updated_at": "2026-10-14T16:20:00Z",
  "expires_at": null,
  "revision": "claim/…"
}
```

### Minimal session subject (DQ2)

```
agent/<agent-id...> ── parent/owner ──► session/agent/<agent-id...>/<incarnation-component>
                                                 ▲
                                                 └── note.target
```

The new repository-owned subject family is `session`, with
`client_writable=false`. Its id contains the full parent agent subject and
the agent's existing authoritative `incarnation_id`, not a new UUID or a
provider-native `session_id`. The final component is the UTF-8 incarnation
id percent-encoded with uppercase hex, leaving only RFC 3986 unreserved
bytes (`A-Z a-z 0-9 - . _ ~`) literal. Split at the final slash to recover
the parent and decode that component exactly once; the encoding preserves
case and distinguishes literal `%` from an escape. A session has exactly
one immutable parent/owner agent, encoded in its id.

For example, parent `agent/example/worker` and incarnation
`1234:2026-10-14T15:57:40.016Z` yield the synthetic target in the wire shape
above. This is a subject reference derived from an existing runtime identity,
not another incarnation allocator. Parent and incarnation must match runtime
evidence at local admission; a client cannot declare a session by naming it.
Session target read access is the parent agent's read access.
Malformed session targets, like malformed note subjects and note fields, return
the typed client error code `validation-failed`, not `internal`.

A session target is current only while authoritative runtime state says its
parent agent is running with that exact incarnation. Matching a retained
incarnation id without running state is insufficient. It stops being current
on session end or replacement; its notes do not transfer to another
incarnation, including a resumed provider session with a new incarnation.
The reference and note claim history remain addressable after currency ends.

This minimal type provides only identity, parentage and currency for note
targeting and digest resolution. It adds no session declaration, lifecycle
claim, start/stop/resume API, transcript, session collection, assignment,
ownership transfer or other feature. Existing runtime and harness claims
remain authoritative; no claims beyond `note.set` are introduced here.

#### Runtime identity evidence

Read-only research against fetched `origin/main` at
`bb51bdcb0cd549b06c298d81c1907a7fd35f0c83` (2026-11-01), after the
`st3-*` → `fractal-*` crate rename:

- `crates/fractal-schema/src/lib.rs:3761–3774` defines `runtime.observed`
  with `status`, `runtime_id`, `incarnation_id` and `host`.
- `crates/fractal/src/main.rs:14662–14679` derives a running PTY incarnation
  from `pid:created_at`. `crates/fractal-kernel/src/model.rs:183–193`
  appends `:bound:<binding-id>` for a terminal-bound agent invocation.
  Therefore the complete authoritative incarnation is opaque; do not
  reconstruct it from a PID or strip a binding suffix.
- `crates/fractal-schema/src/lib.rs:3844–3858` records harness
  `session_id` alongside `agent` and `incarnation_id` in
  `harness.session-file`. Provider session identity is not runtime currency.
  `crates/fractal/src/main.rs:17046–17049` treats incarnation replacement
  or `absent`/`stopped`/`exited`/`vanished` runtime status as ended.
- The subject registry (`crates/fractal-schema/src/lib.rs:989–1211`) has
  no harness-session or incarnation family. It does include
  `assistant/chatgpt[/session/assistant-grant/ID]` (lines 1091–1095), a
  grant-bound external-assistant audit subject, and `planning-session/ID`
  (lines 1139–1143), neither of which names an agent runtime incarnation.
  Blame dates these to 2026-10-11 and 2026-09-05 respectively; registry
  history since 2026-10-14 adds no harness-session subject to reuse.

### Write rules

| Actor | May create notes on | May edit or archive |
|---|---|---|
| `person/example` | any target it can read | own notes |
| `agent/<org>/<name>` | itself; sessions whose parent is itself; mission runs and step runs it is assigned to | own notes |

The `SubjectEncodedAuthor` write policy requires the claim actor to equal
the author encoded in the note subject. This rule is stateless, analogous to
`SameSubjectActor`, and is validated in `fractal-schema` (`validate_actor`,
`validate_fields`) and again at replicated admission.

Notes are written only through `st note add|archive` and
`POST /v1/subject-notes` / `PUT /v1/subject-notes/{id}`. Generic claim
admission (`POST /v1/claims`, `st claim`) refuses `note.set` with
`claim-write-forbidden`, using the same dedicated-authorized-operation
refusal as `AuthorizedParticipant` claims. `SubjectEncodedAuthor` remains
the note's write policy; it does not grant generic claim write access.
Target access and agent assignment restrictions require graph state and are
enforced only at local note API admission. Peers trust the origin's check;
replicated admission does not enforce these restrictions (DQ8).
Person- and agent-authored notes follow the same audience rule: new notes use
`agent`; neither author type can create a private `self` note. Existing
`self` notes are not hidden from agents. The schema retains the open enum
for possible stricter levels, not a private-note mode (DQ3/DQ9).

### Read access

| Current note | Author | Other persons with target access | Resolved agents |
|---|---|---|---|
| Any audience value | yes | yes | yes |

Lists and collection windows apply note currency and target access, not an
audience filter. Digests include every current note resolving to the
recipient. Agent authors can read their own notes; no author has a separate
private-note mode.

Publication is gated by a `daemon.started` feature flag
(`features.subject_notes=1`) on every active member, using the same
`validate_publication` barrier as directive notes.

### Projection

```
subject_notes(note PK, target, author, audience, claim_id, head_key)
  index (target), index (author)
```

Built from `claims` with the same trigger, pending/dirty and
`flush`/`rebuild` structure as `person_directive_notes`. A current note has
non-null text and no passed expiry. A note targeting `session/…` additionally
requires that session subject to be current under the authoritative
runtime-state rule above; there is no separate note `session` field.

## API

| Op | Route | Result |
|---|---|---|
| `subject-notes.list` | `GET /v1/client/subject-notes?target=…` | current notes on the target, newest first |
| `subject-notes.mine` | `GET /v1/client/subject-notes?author=…` | current notes by the actor |
| `subject-notes.create` | `POST /v1/subject-notes` | created note |
| `subject-notes.update` | `PUT /v1/subject-notes/{id}` | updated note; `text: null` archives |
| collection stream | `notes` window on the existing st3-feed collection socket | inserts, updates and removals |

Create and update responses include the stable note `id` even when the
`note` is null because it is archived or no longer current. An author can
archive an expired note without making its text readable to other actors.

Clients subscribe to the `notes` window and derive counts and newest-note
previews from it. Agent and mission collection rows have no note fields.

The collection is the `notes` `Window` variant in `fractal-feed`, with `from_id`
dispatch and a server collection branch in `crates/fractal-api-client-v0/src/api/client_v0.rs`.
Expiry requires clock-follow invalidation through `collection_follows_clock`.
Runtime-state changes invalidate currency for notes targeting sessions.

## CLI

```
st note add TEXT --on TARGET [--session current] [--expires-at T]
st note ls [--on TARGET]      # notes on TARGET, or notes by the actor
st note archive NOTE_ID
st directive set|clear|show    # target: person directive note
st note                       # deprecated alias for directive show
st note set|clear             # deprecated aliases for directive set|clear
```

This is the target command split, not the current implementation. DQ1 keeps
both records: the person directive note and subject notes. `note add|ls|archive`
continues to manage subject notes; the directive note gets its own command,
`directive set|clear|show`. The `st` → `fractal` rename uses one clap CLI:
`fractal` is the binary, with `st` and `st3` as aliases. Thus
`fractal directive …` and `st directive …` expose the same command; rename
coordination with `agent/compoundingtech/fractal-rename` is not a blocker.
Bare `note` and `note set|clear` keep working as deprecated aliases.

There is no `--for` option or audience default to choose: every note is agent
context, including notes authored by a person. `--on session/…` directly
targets a session subject. `--on agent/… --session current` resolves that
agent's current authoritative running incarnation and replaces the target
with its session subject; it does not store a `session` field. The flag is
valid only with an agent target and fails if there is no current running
incarnation; it never falls back to an agent-wide note. Explicit session
targets and this shorthand use the same write rules.

## Agent context delivery

```
all current notes + recipient/runtime changes
  ──► resolve current notes per agent ──► daemon-origin, non-waking notes digest
every session start (including first incarnation) ──► same digest path
```

The daemon emits a digest when the set of current notes resolving to an
agent changes: note creation, update, archive, expiry, session end, or
recipient-set changes such as a step assignee or `lease_owner` change.
It also emits a digest at every session start of every incarnation, including
the first, when the agent has at least one current note. Digest delivery is
idempotent per `(agent, incarnation, sorted note revisions)`.
No-notes reconciliation checks for a prior daemon digest with a recipient-indexed
`SELECT 1 … LIMIT 1`; it does not materialize or sort the recipient's mailbox.

The message sender is the daemon, not the note author. Author attribution
stays in the content. The digest is non-waking, has no session-start wake tag,
and reaches the agent at its next turn (CR03/CR04). At silent release, the
daemon re-renders the digest from current notes and drops it if none remain
current. This requires a currency check in `crates/fractal-missions/src/silent.rs`.

Existing session-start hooks and `handoff::render` are not the delivery path:
the hooks emit no context, and handoff skips first incarnations. The digest
path covers every session start independently of hooks and handoff.

Target resolution: `agent/…` targets resolve to that agent. Current
`session/…` targets resolve only to their parent agent while that exact
session runs, so their notes participate in the same digest; they are not
delivered to a replacement incarnation. `mission-run/…` and `step-run/…`
targets resolve to current step assignees and lease owners. `mission/…`
targets resolve to agents with an active step in a current run of that
mission. Recipient resolution is refreshed when assignments change.

Context block format (bounded to 16 notes, newest first):

```
<st-notes>
- [note/person/example/0192…] person/example, 2026-10-14 16:20Z: wrap up and hand off; this session retires soon
</st-notes>
Notes inform you. They never assign work or grant authority.
```

Digest rendering neutralizes delimiter sequences in authored text, including
`</st-notes>`, so a note cannot terminate or introduce a context block.

## Clients

```
docs/st3/client-v0/schemas  ──codegen──►  fractal-client (Rust)  ──► fractal-tui, Fractal TUI
                                      └─► @fractal/client (TS, Effect Schema) ──► packages/fractal-ui ──► apps/fractal-web
```

Rust and Swift models require updates to the codegen templates
`crates/fractal-client-codegen/templates/generated.rs.in` and `Models.swift.in`,
plus the operation/client templates and client-v0 schema JSON. Regenerate all
SDKs after these updates; schema JSON alone does not generate those models.

- stui: a `notes` section in `agent_details` and `mission_detail`; quick input
  reuses the feedback-gate text editor (`edit.rs`) and `drafts_store.rs`.
- Fractal TUI (`schickling/dotfiles` `flakes/fractal`): already depends on
  `fractal-client` and `fractal-conversation-ui` through a pinned Fractal git rev.
  The prototype may pin that rev to the draft Smalltalk branch only on a
  non-main Fractal branch; display and quick input are prototyped in the
  `prototype` step.
- Fractal web: keep **both** entry points (Johannes, 2026-11-02; Axe request
  `c74ire`, answer `8kco2i`): the detail view shows the Notes list with
  **Add note** (V1), and unmodified lowercase `n` opens the quick-add popover
  on the selected row (V2). V1 and V2 are kept together, not alternatives.
  Both use one shared `NoteComposer`, the same target-bound draft and note
  write actions. At rest the detail view's Notes is one line (a disclosure and
  a quiet **Add note** that expands the composer in place); each sidebar row's
  actions menu also offers **Add note…**. Shortcut handling excludes editable
  fields, typeahead collections (the sidebar tree), IME composition, menus and
  dialogs, can be turned off per user, and restores focus on close.
  `packages/fractal-ui` provides presentational `NoteList`, `NoteBadge`,
  `NoteComposer` and `NoteQuickAdd` components taking the generated
  `SubjectNote` type; `apps/fractal-web` wires both entry points to the
  client. The kit imports no app code.

Every composer (stui, Fractal TUI and Fractal web) accepts plain text only,
up to 1024 UTF-8 bytes, without an audience control or tag picker. Every
note is context for the agents resolved from its target, so there is no
**Only me** / **Shared with the agent** choice. Audience-specific badges and
previews must not imply that some current notes are withheld from agents;
agent-context visibility is not a delivery acknowledgement.

## Registration checklist

A note kind registration includes:

- `retention.toml` and `fixtures/convergence/claims-v1.json`.
- Regeneration of `docs/st3/schema.md`.
- `checkpoint_rules` reset/rebuild coverage and `canonical_audit` registration.
- The `features.subject_notes=1` feature barrier flag.
- `release-notes/<slug>.json`.
- A client-v0 fixture and contract test.

## Design questions

- **DQ1 Directive note relation — resolved (2026-10-19):** Johannes chose
  `rename-only` (Axe Q4 request `ke7ykp`, answer `rpagr1`). Keep both
  `person.directive-note-set` and subject-note records separate. Give the
  directive note its own command, `directive set|clear|show`, while
  `note add|ls|archive` stays for subject notes. Bare `note` and `note set|clear`
  remain deprecated aliases. The `st` → `fractal` rename uses one clap CLI
  with `st` and `st3` aliases, so the command lands once for all binary names.
  Coordination with `agent/compoundingtech/fractal-rename` is settled and is
  not a blocker. The shared CLI implements this split.
- **DQ2 Session as target — resolved (2026-11-01):** Johannes chose sessions
  as their own subject, not a `session` field on an agent note (Axe Q5
  request `c88zos`, answer `kpn312`, relayed by Manager in
  `message/6a5ae12d025284bc`). The minimal `session` family above reuses
  the owning agent's existing incarnation id and runtime currency; no
  other session features are implied. The source implements this target design.
- **DQ3 Audience choice — resolved (2026-10-15):** No audience choice; every
  note is context for its target's resolved agents. Johannes initially
  selected "only me" but said notes need not be hidden from agents (q1,
  Axe answer `5pio7m`). He then chose no audiences and removal of the
  composer control (q2, Axe answer `4r5oi6`), superseding q1's default.
  Keep the open enum so stricter levels can be introduced if needed.
- **DQ4 Further targets and stricter levels:** `resource/…`, `custom/…`,
  `doc/…` targets and possible stricter audience levels remain questions
  for later use. No additional level is defined by DQ3.
- **DQ5 Structure beyond text — resolved (2026-10-15):** Plain text up to
  1024 UTF-8 bytes; metadata is the structure. No tags, inline tag parsing
  or tag picker (q3, Axe request `aa3i68`, answer `qwkr79`).
- **DQ6 Shared subject-ref shape with annotations:** Resolved with
  `agent/dotfiles/annotations`: Agent Provenance reuses the st graph ref shape
  for st subjects, as notes do; artifacts outside st use typed sha256
  references owned by that spec. No second subject-ref shape exists.
- **DQ7 Text limit — resolved:** 1024 UTF-8 bytes, confirmed by q3 (DQ5),
  rather than the directive note's 4096.
- **DQ8 Replicated target restrictions — resolved (2026-11-02):** Johannes
  chose dedicated-only (Axe Q2 request `ih018s`, answer `ip25da`). Notes
  are written only through the note API (`st note add|archive`,
  `POST`/`PUT /v1/subject-notes`). Generic claim admission
  (`POST /v1/claims`, `st claim`) refuses `note.set`, using the same
  dedicated-operation refusal as `AuthorizedParticipant` claims.
  `SubjectEncodedAuthor` still enforces exact authorship. Target/assignment
  checks stay at local note API admission; peers trust the origin's check,
  and replicated admission is unchanged.
- **DQ9 Agent privacy — moot under DQ3:** No private audience exists for
  either author type. Agents and persons create `agent` notes; all current
  notes are agent context regardless of their stored audience. The
  prototype's rejection of agent-authored `self` writes is not a separate
  privacy design decision.
