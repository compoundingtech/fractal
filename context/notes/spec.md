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

The directive/subject-note CLI split is not implemented yet. Rename coordination
is settled: `fractal` and its `st`/`st3` aliases use the same clap CLI, so the
new top-level command will be available through all three binary names.

Client regeneration and verification are deferred until dev4 is cleared.
Run `cargo run -p st3-client-codegen` first, commit the generated output
separately, then run the codegen `--check` and the notes acceptance checks.

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
    target     SubjectReference   families: agent, mission, mission-run, step-run
    text       string | null      nonblank, ≤ 1024 bytes; null = archive
    audience   open enum          known values: "self", "agent"
    session    string?            incarnation id; only when target is agent/…
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
(`crates/st3-client-codegen/src/rich.rs:132–154,584–588`), not a closed
string enum (`rich.rs:593–596`). Known wire values remain `self` and `agent`,
but all new notes use `agent`, regardless of author. Audience is not a user
choice or an active privacy boundary: every current note, including an
existing `self` note, is context for its resolved agents. The open enum
preserves room for stricter levels later; it does not introduce them now.
Readers render unrecognized values as a neutral label, never 'Unknown',
and do not exclude them from agent context on the basis of audience.

Text has no tag field, tag parsing or tag picker. Author, target, optional
session, timestamps and archived state supply the structure (DQ5).

Wire shape (client-v0, generated into Rust, TypeScript and Swift):

```json
{
  "id": "note/person/example/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d",
  "target": "agent/compoundingtech/st-notes",
  "author": "person/example",
  "text": "this agent session should be retired soon",
  "audience": "agent",
  "session": "1682111:2026-10-14T15:57:40.016Z",
  "created_at": "2026-10-14T16:20:00Z",
  "updated_at": "2026-10-14T16:20:00Z",
  "expires_at": null,
  "revision": "claim/…"
}
```

### Write rules

| Actor | May create notes on | May edit or archive |
|---|---|---|
| `person/example` | any target it can read | own notes |
| `agent/<org>/<name>` | itself; mission runs and step runs it is assigned to | own notes |

The new `SubjectEncodedAuthor` write policy requires the claim actor to equal
the author encoded in the note subject. This rule is stateless, analogous to
`SameSubjectActor`, and is validated in `st3-schema` (`validate_actor`,
`validate_fields`) and again at replicated admission.

Target access and agent assignment restrictions require graph state and are
enforced only at local API admission. Replicated admission does not enforce
these restrictions; see `DQ8`.
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
subject_notes(note PK, target, author, audience, session, claim_id, head_key)
  index (target), index (author)
```

Built from `claims` with the same trigger, pending/dirty and
`flush`/`rebuild` structure as `person_directive_notes`. A current note has
non-null text and no passed expiry. A session-scoped note is current only
when authoritative runtime state says the target agent is running with that
incarnation; incarnation-id equality alone is not sufficient.

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

The collection is the `notes` `Window` variant in `st3-feed`, with `from_id`
dispatch and a server collection branch in `crates/st3/src/api/client_v0.rs`.
Expiry requires clock-follow invalidation through `collection_follows_clock`.
Runtime-state changes invalidate session-scoped note currency.

## CLI

```
st note add TARGET TEXT [--session current|ID] [--expires-at T]
st note ls [TARGET]           # notes on TARGET, or notes by the actor
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
context, including notes authored by a person. `--session current` resolves
the target agent's current incarnation.

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

The message sender is the daemon, not the note author. Author attribution
stays in the content. The digest is non-waking, has no session-start wake tag,
and reaches the agent at its next turn (CR03/CR04). At silent release, the
daemon re-renders the digest from current notes and drops it if none remain
current. This requires a currency check in `crates/st3/src/silent.rs`.

Existing session-start hooks and `handoff::render` are not the delivery path:
the hooks emit no context, and handoff skips first incarnations. The digest
path covers every session start independently of hooks and handoff.

Target resolution: `agent/…` targets resolve to that agent. `mission-run/…`
and `step-run/…` targets resolve to current step assignees and lease owners.
`mission/…` targets resolve to agents with an active step in a current run of
that mission. Recipient resolution is refreshed when assignments change.

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
docs/st3/client-v0/schemas  ──codegen──►  st3-client (Rust)  ──► stui, Fractal TUI
                                      └─► st3-client (TS, Effect Schema) ──► packages/fractal-ui ──► apps/fractal-web
```

Rust and Swift models require updates to the codegen templates
`crates/st3-client-codegen/templates/generated.rs.in` and `Models.swift.in`,
plus the operation/client templates and client-v0 schema JSON. Regenerate all
SDKs after these updates; schema JSON alone does not generate those models.

- stui: a `notes` section in `agent_details` and `mission_detail`; quick input
  reuses the feedback-gate text editor (`edit.rs`) and `drafts_store.rs`.
- Fractal TUI (`schickling/dotfiles` `flakes/fractal`): already depends on
  `st3-client` and `st3-conversation-ui` through a pinned Smalltalk git rev.
  The prototype may pin that rev to the draft Smalltalk branch only on a
  non-main Fractal branch; display and quick input are prototyped in the
  `prototype` step.
- Fractal web: `packages/fractal-ui` gets presentational `NoteList`,
  `NoteBadge` and `NoteComposer` components that take the generated
  `SubjectNote` type as props; `apps/fractal-web` wires them to the client.
  The kit imports no app code.

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
  not a blocker. This CLI split is not implemented yet.
- **DQ2 Session as target:** No `session` subject family exists; harness
  sessions appear only as `incarnation_id`/`session_id` fields. This spec uses
  a `session` field on agent-targeted notes. An alternative is a new
  `session/…` family. Resolved by whether other features need sessions as
  first-class subjects.
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
- **DQ8 Replicated target restrictions:** Must replicated admission enforce
  agent target/assignment restrictions too? This requires
  `AuthorizedParticipant` with ownership evidence; the draft enforces these
  graph-state restrictions only at local API admission.
  The generic claim API (`POST /v1/claims`, `st claim`) applies only
  `SubjectEncodedAuthor`; target restrictions apply only through the note
  API.
- **DQ9 Agent privacy — moot under DQ3:** No private audience exists for
  either author type. Agents and persons create `agent` notes; all current
  notes are agent context regardless of their stored audience. The
  prototype's rejection of agent-authored `self` writes is not a separate
  privacy design decision.
