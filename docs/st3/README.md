# Fractal documentation

The root [README](../../README.md) introduces the product and links its guides.
[Getting started](../getting-started.md) walks through installation and a first mission.
[Development](../development.md) covers source builds, CI, shared UI models, and repository layout.
[Home Manager](../site/reference/home-manager.md) covers declarative installation and user services.

Use these documents for implementation details:

- [Architecture](design.md) defines the stable system shape.
- [Mission graph runtime](mission-graph-runtime.md) defines the KDL language and execution model.
- [Configuration and KDL](../site/reference/configuration.md) defines day-to-day publication and revision workflows.
- [Owned sets](owned-sets.md) defines complete membership publication, source ordering and retirement.
- [Seat rollout](owned-seat-cutover.md) defines idle cutover, strict native-session continuity and source status.
- [Free mode](../site/reference/configuration.md#free-mode): within a fleet, an agent may do anything the person who
  runs the fleet may do, as itself, until principals and grants land.
- [Person delegation](person-delegation.md): record a person's prior answer or instruction as
  the agent, with the person's allowed list, exact episode, and quoted evidence.
- [External assistants](external-assistants.md): separate assistant principals, owner-issued expiring grants, and proof-bearing read boundaries.
- [Schema registry](schema.md) lists the generated public subject, resource, and claim vocabulary.
- [Typed custom subjects](custom-subjects.md) defines immutable registrations, generic reads and replies, and derived-state freshness,
  and the decision-tree manifest.
- [Data authority](data-authority.md) separates durable facts from projections and caches.
- [Large SQLite values](large-values.md) records variable-value storage debt and the source-site guard.
- [0.x compatibility](compatibility.md) states upgrade baselines, independent contracts and current platform proof.
- [Binary releases](binary-releases.md) describes release artifacts, installation and upgrade checks.
- [Install/upgrade troubleshooting and support](../when-something-is-wrong.md) provides the diagnostic entry point and sanitized issue path.
- [Claim backups](backups.md) explains live snapshots, offline restore, and recovered writer identities.
- [Stable node identity](node-identity.md) explains computer renames, same-state recovery and local ownership diagnostics.
- [Fleet replication](replication.md) defines convergence, inspection, and repair.
- [Seat state truth](seat-state-truth.md) says which state a seat shows, when it is `down`, and where the rule lives.
- [Agent seat queues](seat-queue.md) explains how each seat orders its mission runs and how moves
  are recorded and replicated.
- [Checkpoints and trimming](checkpoints.md) explains how every node of a fleet agrees to delete old
  claims together: when a checkpoint is due, seal, verify and trim, which claims a rule may drop and
  why, excusing an unreachable member, and what the first real trim taught.
- [Checkpoint capture in bounded memory](checkpoint-capture.md) is the design for capturing,
  planning and proving a checkpoint within a fixed memory budget, with a capture that resumes
  after a restart (#2492).
- [Retention](retention.md) names how long Fractal keeps each claim kind: the policy file, its four
  classes, and how a rule is added and measured.
- [Shell completion](cli-completion/spec.md) defines live entity candidates, their descriptions,
  and short-name resolution.
- [Lanes](lanes.md) defines the ordered lanes a mission run works through, such as the merge
  train, and `st lanes`.
- [Attachments](attachments.md) defines the images a message carries between machines: where the
  bytes live (never in sync), the limits, upload and read, and delivery to a seat.
- [Long messages](long-messages.md) explains document references for messages beyond the inline limit.
- [Conversation search](conversation-search.md) describes searching normalized conversations and opening a match.
- [Resource subscriptions](resource-subscriptions.md) defines observers and automatic intake.
- [Terminal agent retirement](agent-retirement.md) explains the owner-local writer fence, cross-peer policy and coordinated upgrade gate.
- [Suspending a seat](suspend.md) explains `st agents suspend` and `resume`: when a seat is quiet
  enough to stop, and how each harness comes back on its own native session.
- [Seat handoff](seat-continuity.md) explains the one document a seat leaves for the session after it, and the first
  message a restarted seat is sent with that handoff and the live state of its work.
- [Seats across deploys](seat-deploys.md) explains how a running seat's driver and channels follow
  a replaced Fractal binary without ending the provider session, and how Fractal reports a stale message path.
- [Silent messages](silent-messages.md) explains the silent and wake message kinds, capped held batches, and where status and run events go.
- [Delivery probes](delivery-probes.md) describes token-free native-channel probes, per-direction
  read latency, overdue attention, and the replicated results in `st doctor`.
- [Live-path priority](priority.md) explains how the daemon and each PTY server outrank the builds
  and tests their harnesses run, on Linux and macOS, and what needs root. It also explains how
  stopping a seat ends every process the seat started, and what a host without systemd misses.
- [Seat lanes](seat-lanes.md) explains how a seat that declares `capacity` runs with its builds in a
  CPU and memory cap of its own, behind a per-host switch, and what a host guard must not count.
- [Subagents](subagents.md) explains how Fractal records the subagents a seat's harness runs as claims
  on the seat, with a lease, and ends them when their harness, session or seat goes away.
- [Harness questions](harness-questions.md) explains how Fractal records the questions a seat's harness
  asks natively, as ids and option positions without text, retaining compact ended records forever.
- [Harness login](harness-login.md) explains credential failures and recovery on the owning host.
- [Managed harness access](harness-access.md) explains install permissions and per-seat overrides.
- [Model accounts](accounts.md) explains how a person declares several Claude and Codex accounts, how a
  seat binds one or a pool, and how a pooled seat at its limit restarts on another account.
- [Demos](demos.md) explains `st demo play`: a seeded, scripted demo in the terminal interface,
  in its own instance, with timed and key-paced beats and no model tokens.
- [Instances](instances.md) explains `--instance NAME`: a second, fully separate Fractal on one machine
  with its own state, sockets, service names and harness accounts, and the uninstall that removes it.
- [Sekrets](sekrets.md) explains the opt-in gateway that runs any CLI with a credential no seat
  can read: profiles, policies, grants, how it tells a seat from its person, and its sandbox.
- [Tiles](tiles.md) explains how root runs an agent as its own Unix user, in its own namespaces
  and root-owned cgroup: the registry, launch, cleanup, the endpoint, and what the tests prove.
- [Command recorder](command-recorder.md) explains how every `git` and `gh` call Fractal starts is
  recorded, the log format, and what the recorder cannot see.
- [Gate evidence](gate-evidence.md) explains signed test and review records, local checks and CI verification.
- [Profiling the daemon](profiling.md) explains how the daemon accounts for its own time: waits
  for the writer and read connections, SQLite statements, CPU, and callers.
- [What a run waits on](waiting-on.md) explains `st missions waiting-on RUN`: each open step's
  wait and a run's stalled verdict, read in milliseconds from published state.
- [Operational queries](operational-queries.md) explains bounded mission lists, outcome history,
  mission run summaries, and the five-minute performance report in `st doctor`.
- [Agent migration](agent-migration.md) defines isolated rehearsal, cutover, and rollback.
- [Eval audit](eval-audit.md) records the test intent and prompt boundary for each Fractal eval.
- [Running Fractal with omp](omp.md) covers omp seat setup, behavior, and known limits.
- [Product roadmap](roadmap.md) records accepted future work.
- [CLI reference](../site/reference/cli.md) introduces the command surface and exact installed help.
- [Client v0 contract](client-v0/README.md) defines the shared TUI and mobile JSON, event, action,
  pairing, and terminal protocols.
- [Device signing](device-signing.md) defines paired-client keys and signed requests.
- [Operational-state contract](operational-state/README.md) separates immutable history, current
  projection, and actor-specific actionable views and defines screen/CLI parity.

The [examples](../../examples/fractal/README.md) show small, tested mission patterns. Use the evals for
failure proof, not as introductory examples.

## Observe, don't instruct

Fractal observes agents and does not instruct them. It learns what an agent does from driver hooks and
harness events: sessions, turns, plan mode, subagents, tool calls, usage, and what a seat is blocked
on. It never asks an agent to report on itself and never tells an agent how to behave. A seat starts
idle with no prompt and takes no turn until a person types or a message is posted. Work reaches it
as a message that names a ready step, and the step's goals and constraints say what the work is.
A seat that sits idle holding a claimed step, with nothing set to wake it, gets one message naming
the step and what it waits on ([idle holders](mission-graph-runtime.md#idle-holders)).
Fractal's only other text for agents is the skill that `st skill` prints, which describes how to use Fractal
and sets no rules of conduct. Keep this rule when changing drivers, messages, or the skill.
