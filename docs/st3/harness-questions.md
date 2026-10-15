# Harness questions

A seat's harness can ask its person a question natively: a multiple-choice prompt the harness
shows in its own interface. st records that such a question exists, how it is shaped and how it
ended, so other clients can tell the question is waiting. It records **ids and positions only,
never text**. The question text, option labels, free-text answers and notes stay in the harness's
own timeline; a client that shows the question joins its text from there.

## Identity

A question is the subject `harness-question/ID`. `ID` is the first 16 bytes, in hex, of a SHA-256
over the agent subject, the harness session id and the harness's native id for the call, each
length-prefixed (`st3_schema::harness_questions::subject`). The same native call in the same
session of the same seat is always the same subject; nothing else is.

## Claims

Both kinds are system-only: only the daemon writes them, on behalf of the harness adapter.

| Kind | Fields | Meaning |
|---|---|---|
| `harness-question.observed` | `agent`, `session`, `native_id`, `runtime_incarnation`, `questions` | the call as the harness shows it. Each question is `{id, options, multi, required}`: its id, its number of options, whether it takes several and whether it needs an answer |
| `harness-question.resolved` | `status`, `revision`, `answers`, `receipt` (optional) | how the call ended: `answered`, `answered_elsewhere`, `expired` or `cancelled`; the revision it settled; per question id, `{positions, timed_out}`; and the `harness.answer` request that settled it, as two digests and its action id |

A call has 1 to 16 questions, a question at most 64 options, and ids at most 128 bytes without
whitespace or control characters. An answer names unique positions below the question's option
count. `timed_out` is the harness's own per-question flag.

**Revision.** A question's content revision is the ID of its newest observation. The harness
observing the same content again writes nothing, so the revision holds; new content (another
option count, another runtime incarnation) is a new observation and a new revision. Settling the
question writes the resolution and leaves the revision unchanged. A resolution names the revision
it settled and is written only while that is still the newest observation.
An observation is fenced on the revision inspected before appending (or on absence for a new
question), so a concurrent resolution cannot be followed by an observation of new content.

The question reader (`Store::harness_question`) is the only reader contract for these subjects.
The generic subject views (claims, actual state) show the claims while the store holds them.

**Once.** A question resolves once. A second resolution returns the first; a resolved question
takes no new observation.

## Retention

A pending question is kept whole until it ends. Once it is resolved, it and its answer receipt
stay for a day (`RESOLVED_RETENTION_MS`, 24 hours) after the resolution was accepted:

- readers treat a question resolved at least a day ago as gone (`Store::harness_question` with
  the read time);
- the checkpoint rule `all-while-pending,none-once-resolved-1d-before-cut` (rules version 16)
  drops every claim of a question whose resolution was accepted at least a day before the cut.
  The proof compares each question as of the cut, before and after the drop.

A resolved question is therefore gone to readers a day after it ended and its claims leave the
store at the first stable checkpoint whose cut is a day past the resolution, which can be later.
There are no tombstones for answer keys: once a question is gone, a request naming it finds no
question.

A question stays pending until the harness writes its end. The harness adapter writes `expired`
or `cancelled` when the harness gives up on or withdraws a question; st does not end a pending
question by itself.

## Client reads and answering

`harness-questions.list` and `harness-questions.get` expose question metadata and the resolution,
without question text, option labels or the internal receipt. The web view joins text from the raw
timeline. Each resource has `answer.state` and, when unavailable, a typed reason:
`adapter_missing`, `adapter_unavailable` or `resolved`.

`harness.answer` takes `{question, answers}`, where `answers` maps question ids to
`{positions: [0, 2]}`. Positions are zero-based. No text, Other value or note is accepted.
The action requires `terminal.control` and the authenticated person's authority over the
agent's live runtime. Every agent actor is refused, including agents in Unix free mode.

The fence carries the question's content revision in `subject_revisions[question]` and its
`runtime_incarnation`. Snapshot provenance and live-runtime authority are checked before
replaying a receipt. The content and incarnation fences prevent an answer from reaching a
changed question or restarted runtime.

An accepted answer returns `harness_answer.status: accepted` and the resolution, including the
harness's native `timed_out` for every answer. An identical retry uses the same action id,
idempotency key and parameters. Reusing a key with another action id or different parameters
returns `idempotency-conflict`. An unknown key is new; a new key for a resolved question returns
`already_resolved` before adapter availability or answer validation. Receipts live only inside
the resolution claim and expire with it, not in the generic action receipt store.

Invalid answers return `invalid-answer` with typed `details.reason`: `unknown_option`,
`too_many`, `required_missing`, `empty_single`, `too_long`, `unknown_question`,
`other_not_allowed` or `note_not_allowed`. Malformed action envelopes continue to return
`validation-failed`. This is an additive client contract.

The daemon's settle sink is injectable at its boundary. No harness adapter is wired by this
contract: by default availability reports `adapter_missing` and answering returns that typed
`not-answerable` refusal before any claim or receipt is written.

## CLI

Read metadata with `st harness-questions list --as person/ada` or
`st harness-questions show QUESTION --as person/ada`. To answer:

```sh
st harness-questions answer QUESTION --as person/ada \
  --answers-file answers.json --idempotency-key harness-answer-retry-key
```

`answers.json` contains a map such as `{"q1":{"positions":[0]}}`. Keep the same file and key
for retries. The CLI derives the action id from the length-prefixed key and question id, so the
printed or recorded key is sufficient to recover the action identity; it reads the current
question metadata to populate the content and incarnation fences.
