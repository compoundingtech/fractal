# Seat handoff

A seat's work lives in the graph, but a session's understanding does not. When a seat restarts and
the harness cannot continue the earlier conversation, the new session knows only what st gives it.
The handoff is that gift: one document per seat, written by the seat, and a first message that
puts it in front of the next session together with the live state of the seat's work.

Same-session resume (`claude --resume`, `codex resume`) stays preferred wherever it works. The
handoff covers the cases where it does not: a move to another account or model, a fresh session
after a declaration change, a seat killed without warning.

## The document

Each seat has one document, `doc/agents/IDENTITY/handoff`, where `IDENTITY` is the seat's name
without `agent/`. It is an ordinary versioned document, so every earlier handoff stays readable.

```sh
st agents handoff write --as "$ST_AGENT" <<'TEXT'
Reviewing the widget change (branch agent/widget, PR 12). Waiting on CI.
Next: address the review comment about retries, then ask the curator to queue it.
A watch on PR 12 is mine; nothing else is running.
TEXT
st agents handoff show
```

`write` replaces the handoff from standard input or a file. `show` prints the latest, for this seat
or any named seat. Only a seat writes its own: the daemon refuses a `doc/agents/…` write from a
seat bound to another identity, because the next session reads the document as its own seat's
words. A person can write any document.

A handoff carries what the graph does not: what the seat was in the middle of, the pull requests
and branches it owns, what it waits on, the next steps, and anything it started that still runs. A
step's progress notes cover the step; the handoff covers the seat. A seat killed without warning
leaves only what it wrote before, so it keeps the handoff current as it works.

## What a new incarnation is sent

When a seat's new incarnation becomes ready, the daemon sends it one message from `daemon/runtime`,
"Session restarted: where you were". It holds

- the latest handoff, with its age and an immutable reference (a handoff over 16 KiB is cut and
  names the reference to read the rest);
- each unfinished step the seat holds or was working on, with its goals and last progress note (a ready step nobody has worked on is announced by its own ready-step message and is left out);
- the seat's unread mail, by sender and title.

The message carries the reserved tag `st3-session-start:INCARNATION`, so it always wakes the seat
and no seat can send one. It is written once per incarnation: the record is the message's own
idempotency key, so a daemon restart or a repeated pass sends nothing more.

A seat with no handoff and no unfinished steps is sent nothing. Neither is a seat's first
incarnation, which has no predecessor, nor one the daemon first finds running more than ten
minutes after it started (a daemon that was down, or the release that added this): such a session
already has its context, and an upgrade does not message every running seat. The daemon looks at
each incarnation once and writes only when it sends, so a settled fleet costs nothing per pass.

A session that continues the earlier conversation receives the message too. It says so: such a
session already knows what it describes.

## Tests

- `crates/st3/src/handoff.rs`: naming, write authority, and what the message renders.
- `reconcile::tests::a_new_incarnation_is_first_sent_the_handoff_its_seat_left`: the first
  incarnation is told nothing, its replacement is told once, and an empty seat is told nothing.
