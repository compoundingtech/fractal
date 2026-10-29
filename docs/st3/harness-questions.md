# Harness questions

A seat's harness can ask its person a question natively: a multiple-choice prompt the harness
shows in its own interface. Fractal records that such a question exists, how it is shaped and how it
ended, so other clients can tell the question is waiting. It records **ids and positions only,
never text**. The question text, option labels, free-text answers and notes stay in the harness's
own timeline; a client that shows the question joins its text from there.

## Identity

A question is the subject `harness-question/ID`. `ID` is the first 16 bytes, in hex, of a SHA-256
over the agent subject, harness session id and canonical native call id, each length-prefixed
(`fractal_schema::harness_questions::subject`). The canonical native id is `sha256:` plus the
64 lowercase hex digits of SHA-256 over the raw native call id. The store hashes every raw
native id before deriving the subject or storing the observation; paths, command strings and
other native text are never stored verbatim as ids. The same native call in the same session
of the same seat is always the same subject.

The claim subject must match its `(agent, session, native_id)` parts. Cross-field subject
validation is not implemented by this schema-only PR; it lands with the first production
callers. Those callers must use the canonical hash before deriving the subject.

## Claims

Both kinds are system-only: only the daemon writes them, on behalf of the harness adapter.

| Kind | Fields | Meaning |
|---|---|---|
| `harness-question.observed` | `agent`, `session`, `native_id`, `runtime_incarnation`, `questions` | the call as the harness shows it. Each question is `{id, options, multi, required}`: its id, its number of options, whether it takes several and whether it needs an answer |
| `harness-question.resolved` | `status`, `revision`, `answers`, `receipt` (optional) | how the call ended: `answered`, `answered_elsewhere`, `expired` or `cancelled`; the revision it settled; per question id, `{positions, timed_out}`; and the `harness.answer` request that settled it, as two digests and its action id |

A call has 1 to 16 questions, a question at most 64 options. Question ids and answer-map keys
must be canonical zero-based positional ids `q0` through `q15`, or `sha256:` followed by the
64 lowercase hex digits of a hash of the native question id. They must never be native
question text, paths or commands. Session and incarnation ids remain bounded opaque ids.
There are no stored option ids: unique answer positions directly index the native option
order, so position-to-option mapping never depends on a question or option id.
`timed_out` is the harness's own per-question flag.

**Revision.** A question's content revision is the ID of its newest observation. The harness
observing the same content again writes nothing, so the revision holds; new content (another
option count, another runtime incarnation) is a new observation and a new revision. Settling the
question writes the resolution and leaves the revision unchanged. A resolution names the revision
it settled and is written only while that is still the newest observation.
An observation is fenced on the revision inspected before appending (or on absence for a new
question), so a concurrent resolution in the same store cannot be followed by new content.

The question reader (`Store::harness_question`) is the only reader contract for these subjects.
The generic subject views (claims, actual state) show the claims while the store holds them.

**Once, locally.** The write-once resolution check is local, not a distributed compare-and-set.
A second local resolution returns the first; a resolved question takes no new observation.
Two disconnected writers can both return success before seeing each other's resolution.
Replay converges to the first resolution in canonical claim order. The losing writer has
already reported local success; after replication its reads show the winner's status,
positions and receipt, not its own. A resolution retry returns that winning record with
`written = false`; its losing receipt no longer satisfies `settled_by`. This is not proof
that a native answer was delivered only once. Cross-writer native settlement is work for the
production caller PR, not implemented here.

## Retention

A pending question is kept whole until it ends. The owner's compact record — question id,
status, answer positions and receipt — is retained indefinitely and never hidden by age,
therefore at least as long as the native timeline entry, even if that entry later expires.
The newest observation (identity and option shape) and the resolution stay in the claim log
and in `Store::harness_question` forever.

Only redundant observation history may expire: a checkpoint whose cut is at least one day
after resolution (`RESOLVED_RETENTION_MS`) keeps the newest observation and drops older
observations, subject to the ordinary checkpoint guards. No text or bulky harness timeline
data is stored in these claims. Independently derived bulky data may expire, but never the
compact owner record.

The rule `all-while-pending,newest-once-resolved-1d-before-cut` uses rules version 18,
following #1473's allocated version 17 and main's review-reader version 16. Whichever PR
lands second must renumber in a plain follow-up commit. The proof compares the same compact
question before and after compaction. Resolved questions never become pending again: re-observation of the
same native call is refused even after history expiry or checkpoint trimming, including when
the resolution status is `expired`. Its original resolution and receipt remain authoritative.

A question stays pending until the harness writes its end. The harness adapter writes `expired`
or `cancelled` when the harness gives up on or withdraws a question; Fractal does not end a pending
question by itself.

## Client reads and person answers

`harness-questions.list` (`GET /v1/client/harness-questions`) and
`harness-questions.get` (`GET /v1/client/harness-questions/{id}`) require
`read.projections`. They expose metadata only, in the ordinary client-v0 envelope:
the agent, opaque native session, hashed native call id, runtime incarnation, question
ids, option counts, selection constraints and any resolution. No text, option labels,
notes or internal receipt crosses these resources. The web view joins text from the
existing raw native timeline; a missing or ambiguous join must not enable answering.

