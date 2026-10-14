# Notes spec

This document specifies st notes: short authored texts that a person or an
agent attaches to one st subject, shown in stui, the Fractal TUI and Fractal
web, and delivered as agent context when the audience includes the agent. Terms
are defined in [ontology.md](./ontology.md).

## Status

Draft. `vision.md` and `requirements.md` do not exist yet. They become
authoritative only after Johannes confirms them in the interview. The
candidate requirements below are input for that interview, not constraints.

## Scope

Defines: the note resource, its claim, its projection, the HTTP API, the CLI,
agent-context delivery, and the shared client schema for stui, the Fractal TUI
and Fractal web.

Does not define: git and pull-request annotations (owned by
`agent/dotfiles/annotations`); the UX variants (documented in the
`ux-variants` step); the existing person directive note, which stays as
specified in `docs/st3/schema.md`.

## Candidate requirements (pending interview)

- **CR01 Low input overhead:** Adding a note to the selected agent or mission
  takes one key or click, the text, and one confirm. No form, no required
  field other than the text.
- **CR02 Visible where the target is shown:** A current note is visible
  wherever its target is shown (agent row/details, mission row/details),
  without opening a separate view.
- **CR03 Agent context:** An `agent`-audience note reaches the target agent at
  its next turn and at every session start while the note is current.
- **CR04 Inform only:** A note never assigns work, approves a gate, wakes an
  agent for a full turn by itself, or grants authority.
- **CR05 Shared schema:** stui, the Fractal TUI and Fractal web read notes
  through one generated client schema; no client defines its own note shape.
- **CR06 Short:** Note text is at most 1024 UTF-8 bytes.
- **CR07 Reactive:** Clients see a new note through their existing collection
  subscription, not through polling.

## Model

```
note/<UUIDv7>                      subject family "note" (new)
  claim note.set   (StateTransition, Durable, write policy: author)
    target     SubjectReference   families: agent, mission, mission-run, step-run
    text       string | null      nonblank, ≤ 1024 bytes; null = archive
    audience   "self" | "agent"
    session    string?            incarnation id; only when target is agent/…
    expires_at string?            UTC RFC3339, like the directive note
  actor  = author (person/… or agent/…)
  accepted_at = time
```

A note is its own subject so that each note has a stable identity, its own
history, and can be referenced from messages, mission evidence or later
annotations. This follows `arrangement/person/NAME/UUIDv7`, the existing
pattern for client-created resources.

`audience` is a string tag rather than a boolean so that further audiences
(for example a mission's whole crew, see `DQ4`) can be added without a schema
break.

Wire shape (client-v0, generated into Rust, TypeScript and Swift):

```json
{
  "id": "note/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d",
  "target": "agent/compoundingtech/st-notes",
  "author": "person/example",
  "text": "this agent session should be retired soon",
  "audience": "self",
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
| `agent/A` | itself; missions, mission runs and step runs it is assigned to | own notes |

Rules are enforced in `st3-schema` (`validate_actor`, `validate_fields`) and
again at replicated admission, as for `person.directive-note-set`.

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
non-null text, no passed expiry, and, when session-scoped, a session that is
still the target agent's current incarnation.

## API

| Op | Route | Result |
|---|---|---|
| `subject-notes.list` | `GET /v1/client/subject-notes?target=…` | current notes on the target, newest first |
| `subject-notes.mine` | `GET /v1/client/subject-notes?author=…` | current notes by the actor |
| `subject-notes.create` | `POST /v1/subject-notes` | created note |
| `subject-notes.update` | `PUT /v1/subject-notes/{id}` | updated note; `text: null` archives |
| collection stream | `notes` window on the existing st3-feed collection socket | inserts, updates and removals |

The `agents` and `missions` collection rows gain `note_count` and the newest
note's text so that list rows show a note without a second request (CR02).

## CLI

```
st note add TARGET TEXT [--for me|agent] [--session current|ID] [--expires-at T]
st note ls [TARGET]           # notes on TARGET, or notes by the actor
st note archive NOTE_ID
st note                       # unchanged: person directive notes
st note set|clear             # unchanged: person directive notes
```

`--for` defaults to `me` (DQ3). `--session current` resolves the target
agent's current incarnation.

## Agent context delivery

```
note.set (audience=agent) ──► projection ──┬─► silent message to target agent
                                           │   (reaches it at next turn, CR03/CR04)
                                           └─► session-start context block
                                               (hooks additionalContext,
                                                pi session_context,
                                                handoff::render)
```

Target resolution: `agent/…` targets resolve to that agent. `mission-run/…`
and `step-run/…` targets resolve to the step assignees. `mission/…` targets
resolve to agents with an active step in a current run of that mission.

Context block format (bounded to 16 notes, newest first):

```
<st-notes>
- [note/0192…] person/example, 2026-10-14 16:20Z: wrap up and hand off; this session retires soon
</st-notes>
Notes inform you. They never assign work or grant authority.
```

## Clients

```
docs/st3/client-v0/schemas  ──codegen──►  st3-client (Rust)  ──► stui, Fractal TUI
                                      └─► st3-client (TS, Effect Schema) ──► packages/fractal-ui ──► apps/fractal-web
```

- stui: a `notes` section in `agent_details` and `mission_detail`; quick input
  reuses the feedback-gate text editor (`edit.rs`) and `drafts_store.rs`.
- Fractal TUI (`schickling/dotfiles` `flakes/fractal`): already depends on
  `st3-client` and `st3-conversation-ui` through a pinned Smalltalk git rev.
  The prototype pins that rev to the draft Smalltalk branch; display and quick
  input are prototyped in the `prototype` step.
- Fractal web: `packages/fractal-ui` gets presentational `NoteList`,
  `NoteBadge` and `NoteComposer` components that take the generated
  `SubjectNote` type as props; `apps/fractal-web` wires them to the client.
  The kit imports no app code.

## Design questions

- **DQ1 Directive note relation:** Keep `person.directive-note-set` separate,
  or express it as a note with target `person/example` and audience `agent`?
  Resolved by the interview; the prototype keeps it separate.
- **DQ2 Session as target:** No `session` subject family exists; harness
  sessions appear only as `incarnation_id`/`session_id` fields. This spec uses
  a `session` field on agent-targeted notes. An alternative is a new
  `session/…` family. Resolved by whether other features need sessions as
  first-class subjects.
- **DQ3 Default audience:** `self` (safe, quiet) or `agent` (more useful,
  reaches the agent)? Johannes's example, "this agent session should be
  retired soon", reads as `self`. Resolved by the interview.
- **DQ4 Further targets and audiences:** `resource/…`, `custom/…`, `doc/…`
  targets and a mission-crew audience. Resolved after first use.
- **DQ5 Structure beyond text:** "Structured" may mean an optional tag
  (for example `retire`, `watch`, `context`) that clients can filter and
  render. Not in the draft; resolved by the interview.
- **DQ6 Shared subject-ref shape with annotations:** Whether notes and git/PR
  annotations share one target-reference schema. Resolved with
  `agent/dotfiles/annotations`.
- **DQ7 Text limit:** 1024 bytes (CR06) versus the directive note's 4096.
