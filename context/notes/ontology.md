# Notes ontology

Status: Draft. Audience and text terminology reflects Johannes's 2026-10-15
interview (spec DQ3/DQ5); session terminology reflects his 2026-11-01
decision (DQ2). Alignment with `agent/dotfiles/annotations` remains.

## Language

- **Note**: A short, authored plain text that a person or an agent attaches
  to one st subject. Its metadata supplies the structure; it has no tags.
  A note informs. It never assigns work, approves a gate, or grants authority.
  _Avoid_: comment, memo, sticky, annotation (see below).
- **Target**: The st subject that a note is attached to, for example
  `agent/<org>/<name>`, `session/…` or `mission/<id>`. A note has exactly
  one target.
- **Author**: The `person/…` or `agent/…` actor encoded in the note id and
  signing its claims. Every claim actor must match this immutable author;
  the author is never a free field.
- **Audience**: Who a note is for. Every note is context for the agents
  resolved from its target; there is no audience choice or private-note mode.
- **Agent context**: Informational notes delivered to resolved agents without
  waking them. Being context does not acknowledge delivery or grant authority.
- **Session**: One runtime incarnation of a parent/owner agent, addressed
  as its own subject using that agent's existing incarnation identity.
  It is current only while authoritative runtime state says the parent
  is running with that exact incarnation; it is not a provider-native
  session or a planning session.
- **Session note**: A note whose target is a session subject, not an
  agent note with an extra scope field. It is context for the parent agent
  only while that session is current; it never carries into a replacement.
- **Archived note**: A note whose author has closed it. The claim history keeps
  it; current views and agent context omit it.
- **Person directive note**: The existing `person.directive-note-set` record:
  one current note per person, read by the agents that work for that person.
  It has no target and remains a separate record from subject notes, with its
  own command, `directive set|clear|show` (see spec `DQ1`). The command split
  is not implemented yet; bare `note` and `note set|clear` remain deprecated
  aliases. `fractal` and its `st`/`st3` aliases share the same CLI command.
- **Annotation**: Umbrella term, owned by `agent/dotfiles/annotations`, for
  structured metadata attached to artifacts. Its record is **Agent
  Provenance** (`agent-provenance`): who/why/where facts about agent-made
  artifacts (git trailers, PR footers, receipts, st claims). Artifacts are
  addressed by typed sha256 references; st subjects use st graph refs as-is.
  Not a note.

## Structure

```
person/… ─┐                         ┌─ agent/<org>/<name>
          ├─ author ─ note ─ target ─┼─ session/… ─ parent ─ agent/…
agent/… ──┘                         ├─ mission/<id>
                                     ├─ mission-run/<id>
                                     └─ step-run/<gen>/<path>

agent provenance ─ target ─ artifact (typed sha256 ref) or st graph ref
```

Leitwort: a note is *about* its target and *context for* its resolved agents.
