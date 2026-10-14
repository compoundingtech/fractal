# Notes ontology

Status: Draft. Terms are proposed for Johannes's interview and for alignment
with `agent/dotfiles/annotations`.

## Language

- **Note**: A short, authored, human-readable text that a person or an agent
  attaches to one st subject. A note informs. It never assigns work, approves a
  gate, or grants authority.
  _Avoid_: comment, memo, sticky, annotation (see below).
- **Target**: The st subject that a note is attached to, for example
  `agent/<org>/<name>` or `mission/<id>`. A note has exactly one target.
- **Author**: The `person/…` or `agent/…` actor that signed the claim that
  created the note. The author is the claim actor; it is never a free field.
- **Audience**: Who the note is written for. `self` means the author's own
  reminder. `agent` means the note is also delivered as context to the agent
  that the target resolves to.
- **Session scope**: An optional narrowing of an agent-targeted note to one
  harness session (incarnation). A session-scoped note stops applying when
  that session ends.
- **Archived note**: A note whose author has closed it. The claim history keeps
  it; current views and agent context omit it.
- **Person directive note**: The existing `person.directive-note-set` record:
  one current note per person, read by the agents that work for that person.
  It has no target. It stays a separate concept (see spec `DQ1`).
- **Annotation**: Machine or agent metadata attached to git and pull-request
  objects. Owned by `agent/dotfiles/annotations`. Not a note.

## Structure

```
person/… ─┐                  ┌─ agent/<org>/<name> [session scope?]
          ├─ author ─ note ─ target ─┼─ mission/<id>
agent/… ──┘        (audience)        ├─ mission-run/<id>
                                     └─ step-run/<gen>/<path>

annotation ─ target ─ git/PR object      (separate system, shared subject-ref shape)
```

Leitwort: a note is *about* its target and *for* its audience.