The list uses the existing cached, snapshot-bound pagination with `limit` and `cursor`
only (default 50, maximum 200). Detail returns one `harness-question` resource. Its
`revision` is the observation revision, unchanged by settlement; `updated_at` is an
RFC 3339 UTC timestamp. `observed_at` and the resolution's `resolved_at` are integer
Unix epoch milliseconds. A resolution contains `status`, the observation `revision`,
and per-question `{positions, timed_out}`. The native harness owns each `timed_out`
flag. Compact resolved records and receipts remain visible indefinitely.

Every resource has `answer.state` (`available` or `unavailable`); unavailable reasons
are `adapter_missing`, `adapter_unavailable` or `resolved`. This describes adapter
availability, not caller permission. Global capability `harness.answer`, version 0,
uses the normal `{id, version, state}` capability shape, without a `reason` field.
No adapter is installed by default; capability is unavailable and submission returns
`not-answerable` with `details.reason: adapter_missing`, without writing a receipt.
Every per-harness flag and the web-client flag remain off; this contract does not wire
the O1 adapter or ship an answering form.

`harness.answer` posts to `/v1/client/actions` with parameters
`{question, answers}`, where `answers` maps canonical `q0`–`q15` or hashed question
ids to `{positions:[0,2]}`. Positions are unique and zero-based in native option order.
A required question needs an answer; single-select needs exactly one position.
No question text, option ids, Other, notes or client-supplied `timed_out` is accepted.
Answer-map keys are data, never identity selectors; identity fields elsewhere remain
forbidden. The action id is at most 256 UTF-8 bytes, checked before native settlement.

The action explicitly requires `terminal.control` and the authenticated concrete
person's authority over the selected live local terminal runtime, using terminal
input's authority check. Every agent actor is refused, including Unix free-mode agents
and agents carrying `terminal.control`. Neither an availability flag nor a person id
in the body grants authority. Unknown `harness.*` actions receive no scope or advertised
capability. Remote harness-answer relay and cross-writer exactly-once settlement are
not provided.

Read detail before submitting. Echo its envelope `snapshot.id`, resource `revision`
under `fence.subject_revisions[question]`, and `runtime_incarnation`. No terminal screen
sequence is needed. Snapshot provenance and current live-runtime authority are checked
before replaying a receipt. A changed observation or runtime refuses with `stale-fence`;
unrelated store writes do not invalidate snapshot provenance.

Keep the same action id, key and parameters for an uncertain retry; the request digest
excludes the fence, so fresh fences do not change request identity. Exact retained
retries return `harness_answer.status: accepted` without settling again. Reusing an
actor's key with another id, type or parameters returns `idempotency-conflict`,
including reuse across generic client actions, private-notes writes and harness answers
in either direction. Private-notes writes retain their owner-local exact-request recovery,
current ownership checks and fence validation; the shared graph reservation never replays
private content or private fences.
Harness receipts stay solely in the existing compact resolution. An unknown key is
new; a new key after resolution returns `already_resolved` before adapter or answer
validation, but still requires current authority, snapshot provenance and question/runtime
fences. The result is an ordinary completed action with a nested `harness_answer`
containing its status and the authoritative resolution. `accepted` acknowledges adapter
settlement; it does not prove the harness resumed or completed a later turn.

Receipt replay and key occupancy are owner-local authority, not replicated metadata.
Generic receipts come from reserved `client.action-result` local observations with the
local origin and no actor; any legacy `custom.client.action-result` for the key refuses
with `legacy-action-receipt-untrusted` before settlement. Harness receipts remain in the
already-reserved, system-only compact resolution: only an originating local resolution
with no actor can authorize replay or reserve a local key. Peer resolutions remain
readable as terminal metadata but never become local replay authority. This reads existing
claim provenance; it changes no stored format, retention rule or replication policy.

Invalid selections return `invalid-answer` and typed `details.reason`:
`unknown_option`, `too_many`, `required_missing`, `empty_single`, `too_long`,
`unknown_question`, `other_not_allowed` or `note_not_allowed`. Malformed envelopes
return `validation-failed`. `not-answerable` carries `adapter_missing` or
`adapter_unavailable`. These are non-retryable refusals, not successful answers.

## CLI

Use an explicit concrete person for reads and answers:

```sh
fractal harness-questions list --as person/alex
fractal harness-questions list --as person/alex --limit 50 --cursor CURSOR
fractal harness-questions show QUESTION --as person/alex
fractal harness-questions answer QUESTION --as person/alex \
  --answers-file answers.json --idempotency-key alex-harness-answer-0001
```

List prints one page. Continue with its `page.next_cursor`, preserving `--limit`.
The generated Rust, Swift and TypeScript list helpers expose the same cursor and limit
controls; callers must follow `next_cursor` when `has_more` is true.

`answers.json` contains only a map such as `{"q0":{"positions":[1]}}`.
The CLI reads canonical detail to populate all three fences. Its action id is
`action/harness-answer-` plus SHA-256 hex over the key and canonical question id,
each prefixed by its UTF-8 byte length as a big-endian u64. Retain the file and key
to reproduce the same action identity. Rust, Swift and TypeScript clients are generated
from the shared schema and templates with `cargo run -p fractal-client-codegen`.

This is an additive client contract, not a new stored schema, receipt format or
retention rule. Enabling an adapter or web answering remains a separate gate: prove the
fail-closed native timeline join, render each error code deliberately, show `accepted`
only as acknowledged, and settle the deployment/authority/relay boundaries before
enabling any flags.
