# When something is wrong

If a populated v0.3.4 store may have unsigned delegation grants, preserve raw state and keys
and follow the [bounded signing audit](st3/backups.md#preserve-suspected-unsigned-history-before-maintenance) **before** doctor, restart or
export. Doctor can seal pending work; these diagnostics are not the initial read-only capture
for that affected history.

For other stores, start with the smallest question: is the daemon reachable, is the agent ready,
or is the work waiting on a dependency? These checks use the worker and README mission from
[missions in practice](https://github.com/compoundingtech/fractal/blob/main/docs/site/guides/missions-in-practice.md):

```sh
fractal doctor
fractal service status
fractal missions show mission-run/garden/readme/one
fractal agents show agent/garden/worker
fractal agents queue agent/garden/worker
```

| What you see | Next check |
| --- | --- |
| Daemon unavailable | `service status` and [startup replay](#startup-replay-and-an-unavailable-api); an active process may still be recovering. |
| Missing `fractal` or `pty` | [PATH and PTY](#path-pty-and-harness-login); check the daemon user's login-shell PATH too. |
| macOS launch or permission denied | [Signing and permissions](#macos-signing-and-permissions). |
| Agent starting or waiting | Attach to its terminal and finish login, trust, or permission prompts. |
| Agent stopped, failed, or missing from the list | Show its exact ID and history; read its stop reason before starting it. |
| Mission queued or blocked | Its predecessor run, step dependencies, human gate, and agent queue. |
| Message waiting or delivery stale | The recipient's local doctor and delivery assessment; inspect the message receipt. |
| Remote member last seen | Check the encrypted route; a sleeping member is normal and catches up on return. See [replication checks](#replication-and-upgrade-checks). |

To inspect a stopped agent and why it stopped:

```sh
fractal agents ls --all
fractal agents show agent/garden/worker --json
fractal subject history agent/garden/worker --limit 20
fractal terminals peek agent/garden/worker
```

The last terminal screen can explain a login, crash, or provider limit; the graph says whether the stop was requested, the owner ended, a suspension is active, or the runtime failed. A stopped agent can retain a historical terminal, and an agent with no recorded terminal may have nothing to peek.

## Planned graph replay

Run `fractal replication replay` locally for the claim count, last measured replay
duration (if known), and writer-lock warning. It is an estimate only: no replay runs
without `fractal replication replay --confirm`. Schedule a planned window because the
daemon holds its writer for the entire replay, delaying other writes. The daemon logs
start and completion and keeps the operator, start time, duration and result in
`replication-replay-audit.jsonl` in its state directory. A concurrent replay is refused,
not queued. Do not restart the daemon to “fix” a replay; let the running replay finish.
The estimate names any blocked owned sets. Canonical replay removes their retained
physical `desired` and `mission_definitions` rows and cannot rebuild them while the
sets remain blocked; authoritative claims and set receipts are not deleted. Resolve
the blockers first if those retained rows must remain available. `--confirm` prints
the same warning before starting; it does not add another confirmation flag.

## Writer queue waits

When an interactive request is slow during replication, distinguish time queued
for the store writer from time holding it. Request-parented `st.db.writer` spans
report `writer_wait_ms`, `writer_hold_ms`, the `queue` class (`foreground` or
`background`), and the static `code.file.path`/`code.line.number` caller site.
These children require a sampled trace context, not `ST3_STORAGE_PHASE_SPANS`.
Caller sites are trace attributes, not metric dimensions.

Foreground batched writes and foreground loans retain arrival order. Background
loans yield to pending foreground work, with one background turn after at most
eight foreground turns to prevent starvation. This bounds dispatch turns, not
wall time: an active envelope, transaction commit, and its finalization are not
preemptible. A long background hold can therefore still delay an interactive
request even when scheduling priority is correct.

Replication verdict pages use background loans and revalidate trust roots after
each acquisition, including the transaction that activates the authority fold.

Compare the caller site and hold stages before attributing a slow request to
queue priority or changing admission chunk sizes.

## Raw terminal setup stalls

Raw terminal setup logs bounded DEBUG phase events, not terminal contents or
capabilities. The API handler records its start timestamp (`started_at_unix_ms`)
and monotonic `elapsed_ms` for `auth`, `lookup`, `capability`, `owner-open`, and
`upgrade`; completed lookup, capability, and open phases also record
`duration_ms`. Here `auth` is the handler's scope/person check, after transport
authentication. For remote ATTACH, the owner peer separately records
`peer-auth`, `owner-attachment`, `owner-open`, and `peer-upgrade`. PEEK uses its
existing owner lease-admission path instead. A phase start without completion
locates the unfinished or rejected operation; it does not prove a timeout cause.
After upgrade, `first-frame` records the first successfully sent PTY byte chunk,
with elapsed time measured from the splice's start and the terminal/incarnation
in its `raw_terminal_stream` span. There are no per-frame debug events.

The TUI records an attempt UUID and target agent on every phase event. Its
attempt-scoped lifecycle records a terminal outcome with the phase reached and
elapsed time even on outer deadline expiry, selection changes, or drop; failures
are INFO-level outcomes rather than completion-only DEBUG messages. Capability
issuance and combined client connect/upgrade retain their 8-second budgets.
Only a capability-issuance timeout retries automatically, at most twice after
1 and 2 seconds, keeping the selected incarnation. Each cancelled issuance
future is dropped before retry; an unused capability opens no PTY and expires
after 60 seconds. Upgrade timeouts name that phase and offer Ctrl-R, without an
automatic retry. The overall setup deadline and single-flight guard still apply.

Without any opt-in variable, every non-successful terminal attempt also emits
one `tracing::info!` event and is retained in
`$XDG_CACHE_HOME/st3/stui/attach.log` (otherwise
`~/.cache/st3/stui/attach.log`), beside `slow.log` and `exits.log`. This uses the
same cache-log permissions and 100 KB reset bound as those existing logs.
It records only attempt ID, target agent, phase reached, outcome, and timing;
it requires no tracing subscriber or OTEL exporter. Start and success events
remain exclusive to the opt-in sink described below.

For a private, device-local reproduction log, launch the TUI with an absolute
JSONL destination:

```sh
FRACTAL_TUI_ATTACH_TRACE_FILE="$HOME/raw-attach.jsonl" fractal ui --client
```

This file is the actual client diagnostic sink (`service="fractal-tui"`), not an
OTEL export or daemon journal. It contains identifiers and timing, not terminal
contents, credentials, or capabilities; treat it as private. Open the target pane
without typing provider input. `first-frame-decoded` means the client decoded a
PTY protocol frame; `first-screen-rendered` follows a native screen draw and
successful synchronized terminal-frame completion. Neither is inferred from
server first-byte timing.

Use a separate file per TUI process. New files have mode 0600; an existing file
must be a regular, single-link, non-symlink file owned by the current user with
mode 0600, or logging stays disabled. Appends stop at 4 MiB without truncation.
The sink is best-effort on I/O errors, size exhaustion, or lock contention; it
never waits for another process's log lock. Records include `at_unix_ms` for
server correlation and monotonic elapsed time for local phase duration.

Server phase DEBUG events use the daemon's stderr sink. Under a user service,
read its journal (for the standard deployment,
`journalctl --user -u smalltalk.service`). Enable the relevant modules in the
managed daemon's `RUST_LOG`, for example
`info,fractal_api_client_v0::api::client_v0::raw_terminal=debug,fractal_api_core::api::client_v0::raw_terminal=debug,fractal_peer::peer=debug`.
Changing a running service's environment requires its normal authorized
deployment; the TUI does not change daemon logging or restart it. Existing OTEL
request spans use `service.name="st-daemon"`; DEBUG stderr events are not exported
by its INFO-and-above OTEL log filter. Correlate server terminal/incarnation and
request times separately from the client attempt UUID.

Endpoint resolution, Unix connect, and WebSocket upgrade are private generated
client boundaries: they are **not separable without codegen** from this caller.
For remote connections the generated connector also combines DNS/TCP/TLS and
upgrade. Separate client-side connect/upgrade and first-binary-arrival timing
must be added at the next client regeneration; server `first-frame` is a send
measurement, not proof that the TUI received or rendered a screen.

The HTTP upgrade includes server authority checks, runtime lookup, and opening
the owner's transport. A client-side upgrade timeout cannot identify which of
these operations stalled. Runtime lookup and local PTY opening retain their
incarnation fences: the local socket must prove the selected daemon PID and
registry creation time before any PTY frame is sent.

Dropping an in-progress HTTP upgrade closes the client's connection; it is not a
remote cancellation acknowledgment. The remote request can still be opening its
owner transport. Failed upgrades drop their captured transports, and established
bridges propagate connector closure, but there is no acknowledgment that all
remote cleanup completed before another attempt. Do not automatically retry an
upgrade timeout on the assumption that dropping its future completed remote
cleanup. Keep provider input out of setup diagnostics.

## Linux PID namespaces and procfs

The runtime requires `/proc` to describe its own PID namespace. Entering a child
PID namespace while inheriting the host's `/proc` mount is unsupported: a local
PID can name a different host process, or no host process at all. Runtime launch
and process observation refuse this configuration with the typed
`ProcessNamespaceMismatch` error rather than guessing process identity or treating
live work as exited. The launch refusal happens before creating a child or
mutating launch state; PTY stop and cleanup also refuse mismatched procfs.

Run the runtime with procfs mounted for the namespace it runs in. Container and
test launchers that create a PID namespace must mount its `/proc` there as well
(for example, `unshare --pid --fork --mount-proc`), rather than inherit an outer
procfs mount. Correctly mounted namespace-local procfs remains supported. This
does not change persisted PIDs or signal targets: both remain namespace-local.
Do not work around the refusal by deleting session records or skipping process
lifetime tests. No running service or procfs mount is changed automatically.

## Free heap and resident memory

On Linux with glibc, the daemon checks free arena bytes every 30 seconds on a
blocking thread. When `mallinfo2().fordblks` exceeds 256 MiB, it calls
`malloc_trim(0)` to return whole free pages in all arenas to the OS. This is a
reclamation trigger, not a hard RSS limit: live or fragmented pages cannot be
returned. Free arena bytes can remain high after pages are discarded.

Set `ST3_MALLOC_TRIM_THRESHOLD_BYTES` and `ST3_MALLOC_TRIM_INTERVAL_SECS` in
the daemon environment to change the threshold and interval. Set the threshold
to `0` to disable trimming. The interval must be positive; invalid configuration
fails startup. Restart the daemon after changing these values.

`st.allocator.trim.duration` records seconds per call. `st.allocator.trim.released`
records nonnegative byte decreases with `source=fordblks` or `source=rss`.
These are process-wide observations, not exact attribution to trimming:
concurrent allocations can affect both samples. The debug log includes raw
before/after values and the glibc return flag. Trimming can pause allocator users
during each call and cause page faults when discarded pages are reused.
Other platforms and non-glibc Linux builds do not start this worker.

## Check usage without starting another turn

```sh
fractal usage --hours 24 --by agent
fractal usage --hours 24 --by mission
```

These are observed token totals. Cost is an API-equivalent estimate, and unpriced models are shown as unpriced. Account limits and stop policies can explain why an agent stopped; the agent's reason is more useful than repeatedly restarting it. See [model accounts](st3/accounts.md).

## A conversation does not follow new entries

The footer's updated time means a frame arrived, not that its newest entry was
displayed. To locate a stale conversation inside the TUI, start a new process:

```sh
umask 077
FRACTAL_TUI_FRESHNESS=1 FRACTAL_TUI_FRESHNESS_LOG="$HOME/tui-freshness.log" fractal tui
```

This opt-in DEBUG trace writes only the `fractal_tui::freshness` target to the
append-only log, not the terminal. No `RUST_LOG` setting is required. Unset the
variables to disable it. Protect the file: identifiers, endpoints and timestamps
are metadata, not anonymous data. No message bodies, headers, tool arguments,
rendered text, or authentication credentials are recorded.

Compare `frame` (subscription, target, session, replacement flag, count and maximum
timestamp/sequence/id), `held` (merged session, newest entry and count), `adapted`
(last displayed entry and count), and `document` (last rendered entry and line
count). Filtering and folded tool runs can intentionally change the last ID.
`startup` and `connected` identify the binary build and endpoint; `update_before`,
`presented`, `end_before` and `end_after` record pane keys, focused/main pane,
follow state, top, total and height. `document` runs during drawing, so use
`presented` for the complete frame's pane binding. End's `end_after` is before
redrawing; the next `document`/`presented` pair shows the resulting viewport.
The first stage that stops advancing identifies where to investigate next.

## Recover the cause

Finish a first-run prompt by attaching, then detach with **Ctrl+\\**:

```sh
fractal terminals attach agent/garden/worker
```

After fixing its dependency, restart a stale running agent; explicitly start an intentionally stopped one. Resume a suspended one instead. [Agent lifecycle](seat-lifecycle.md) gives the commands and their different effects. Retry failed work through the graph after correcting its cause; preserve the failure evidence rather than completing a step just to make the display green.

For a Claude agent with stale delivery, check `fractal claude-channel status`, then attach and use Claude's `/mcp` menu. If `plugin:st-channel:fractal` says failed, select it and **Reconnect**. A ready installation does not by itself prove that an agent's channel connected.

If doctor reports an operational contradiction, inspect the bounded repair plan:

```sh
fractal repair dry-run
```

Apply only the exact reviewed token with the procedure in [operational repair](st3/operational-state/README.md#doctor-and-operational-repair). A changed plan needs a new review; repair appends transitions instead of deleting history.

## What the watchdog does

The runtime watches process liveness, readiness, claim leases, and message delivery; `doctor` reports those facts. A fleet may also run an **operations watchdog mission** that inspects health, groups faults, and routes repairs to an operations agent. That mission is an operating policy, not something every new installation receives automatically. Look for it under Missions, and read its run when it reports a fault; Now should show only what needs your action.

A watcher cannot fix a missing harness login, choose a product preference, or make an expired provider allowance return. Follow the recorded cause, and ask a person through a structured request when only that person can settle it.

See [CLI tour](https://github.com/compoundingtech/fractal/blob/main/docs/site/reference/cli.md), [operational state](st3/operational-state/README.md), and [replication](st3/replication.md) for deeper diagnosis.

## Startup replay and an unavailable API

During daemon startup, `fractal doctor` reads a local readiness file without waiting for SQLite or
an API listener. It reports `starting`, the current recovery phase, and committed frontier/target
when projection has begun. Full replay also reports its stage and, during base claims, processed
claims/total; those counts describe uncommitted work, not a newer committed frontier. Updates
occur at stage boundaries and every 1,000 base claims. The record changes to `serving` only once
both local API and paired gateway sockets have bound. Serving means the APIs accept requests;
projection health remains a separate doctor check.

`fractal service status` adds this readiness to the daemon service line, so a service manager's
`active` or `running` state is not mistaken for API readiness. `fractal doctor --json` includes the
local record as `startup`; if the API is unreachable during recovery it reports a failed
startup check and exits nonzero. Once the API answers, `starting` is informational and the
API's health checks determine the result, even before the gateway has bound.
Command clients that normally announce outage retries return a full-replay phase promptly after
an unsuccessful connection attempt. In ordinary startup phases, they print the phase and keep
waiting under `--daemon-wait`, so a brief restart can finish normally. Long-lived drivers retain
their connect-outage retry behavior. Existing clients still get the
kernel's immediate missing-socket/connection-refused error during recovery and keep their own
retry policy; no early listener accepts requests it cannot yet answer.

The private, rebuildable record is `API_SOCKET.readiness.json`, beside the effective API socket,
with a held `API_SOCKET.readiness.lock` establishing its lifetime. Readers ignore unlocked files,
including a leftover `serving` record after SIGKILL; a restart overwrites the record with
`starting`. These files are local observations, not replicated claims or a client API endpoint.
No client HTTP response schema or API error code changes.

Local startup observation is enabled on Linux. On macOS and other platforms, or when the
observation files or OFD locks are unavailable, startup logs one warning and continues without
a sidecar. Doctor and command clients then use their existing API checks and outage waits.
Only lock-contention errors refuse a second observer; other observation failures never block
startup. Writers and readers resolve socket discovery links and canonicalize the parent
directory, including paths through symlinked directories.


If the frontier or replay counts continue moving, allow recovery to finish. Repeated restarts
can repeat work and make a long replay harder to diagnose. Retain the starting source, target
source, graph size, recovery phase and elapsed time; compare them with the release's impact
notes. On macOS there is no local startup sidecar: use service logs and the existing API checks.
If recovery stops advancing or exits, preserve its bounded error log and report it. Do not
reset state, delete the database or downgrade across a migration to get a reachable API.

## PATH, PTY and harness login

```sh
command -v fractal pty
fractal --version --json
fractal doctor
```

Use the bin directory from your chosen archive or Nix route in the daemon user's login-shell
PATH. Open a new terminal after updating your shell profile. Avoid mixing an old archive's
`st` with a new Nix profile or another PTY install. `doctor` checks the daemon's environment;
working commands in your current shell alone do not prove the service sees them. Refresh
service definitions with `fractal service install` after a changed executable path when they are
manually managed. If Home Manager owns the daemon, update its pinned input and activate that
configuration instead; follow the [Home Manager upgrade](https://github.com/compoundingtech/fractal/blob/main/docs/site/guides/upgrading.md).

Log in to the harness as that same OS user, attach to the affected agent and finish login/trust
prompts. Check its declared workspace and [account](st3/accounts.md) rather than copying another
user's credentials. A missing Claude channel needs `fractal claude-channel status` and the connection
check above. A failed omp/OpenCode admission has exact boundary diagnostics; use
[admission guidance](st3/seat-deploys.md#harness-admission-at-the-next-launch). Attach and inspect
before restarting repeatedly or overriding admission.

## macOS signing and permissions

Verify Python 3 is installed, the configured app path is stable, and the selected signing
identity exists. Release executables are ad-hoc signed, not notarized. Follow macOS's explicit
approval flow for the verified download and run `fractal service permissions` for the service's
Full Disk Access/Developer Tools guidance. Permissions can need renewed approval after ad-hoc
updates. Do not globally disable Gatekeeper or remove quarantine from unrelated files.
A configured missing or wrong-team signing identity fails instead of falling back; see
[macOS installation](st3/macos-installation.md). The optional voice helper is separate from
ordinary agent and mission use.

## Imported agents and saved sessions

```sh
fractal import ls --all
fractal import show SESSION
fractal agents show agent/import/codex/ID
```

Use actual IDs from discovery; these are placeholders. Check the native session's owning home,
workspace and saved declaration. An older imported Codex agent with explicit `resume` arguments
can time out even after installing a new binary. Follow the [exact declaration repair](seat-lifecycle.md#repair-a-codex-import-created-before-strict-resume);
re-importing does not rewrite an existing agent. Keep exported environment values and transcript
contents private. Inspect delivery state and stop reasons for other stale agents before deciding
whether to resume, start or restart them.

## Replication and upgrade checks

```sh
fractal replication status
fractal doctor
```

Compare sources on every member and follow [two-machine checks](https://github.com/compoundingtech/fractal/blob/main/docs/site/guides/two-machines.md)
for frontier/digest inspection and a real message receipt. A sleeping member can catch up later;
a mixed-rules fleet cannot finish new checkpoint sealing. Equal replicated history does not prove
equal projections or valid inner claim signatures. Review those doctor checks separately.
Follow [replication recovery](st3/replication.md#recovery); a reset or copying another member's
keys/database is not a normal troubleshooting step.

## Report an install or upgrade problem

Use [Fractal GitHub issues](https://github.com/compoundingtech/Fractal/issues/new/choose)
with the **Install or upgrade problem** template as the single support path. Initial triage is
owned by the Fractal distribution agent, which routes
runtime, signing and replication failures to the relevant maintainer. No separate support
account or chat channel is required.

Include the exact installed and responding-daemon sources, OS/architecture, install route,
starting/target release, sanitized doctor and service output, failing command and expected
behavior. If the daemon is unavailable, say that and include its startup phase and elapsed
time instead of waiting for doctor to succeed. Harness problems also need the harness version
and a bounded error excerpt. Preserve timing context; replay observations are not guarantees.

Review output **before posting**. Replace host names, usernames, real paths, agent, person and fleet
IDs, tailnet addresses and repository names with consistent invented values. Remove tokens,
private keys, invite/pairing secrets, bearer credentials, transcript prompts, claim/document
bodies and exported environment values. Keep source hashes, versions, error codes, field names
and relative timing intact. Doctor output, terminal screens and logs are not automatically safe
to publish. Do not upload a database, claim backup, configuration file or full transcript to a
public issue. If an excerpt cannot be made safe, describe the error and say evidence was withheld.
