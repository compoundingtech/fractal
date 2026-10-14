# Running st with omp

An omp seat is an ordinary st agent. Declare its workspace, harness, model, and effort in KDL:

```kdl
version 2
agent "worker" {
  workspace "/srv/example/worker"
  harness "omp" {
    model "openai-codex/MODEL"
    effort "medium"
  }
}
```

Before the first launch of an installed omp build, st measures its delivery contract in a disposable
session and refuses to launch a build that fails ([harness admission](seat-deploys.md#harness-admission-at-the-next-launch)).
st supplies the channel extension, session directory, and boot prompt. The launcher runs under `bun`, which must be available in the daemon's
login-shell environment. The seat uses the provider login of the user running the daemon. Confirm
the selected model in the session transcript; omp may resolve an unavailable model name to another
one.

The session JSONL is stored in st's driver state for the seat. The `model_change` entry and each
assistant turn identify the selected provider and model. omp's Python `eval` tool has a filtered
environment; run st commands from its shell tool so `ST_AGENT`, `ST3_BIN`, and `ST3_ENDPOINT` are
available.

## Driver launch and native binding

The managed seat enters the shared `omp_session::run_native` launcher in process.
The in-process OMP extension starts `driver omp-channel` on the executable named by
`ST_OMP_CHANNEL_BIN`. The shared wrapper uses
`ST_OMP_CHANNEL_{BIN,CATALOG,IDENTITY,RUNTIME_ID,SESSION,SEQ}` for its channel
configuration; `ST_OMP_CHANNEL_EXPECTED_NATIVE_SESSION` and
`ST_OMP_CHANNEL_RESUME_GENERATION` fence a cold residency resume.

`session_start` sends the native OMP session ID before accepting Rust's hello.
A matching ready frame completes the two-way exchange. A cold resume must prove the
expected native ID, wrapper incarnation and residency generation before its candidate
binding becomes authoritative. A later in-process session switch replaces the named
predecessor and binds the new session. When OMP exposes its session-file getter, the
session and ready frames also carry that exact JSONL path for transcript readers.

## Exact-build admission

An ordinary launch parses OMP's own unambiguous `MAJOR.MINOR.PATCH` release and measures
its exact executable, interpreter, installation metadata and shipped extension identity.
A residency resume validates its native checkpoint before even starting the version child;
a missing, stale, foreign or changed binding refuses the launch. There is no minor-version
allowlist. A prerelease or build-metadata banner is not admitted as its base release.

Every unseen identity must pass five checks in a disposable RPC session:

1. Load the shipped extension and recorder, exercise their API calls, and establish a real
   native channel ready binding.
2. Observe session, agent, turn and message lifecycle events through terminal `agent_end`.
3. Prove a positive `ctx.isIdle()` sample and the channel's active-to-idle transition.
4. Correlate tool approval requests and resolutions by session, tool name and call ID;
   the local fixture denies only its empty fixture tool.
5. Deliver a fresh nonce, require one transport acknowledgement, and prove completed
   consumption in the fixture model and native messages.

The probe uses empty HOME/XDG/credential/workspace/PTY roots and a local model fixture.
Its model requests go only to loopback with dummy fixture keys. Successful and refused
measurements are retained under the harness state root's `harness-admission` directory,
with probe and adapter identity. Replacing the executable, interpreter, package manifest,
available lockfile, extension or probe requests a new measurement. Historical captures
certify only their exact measured builds, not every patch in a release family.

The probe does not certify pricing, context arithmetic, every provider/model combination,
or every interactive mode. Its retained test captures are in
[`crates/st-drivers/tests/fixtures/harness-admission`](../../crates/st-drivers/tests/fixtures/harness-admission/).
A refusal stops a new OMP provider before it claims the live session. Admission runs on
restart and residency resume; it does not stop an already-running provider retroactively,
and adopting an existing provider does not repeat the probe. See
[harness admission](seat-deploys.md#harness-admission-at-the-next-launch) for the current
incarnation's diagnostic and doctor result.

### A person's admission exception

A person whose machine cannot run the isolated probe can allow its exact installed build
from their own terminal, outside an agent seat:

```sh
st admission override omp --binary /path/to/omp --reason 'The isolated probe cannot run in this offline installation'
```

Use the executable the affected seat actually launches. Omitting `--binary` uses that
terminal's PATH; `--state-dir` selects a nondefault daemon state directory. Run as the
operating-system user that owns the seats, then restart only the affected seat.
The command requires no daemon, producer/probe child or model credentials. It retains
reason, time and exact producer/extension identity in a separate `*.override.json` record;
failed measurements remain intact, and the driver identifies the active exception.
An executable, interpreter, manifest, lockfile or extension replacement needs a new
exception. The exception persists across st updates with the same producer and extension.

To restore normal admission at the next launch:

```sh
st admission revoke omp --binary /path/to/omp
```

The exception cannot supply a missing interpreter or repair an incompatible producer or
extension. It is a person's explicit allowance, not passing measured evidence.

## Seat label authority

The declared `name` is the human label; the desired-state declaration and normalized
`display_name` retain the same value and the original declaring actor. Change that label with:

```sh
st agents rename agent/worker 'Build worker'
st agents rename agent/worker --clear
```

In free mode any person or agent can rename a seat; a bound harness acts as itself.
Rename refuses a launch this build cannot read, because republishing would erase it.
Clearing restores the subject without its `agent/` prefix. A label-only revision keeps
its predecessor's launch lineage, restart budget and crash-loop hold; it does not restart
a finished, exhausted or parked seat.

## Native subagent reporting

OMP contexts identified as `sub` share the process-wide stash and never open a seat
channel, receive its mail or close its binding. Their lifecycle frames use the top-level
seat's existing channel. A run reports start, bounded progress and busy heartbeats,
then an end outcome. Progress is at most once per ten seconds; a busy heartbeat is
once per minute. A proven idle or unreadable context clears cached waits and stops
reporting, and a late turn-end after the end emits nothing. Human asks and approvals
keep a foreground run busy. IDs are bounded to 256 UTF-8 bytes and names to 128 bytes.
st ends a subagent that has not reported for five minutes; see
[subagents](subagents.md) for the roster and lifecycle contract.

## Authored resume, transcripts and delivery

An authored `args "--resume" "/absolute/path/<time>_<uuid>.jsonl"` (or `--resume=…`) keeps its
original argv and transcript. On a fresh native omp/pi driver start, st links an absent or empty
managed `provider-sessions` directory to the transcript's parent after checking the filename UUID
against the session header in the first two lines. It never copies the transcript: omp continues
appending to the authored path, and the channel must see those same bytes to publish its native
session binding. Already-correct links and inventories containing the same inode are left alone.
Relative or missing paths, invalid filenames or headers, non-empty inventories and foreign
(including dangling) directory links are left untouched; the driver log records a typed
`authored_resume_link_skipped` reason. A legacy directory may contain several sessions; the channel
still looks up the reported UUID. This does not backfill an already-running driver's binding.
Conversation readers use the current incarnation's exact native binding (session ID and path),
not the newest sibling in that directory. An unreadable bound transcript, or a linked inventory
without a current binding, stays unavailable rather than displaying another seat's conversation.

The native conversation header counts assistant usage and cost once per message, including
messages containing only tool calls. Message-level metadata belongs to its last projected block,
so newest-first header windows retain it even when a message spans the window boundary.
Thinking and text parts do not each contribute another response's cost.

Ordinary mail arriving during a running turn stays outside OMP until the current tool batch
returns or the run ends. An ordinary tool call that runs longer than the ten-second hold limit
can still be backgrounded. A native ask or approval overrides that limit: no mail is handed to
OMP while either modal is open, even if the context reports idle. The sender's delivery status
shows the mail as held for the native answer, not delivered.

The matching ask result (including cancellation or timeout), approval resolution, or terminal
run cancellation releases the hold. Mail reaches OMP at the next safe tool-batch boundary, in
arrival order as one steering message. Session restart clears the old modal; the successor
channel re-sends unacknowledged mail. A channel reconnect alone does not clear a live modal.
Read the exact graph message with `st conversations read` before acting on it, and archive it
after the related action completes.

The channel reports idle only after `ctx.isIdle()` proves that the native turn has settled.
It keeps sampling through slow final unwind rather than abandoning the idle edge after a
timeout. New activity, session replacement, or channel replacement retires the old sampler;
an outstanding human ask or approval keeps its blocking observation, including on reconnect.

## Channel telemetry

The managed channel forwards `timeline` and `context` frames to the shared pi-family
normalizers. Response usage comes from `timeline` / `message_end`, including the provider's
token buckets, model, and reported cost; context occupancy is a separate measurement.
A `turn` frame carries a terminal result, not usage: provider errors become timeline errors.
`session` and `ready` frames report the native session ID and transcript for resume.

Native transcript tool results use the message-level `toolCallId` and `isError` to produce
one typed result paired with its exact call, preserving composite IDs and failure status.
Text blocks within that result remain its content, not separate plain tool messages.

Visible native OMP `custom_message` records are content entries with
`body.media_type = "application/vnd.omp.event+json"` and `body.text` containing this JSON envelope:

```json
{
  "_tag": "OmpEvent",
  "version": 1,
  "kind": "irc:incoming",
  "attribution": "agent",
  "content": "An incoming message",
  "details": {"from": "worker", "message": "An incoming message", "id": "message-id"}
}
```

`kind`, `attribution`, `content`, and `details` retain the native values; missing values are
`null`. The timeline role remains `system` as a transport role. Consumers derive provenance
from the envelope, not display-text heuristics: IRC senders come from `details.from`; async
task identities stay in `details.jobs` with their native `type`, `label`, and `id`. Mixed or
multiple jobs remain one native event unless their body has an authoritative partition.
Human, daemon, system, and absent attribution are not inferred to be agents.
Structured resource observations retain their content and details in the same envelope.
`display: false` stays hidden; an absent `display` retains the native reader's visible behavior.
The native-value byte bound applies separately to content and details (8 KiB each), with
kind and attribution bounded separately to 256 UTF-8 bytes. Oversized fields become marked
truncated strings without corrupting the envelope's JSON; the marker is added after the
retained byte window. In particular, administrative details is the native record object when
it fits, and a marked JSON-preview STRING when it exceeds 8 KiB. Complete original fields
remain accessible through the negotiated native-source continuation. A wide identity cannot
make the surrounding conversation unreadable.
Timeline page limits are item-count upper bounds: a page can contain fewer entries to stay
within the gateway response budget. Pagination preserves each complete envelope and advances
the continuation cursor by the number actually returned; following it does not skip entries.
Consumers must opt into this media type. Legacy conversation adapters may omit these system
entries rather than render their JSON.
Pi's existing plain custom-message projection is unchanged.

The observed administrative entry types `model_usage`, `title_change`, `session_init`, and
`ttsr_injection` use the same envelope rather than an unrecognized-entry text fallback.
Their `kind` is the native entry type, `content` is `null`, and `details` contains the bounded
native record, including its IDs and timestamp. Native attribution is retained when present;
otherwise it stays `null`. The timeline timestamp uses the same native/predecessor rule as
conversation entries. `display: false` remains hidden.
Main's typed bookkeeping, reasoning, tool views, timing metadata and compaction summaries remain
visible according to the current native-reader omission table; this change does not restore an
older hidden-record list. This event whitelist applies only to OMP: Pi and genuinely unknown
entry types retain their current behavior. Negotiated conversation blocks keep their typed
view and authenticated continuation to the complete raw record beside the bounded envelope.
Both legacy and negotiated preparation preserve the envelope's parseable JSON. A complete lone entry may exceed the 768 KiB page target up to the hard response ceiling
(1,048,576 minus 128,000 reserved bytes); larger entries retain the current native reader's
visible size-limit/error continuation behavior. Cached and keyset pagination explicitly refuse
an entry above that hard ceiling if it still reaches paging; they never silently skip it.

`crates/st3/fixtures/omp-admin-events.jsonl` preserves one native shape per administrative kind
from a session resumed on 2026-10-04. Native IDs and timestamps remain exact, but every payload
string and numeric usage value is redacted. No bootstrap prompt, injected rule, task, model
label, or tool name from that capture is published.

`crates/st3/fixtures/omp-native-events.jsonl` contains a redacted 2026-10-01 OMP capture:
the native tool call/result IDs and error flag, plus an incoming IRC event's envelope.
Tool arguments, result text, sender name and incoming text are redacted; no credentials,
machine paths, or host metadata are retained. Task-job and structured-resource boundary
cases are synthetic regression inputs, not additional claims about that capture.

Unsupported frames, including `pre_compact`, produce a structured debug event with
`frame_type` and `observer_enabled`; frame payloads are not logged. When the managed observation
outbox is unavailable, unforwarded `timeline`, `context`, and `turn` frames produce the same
diagnostic. The managed route does not perform the standalone channel's pre-compaction context
recovery stub.

## Interrupted ask bridge

Native session continuation belongs to the shared driver mechanism: the daemon names a relaunch's
session in `ST3_NATIVE_CONTINUE_SESSION`, and `native_resume::continued()` selects that exact
session when available. Suspension resume uses `native_resume::requested()` and refuses if it cannot
bind the named session. The bridge below inspects only the OMP session successfully selected by
one of those paths. Without a selected session, including a `native-continue-unavailable` fallback,
there is no bridge; it never chooses a transcript or session itself.

OMP 18.4.10 needs a temporary, token-free recovery bridge for an open `ask` picker:
`arn:lmig:smalltalk:2026-10-02-omp-ask-resume-bridge`
([migration registry](https://app.notion.com/p/OMP-interrupted-ask-resume-bridge-st3-3ede3d41f4a3818a9e37ec160c006bbf)).
When the selected transcript has an unresolved last ask, st admits the exact installed OMP build
and the same shipped extension selected by interactive launch before running a bounded RPC heal
pass with closed stdin. Admission may run isolated extension probes; an unchanged exact identity
reuses its measured result. Interactive launch repeats admission before its own fork, then receives
`ST3_OMP_PENDING_ASK` naming the pending toolCall id. `--no-extensions` disables discovery, but a
fleet launcher can still prepend explicit `-e` extensions. The heal clears `PTY_SESSION`,
`PTY_ROOT`, `PTY_SESSION_DIR`, and inherited
`ST3_*`, `ST2_*`, and `ST_*` environment variables (except the launcher's required `ST_AGENT`
identity) so those extensions cannot reach the seat's PTY or st channel. The bridge's transcript
read logs I/O failures, fails open, decodes bytes lossily, and skips malformed JSONL lines while
continuing to inspect later valid entries. Read, heal, retry, or diagnostic failures do not stop
native continuation; an unadmitted OMP build still prevents launch through the driver's gate.
The extension holds mailbox delivery, waits for its ready frame and first idle, and requests one
raw F5 from the Rust channel. For OMP's native interrupted result, the same toolCall id, questions,
and options reopen without a model turn or human action. Channel readiness resumes when that ask
starts again, or after a 120-second timeout with a diagnostic. Both exits send `delivery_ready`
to release the Rust channel's first-idle delivery gate independently of activity. A reopened
picker remains blocked on the person and ordinary mail stays outside OMP until it resolves.
Its existing state-frame `reason` names the reopened toolCall id
(`reopened interrupted ask <toolCallId>`), alongside `ask: "question"`, so clients can explain
what the seat is waiting on. Shared native continuation does not depend on the bridge.

The contraction criterion is behavioral, not a version equality assertion. The standalone
`crates/st3/fixtures/omp-resume/native-reopen-probe.py` copies its adjacent public canary fixture into
a disposable native session directory, renders native resume in a PTY under an empty HOME with no
auth or extensions, and never sends F5 or a prompt. It requires `OMP_BIN` pointing at the raw pinned
OMP executable (not a fleet launcher); `--omp` overrides that variable. Optional `--fixture`,
`--timeout` (seconds, default 30), and `--screen-out` select the capture and output. Exit codes:
**0** means the F5-to-retry UI rendered without native reopening (bridge still needed);
**1** means the original picker reopened natively (contract this migration);
**2** means probe error, not evidence either way.

Smalltalk does not provide OMP in its checks: the native Rust test runs this probe only with
`OMP_BIN` set and otherwise prints an explicit skip. The hermetic gate belongs to the dotfiles
OMP pin, `flakes/external/omp`: run the same script and fixture verbatim as a flake check on
**every** OMP bump, including patches. That is where native reopening must fail the bridge-retaining
check with `arn:lmig:smalltalk:2026-10-02-omp-ask-resume-bridge`; remove the marked bridge when it
fires. The fixture keeps the canary prompt, actual tool arguments and interruption records, but
no machine paths, hosts, credentials or usage/account metadata.

The detector treats an error tool result for the pending toolCall id as interrupted when its text
starts with `Previous OMP process exited before this tool returned`, or when it follows a
`custom/session_exit` whose `pendingToolCalls` lists that id, with no intervening user or
non-aborted assistant message. Other results, including a person's cancellation without that exit
context, answer the ask. The bridge does not change shutdown timing.
