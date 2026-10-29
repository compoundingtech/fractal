# Fractal fleet compatibility

## Required rolling window

[R49](../vrs/requirements.md#must-support-rolling-fleet-upgrades) requires this window: adjacent published releases
**N and N-1 must coexist for at least 14 days** without lost work, duplicate
schedule fires, or blocked schedules. Features that need all members activate
only when every active member advertises support. An older-release member that
returns within **30 days offline** must not block or corrupt the fleet.

**Implementation status: open — [#4002](https://github.com/compoundingtech/fractal/issues/4002).**
The adjacent-release window is a required target and design sketch, **not a current guarantee**.
Hosts roll out independently today through
[per-feature peer capability negotiation](replication.md#peer-protocol-capability-negotiation):
every new peer operation and changed ID or wire format must be gated before dispatch
at each hop. Missing support requires a genuinely compatible fallback or a typed
`peer-too-old` refusal with upgrade guidance, never a speculative request to an older
decoder. An unsupported feature does not make the entire mixed fleet unsupported.
The adjacent-release target covers baseline work and features without a negotiated
gate; it neither substitutes for capability checks nor authorizes new ungated
incompatible changes. Features requiring all-member activation have the additional
admission conditions below, not a coordinated host-rollout requirement.

Read each release's **Upgrade impact** before restart. A patch tag is a publication
label, not a promise of compatible databases or safe rollback. Check exact sources
using `BUILD.json`, `RELEASE.json`, `st --version --json`, and the daemon's
`machine_version` in `st doctor --json`. Downloading or installing an archive does
not restart or replace the running daemon **process**. It does replace installed
command files and, on macOS, the app bundle. Until an explicit restart, the daemon
keeps its current build while new CLI invocations use the installed build. Follow
the [upgrade procedure](../site/guides/upgrading.md) rather than treating installation
alone as a daemon upgrade.

### Additive claims and unknown-field retention

During the window, optional claim fields must be additive: absence preserves old
semantics, and an older reader must retain unknown fields without stripping or
rewriting their signed representation. Replication, replay, restart, forwarding,
and later upgrade must preserve those fields in both directions. Unrecognized
optional data must not prevent known work or schedules from progressing. New
semantics that cannot satisfy this rule require fleet feature activation.

**Open ([#4002](https://github.com/compoundingtech/fractal/issues/4002)):** older
daemons currently retain unknown-field claim failures as retryable records; that
retention alone does not establish compatible projection or schedule progress.
The `owner_run` addition in `restart-retries-a-parked-launch.json` illustrates this
gap: its text requires daemons to deploy together, despite compatibility `none`.
That historical fragment is immutable; its classification needs a fresh correction.

### Fleet feature activation and offline rejoin

Design sketch (**open, [#4002](https://github.com/compoundingtech/fractal/issues/4002)**):

1. Members advertise versioned capabilities in the authenticated replication
   exchange, bound to the authenticated member identity rather than a release tag.
2. Activate an all-member feature only when **all active members advertise it**.
   A missing or unknown advertisement is not support. A partition must not allow
   one side to silently exclude an active older member and activate incompatible
   writes. Persisted activation and write admission must remain safe across restart.
3. An older member returning within 30 days catches up without blocking existing
   work, duplicating a schedule occurrence, or corrupting retained claims. Membership
   and capability admission must fence unsupported writes before accepting them,
   including when a feature activated while the member was absent.
4. A member offline **more than 30 days must re-sync before writing**; it cannot
   resume stale ownership or schedule authority merely because its old process
   restarts. Re-sync must establish current membership, capabilities, and history
   before write admission. This does not authorize discarding unsynchronized work.

The active-member definition, authenticated freshness/absence evidence, durable
activation record, and re-sync completion/write fence are unresolved protocol
details in #4002. A local timeout alone is not authority to remove a member.

### Checkpoints in a mixed fleet

The current [checkpoint contract](checkpoints.md) requires exactly equal rules
digests for sealing and seal verification. Preserve that safety check: release
ordering is not digest compatibility. During N/N-1 coexistence, retain compatible
rules or defer incompatible sealing behind feature activation; continued claim
replication, ownership transfer, and schedules must not depend on a new seal.
Do not trim history an admitted older reader still needs. Mixed digests may defer
new sealing, but must not stall work or schedules.

**Open ([#4002](https://github.com/compoundingtech/fractal/issues/4002)):** prove
progress and retained-history safety across the full window, including a returning
older member. Existing digest checks alone do not prove them.

### Client API capability negotiation

Negotiate against the **serving daemon**, not the fleet's newest release:
`st3.client.v0`, projection versions, capability versions, and release tags are
independent. Existing API reads must work against either N or N-1. Unknown optional
capabilities may be ignored; an unsupported required capability must produce an
explicit refusal before using it. A client opting into a new response shape must
first observe support; member-local API support does not activate an all-member
feature. See [client-v0 discovery](client-v0/README.md#discovery-lists-and-details).
Negotiation exists; **N/N-1 read and refusal coverage is open in
[#4002](https://github.com/compoundingtech/fractal/issues/4002)**.

### Out of scope: downgrade

Coexistence of distinct members is not permission to run an old binary against an
upgraded member's database or driver resume state. Database downgrade, arbitrary
nonadjacent release pairs, and historical 0.x rollback are outside this window.
The offline-return rule is separate from the adjacent-release pair: it requires
safe admission or fencing even if publication advanced while the member was away.

## Upgrade starting points and limits

These are the current documented evidence boundaries, as of 2026-10-07. Use the
[upgrade procedure](../site/guides/upgrading.md) and every intervening release's impact notes when your
starting source is older than the previous published release.

| Starting state | Guidance and evidence |
| --- | --- |
| New empty installation | Use [getting started](../getting-started.md). Initial distribution scope is Linux x86_64 and macOS Apple Silicon, one fleet per person. Archive CI checks packaging and CLI/TUI startup; a clean machine with a real harness login needs a separate onboarding rehearsal. |
| v0.3.16, source `5c0a151e` → v0.3.17, source `f4cec96e` | The [v0.3.17 report](https://github.com/compoundingtech/fractal/releases/tag/v0.3.17) classifies the full source interval. Schema identity remains 16 and checkpoint rules remain 11, but the interval adds local tables and an index, narrows the mission-root tree response, and lists manual steps for integrations. This establishes classified upgrade guidance, not a completed isolated populated-fleet rehearsal or a guaranteed interruption time. |
| Earlier st3 source | Read all intervening notes, including intermediate migrations and manual repairs. Rehearse a populated copy before rollout; no blanket upgrade or downgrade guarantee covers every earlier 0.x source. |
| Populated v0.3.4 with possible unsigned delegation grants | Preserve raw SQLite state and keys **before doctor, restart or export**. A bounded candidate rehearsal proved recovery of preserved unsealed work; already-sealed unsigned history remains affected. It is not exact v0.3.5 archive acceptance. |

Prefer roll-forward when the old binary cannot read the current database, claim vocabulary or
live driver resume format. Database migrations are forward-only unless a downgrade is explicitly
implemented and verified. A saved archive or the macOS installer's file rollback does not undo
a migration. A [claim restore](backups.md) rebuilds into the restoring build's schema under a
fresh writer; it is not a downgrade or permission to overwrite a running member.

## Independent compatibility contracts

| Contract | What to check |
| --- | --- |
| SQLite schema and startup DDL | Read all transitions in the source interval. A numeric schema identity alone does not describe every startup index or projection change. |
| Checkpoint rules | All participating members must agree on rules before new sealing. Mixed builds can replicate signed history while sealing stalls or projections differ. A rules bump alone does not establish a full-replay trigger. See [checkpoints](checkpoints.md). |
| Claim registry and replication | Admitted history, known claim vocabulary, projection digests and signature verification are separate checks. Equal replication digests alone do not establish claim-signature validity. An exchange's optional `build` names the sender's build for divergence faults; older peers omit it and ignore it. See [replication](replication.md). |
| Client API | `st3.client.v0`, projection versions and capability versions are distinct from release tags and database schema. Discover capabilities first; unknown required capability versions stop the client, while unknown optional capabilities may be ignored. See the [client contract](client-v0/README.md#discovery-lists-and-details). |
| Native harness and driver resume | Provider APIs, managed extensions and saved resume state have their own contracts. See [seats across deploys](seat-deploys.md); rolling back across the push-delivery boundary can require coordinated seat restarts. |

Use CLI and the terminal UI from the same release source. Current client version scope is:

| Client | Version/evidence boundary |
| --- | --- |
| CLI and the terminal UI | v0.3.17 archives embed source `f4cec96e`; native release CI checks both tools. This is not a cross-version client guarantee. |
| TypeScript `@fractal/client` | Repository-private package version `0.1.0`; CI contract/schema tests run against its source revision. The package number does not identify the daemon source. |
| Swift `FractalClient` / iOS app | In-tree client source and client-v0 capability contract; no separate released client-version compatibility matrix is claimed. Fresh-device app onboarding is separate evidence. |

The Swift and TypeScript clients share the client-v0 contract; optional features are negotiated
with the responding daemon.
Matching API labels do not promise every feature on an older daemon. For example, native content
blocks require an upgraded serving/owner member and client opt-in; older clients keep the legacy
shape. Consult the release notes before using new client features.

## Harness support and test coverage

Install and log in to the harness as the OS user who runs the daemon. Record its actual version
when diagnosing an issue; Fractal does not publish a universal minimum-version promise for
all future vendor releases. No exhaustive real-provider version matrix is established for
v0.3.17. The rows below describe tested native contracts, not acceptance of every vendor version.

| Harness | Existing coverage and launch boundary |
| --- | --- |
| Claude Code | Managed channel, hook and transcript contracts are covered by boot/delivery regressions. Install the owned channel and verify its connection, then complete real login/trust prompts. |
| Codex | App-server, native thread binding, import and resume contracts are covered by boot/delivery regressions. Saved imports made with authored `resume` arguments need the [declaration repair](../seat-lifecycle.md#repair-a-codex-import-created-before-strict-resume). |
| pi and omp | Managed extension, mailbox and resume contracts have boot/delivery regressions. An unseen omp build is measured in a disposable native admission probe before launch; a failed probe refuses launch and reports the boundary. |
| OpenCode | Loopback API, mailbox and resume contracts have boot/delivery regressions. An unseen build is measured before launch; a failed native contract disables delivery and retains queued mail. |

The [boot canaries](../../scripts/st3-boot-canaries/README.md) use real Fractal processes with
stand-in providers; they do not prove real vendor login, model turns or every installed vendor
version. Native admission probes identify the exact installed build, and their exceptions expire
when the build identity changes. Follow [admission diagnosis](seat-deploys.md#harness-admission-at-the-next-launch)
rather than bypassing a failed boundary without understanding it.

### Rolling previous-release evidence

Cross-build tests pin exact sources in
[the fleet baseline](../../.github/fleet-compat-baseline.json) and
[the messaging baseline](../../.github/messaging-compat-baseline.json). The ordinary window is
the **previous published stable release → current candidate**, not a permanent historical
source. Both pins record the release tag, its full source commit and native release archive
URLs with hex SHA256 checksums. Fleet compatibility runs the extracted published `bin/st3`;
historical messaging uses the channel source from the same release commit.

After each stable publication, release automation opens a baseline maintenance PR against
`main`. It derives the source from `RELEASE.json`, verifies the dereferenced tag and the two
archives, and advances both unheld pins. It does **not** change the already-published release
commit: a release cannot contain its own source SHA. A human reviews and merges that PR before
the next stable candidate can be published. Baseline-only maintenance does not itself create
another daily release.

CI compares the pins with the actual published stable release list, excluding drafts,
prereleases and unpublished tags. A pin more than one published stable release behind fails.
`why` describes the rolling policy; it is not an exemption. `exception_reason` is normally
`null`. A temporary older hold requires a nonempty explicit reason describing the compatibility
boundary and when the hold can be removed. Automation preserves that hold; remove the reason
and repin once it is resolved. Release preparation requires the latest published baseline
unless the pin carries that explicit exception. See [CI operations](../ci.md#rolling-compatibility-baselines)
for recovery and local commands.

These fixtures prove their specified histories and messaging contracts, not the
full R49 window or downgrade support. The friend-ready idle and delivery gates
remain separate evidence; packaging or documentation checks do not replace their
continuous observations.

### Required compatibility test plan

**Open — [#4002](https://github.com/compoundingtech/fractal/issues/4002).** Extend
`linux-fleet-compat` (`scripts/ci-linux`, `crates/fractal/tests/fleet.rs`), which
currently runs one history-preservation test against one previous published binary.
Keep exact release/source/archive pins; exercise N/N-1 in both owner/peer roles,
with controlled time covering at least 14 days of coexistence.

| Scenario | Required assertions |
| --- | --- |
| Schedules across mixed members, restart, and reconnect | Every due occurrence has one durable fire identity and one execution; no duplicate fire or schedule blocked by unsupported claims, capabilities, or checkpoint rules. |
| Ownership transfer N → N-1 and N-1 → N | Existing work survives, stale owners cannot execute after transfer, and schedules continue without losing or repeating occurrences. |
| New optional claim field in both directions | Old/new readers retain and forward original signed bytes; restart, replay, and upgrade preserve the field; known work continues without permanent retry blockers. |
| API reads against N and N-1 | Baseline list/detail reads remain usable; optional shapes require advertised support and unsupported required capabilities refuse explicitly. |
| Capability activation | One active member without support prevents activation; all active members advertising support permits it; partition/restart cannot bypass the gate. |
| Offline older member returning within 30 days | Catch-up preserves unsynchronized work and current ownership, does not stall schedules or fleet progress, and cannot replay a fired occurrence or introduce unsupported writes. |
| Offline member returning after 30 days | Writes and stale scheduling authority remain fenced until re-sync completes; rejoin cannot corrupt history or lose unsynchronized work. |
| Equal and unequal checkpoint rules digests | Compatible sealing succeeds; incompatible sealing/verification stays refused without blocking work or schedules or trimming required history. |

Extend the fast **release-impact** check to reject upgrade text requiring
“deploy together” (or equivalent coordinated deployment) unless the incompatible
change is capability-gated. The check must validate compatibility status against
the text: `none` cannot accompany a known mixed-fleet incompatibility. Add rejecting
fixtures for the ungated `owner_run` case and accepting fixtures for a described,
tested capability gate. **This validator enforcement is not implemented (#4002)**;
the existing fragment-presence/schema check is not this acceptance proof.

## Platform verification

| Route | Requirements and current evidence |
| --- | --- |
| Linux x86_64 archive | glibc 2.35+ (for example Ubuntu 22.04), Bash or Zsh, CA certificates, `curl`, `tar`, a checksum tool and Git for the walkthrough. Persistent service setup needs a systemd user manager; enable lingering for work after logout. Archives include the pinned PTY runtime and need neither Rust nor Nix. Native release CI verifies extraction, installation, exact source and CLI/TUI smoke tests. |
| Apple Silicon archive | macOS 15+, `curl`, `tar`, `shasum`, Git and Python 3. Installer creates `SmallTalk.app`, with ad-hoc signing unless explicitly configured otherwise. LaunchAgent permissions and login remain user setup. Optional voice-helper builds need Xcode's macOS 26 SDK; voice is not an onboarding prerequisite. |
| Nix profile or Home Manager | A Nix installation with `nix-command` and `flakes` enabled. It supplies build dependencies and PTY; pin and verify each member's source, negotiating capabilities across independently upgraded hosts. Home Manager owns service paths when that route is selected. Harness login and host permissions remain separate. |

A **fresh Mac onboarding rehearsal remains unverified**. The separate macOS workspace CI is
[manually disabled](https://github.com/compoundingtech/fractal/actions/workflows/macos.yml)
as checked on 2026-10-07. Native Apple Silicon release builds and archive smoke checks still run;
they do not establish a clean user's signing, permissions, harness login and first mission.
The approved clean Linux, Apple Silicon and Nix rehearsals must retain exact sources and actual
prerequisites before this status changes. [Report an install or upgrade problem](../when-something-is-wrong.md#report-an-install-or-upgrade-problem).
