# Fractal fleet compatibility

## Required rolling window

**R49 Adjacent-release fleet window** requires this: adjacent published releases
**N and N-1 must coexist for at least 14 days** without lost work, duplicate
schedule fires, or blocked schedules. Features that need all members activate
only when every active member advertises support. An older-release member that
returns within **30 days offline** must not block or corrupt the fleet. Downgrade
is not required.

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
using `BUILD.json`, `RELEASE.json`, `fractal --version --json`, and the daemon's
`machine_version` in `fractal doctor --json`. Downloading or installing an archive does
not restart or replace the running daemon **process**. It does replace installed
command files and, on macOS, the app bundle. Until an explicit restart, the daemon
keeps its current build while new CLI invocations use the installed build. Follow
the [upgrade procedure](../site/guides/upgrading.md) rather than treating installation
alone as a daemon upgrade.

### Updating the deployed-main baseline

The fleet deploys exact green main commits, not release branches or stable tags.
`.github/fleet-compat-baseline.json` names the oldest exact source still deployed,
including a transitional non-main pin until that deployment advances.
`linux-fleet-compat` tests the candidate against that source. Stable-release
publication advances the messaging baseline only; it never moves a main-source
fleet baseline.

Updating a deployment pin includes updating this file in the pin's rollout plan:

1. Read the deployment configuration's locked source SHA. During a staggered
   rollout, keep the oldest still-running source until every member has advanced.
2. Build that exact source with the existing **Fractal tag release**
   `workflow_dispatch` inputs `purpose=compatibility`, `sha` and a distinct
   `request_id`. This read-only mode requires a full commit SHA reachable from an
   origin ref and records those refs in its validation log. It grants no stable
   or edge publication authority and leaves the candidate mode's first-parent
   main gate unchanged. This reuses the
   normal native archive builders; compatibility CI does not compile a second
   historical workspace on every run.
3. Download the run's assembled `fractal-release-attempt-N` artifact. Verify
   `SHA256SUMS`, `RELEASE.json`'s exact source, and both native archives'
   `BUILD.json` source and target. Publish these verified archives, `SHA256SUMS`,
   and `RELEASE.json` once as the prerelease `fleet-SHA`, with a tag pointing at
   that exact commit. Create the draft without assets, upload the verified files,
   then publish. Never replace a published baseline's assets. A mutable `edge`
   URL is not a pin: the next main publication replaces its bytes.
4. Run `scripts/compat-baseline-release repin-fleet fleet-SHA`. It checks the
   published manifest, dereferenced tag, archive checksums and embedded metadata,
   and writes only the fleet baseline. Commit the result with the rollout's
   compatibility change.
5. Require green `linux-fleet-compat` at the candidate head and retain its
   `compat-baseline: fleet source=SHA` log line with rollout evidence. A pin
   change is not complete without this baseline update. Keep the rollout
   canary proof separate from the mixed-version CI proof.

The public workflow does not read private deployment configuration or poll for
pin changes. The explicit rollout step owns synchronization. The pinned archive
remains available independently of short-lived Actions artifacts and rolling
edge publication.

### Service-grant refusal on the pinned baseline

The service downgrade fixtures target the exact source in
`.github/fleet-compat-baseline.json`. Service credentials remain disabled
in production. A service grant must use a `plugin/<host>/<name>` principal,
never an assistant principal. The candidate refuses service-profile issuance
and authentication, including the counterexample `assistant/chatgpt` plus
`profile=service`; the closed assistant pairing grammar cannot issue that shape.

The deployed baseline understands service profiles and requires plugin service
principals. It refuses the offline-seeded `assistant/chatgpt` service-profile
counterexample with `forbidden` / `invalid service principal`. Plugin service
credentials cannot enter the assistant proposal route, and a service record with
assistant scopes is also refused. Replicated grants additionally fail the
authoritative-origin check. The ignored fleet tests exercise both issuer-origin
roles and a candidate → baseline → candidate restart on the same database, with
retained GRANTED/POLICY claims and live, expired, and revoked assistant controls.
Fixtures are signed offline by the candidate store using the stopped node's own
origin and existing node/principal keys: the public claims API refuses these
authority kinds, and production service issuance remains disabled.
A candidate-only test exercises the same offline seeding and refusal path.

The rollback fixture also checks owner inspection: the baseline's legacy
`assistant-grant` response kind preserves the actual plugin principal and service
scopes instead of projecting an assistant identity. A live assistant owner-read
control must still succeed. Neither a successful owner inspection nor this
assistant-route refusal evidence establishes general rollback support or disabled
service authentication on every historical binary. Any future service issuance
must retain the plugin-principal invariant and revisit compatibility before
enabling it.


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

### Declared admission publication

Publishing `declared-priority`, `declared-lane`, or `declared-budgets` (including an
empty budgets block) requires every active fleet incarnation to advertise
`features.declared_admission=1` in its latest own-origin, authenticated, signed
`daemon.started` claim anchored to the admitted incarnation. Publications containing
attention additionally require `features.declared_attention=1` from every active
incarnation, including the local daemon, in that same advertisement. The publication
gate requires signed, current, unambiguous membership and fails closed on missing,
false, conflicted, unsigned, or unanchored capability evidence and unfenced legacy
writers with `declared-admission-feature-barrier`. A refusal rejects the full
publication before any declaration is written; an old member never receives a
new-child publication through a refused write. Attention-free declarations continue
publishing in mixed fleets with version-1 declared-admission support. Upgrade all
active members or fence legacy peers before publishing attention.
Keep every member on a supporting binary while populated metadata exists.
This is an ongoing requirement. The gate checks support only at write time.
It does not prevent a member from starting an old binary later.
It does not prevent an old member from joining later.
An old member drops agent metadata and fails mission revision validation.
Remove attention with supporting readers before a mixed-version rollback; remove
all declared-admission metadata before rolling back to readers that lack version-1
declared-admission support.

New agent authoring accepts only peak `memory-max` and `disk-max` budgets.
`cpu-seconds` and `token-budget` are mission-run totals, including all retries,
and refuse new agent publication through CLI and API paths. Historical agent
readers and wire types retain those fields unchanged; replay is unchanged.
The compatibility assessment found no published uses: the supplied search of
`schickling/factory` at `origin/main` (`b2dacb7`) and dotfiles `nixpkgs/smalltalk`
found zero matches for `declaredBudgets`, `tokenBudget`, `cpuSeconds`,
`token-budget`, or `cpu-seconds`. This evidence is scoped to those sources,
not a claim about every external declaration.

This per-feature gate does not establish the general adjacent-release guarantee
described above. See [declared admission](priority.md#declared-admission).

### Agent charter publication

An agent's `description`, its charter, holds up to 8,192 UTF-8 bytes. A member that
does not advertise `features.agent_charter=1` bounds it at 1,000 bytes whenever it
parses declaration KDL: when it publishes or previews, and when it materializes the
agents a mission declares for a run it evaluates. It admits replicated desired claims
and reconciles a top-level agent from its stored declaration without that check.
Publishing an agent declaration, directly or inside a mission, whose charter exceeds
1,000 bytes therefore requires every active fleet incarnation to advertise
`features.agent_charter=1`, under the membership and evidence policy of declared
admission publication. Like declared admission, the gate fails closed without signed,
anchored fleet membership: a standalone member with no anchored fleet cannot publish a
charter over 1,000 bytes. A refusal fails with `agent-charter-feature-barrier` before
anything is written. A charter of at most 1,000 bytes needs no advertisement. The
gate checks support only at write time: keep every member on a supporting binary
while a longer charter exists.

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
| Earlier Fractal source | Read all intervening notes, including intermediate migrations and manual repairs. Rehearse a populated copy before rollout; no blanket upgrade or downgrade guarantee covers every earlier 0.x source. |
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

### Additive assistant admission index

Store initialization installs `claims_assistant_v1_principal_index` on
`claims(kind, COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'))`
with `CREATE INDEX IF NOT EXISTS`, including supported existing stores. This
matches the parameterized kind/principal admission lookup; a missing principal
continues to mean `assistant/chatgpt`. Existing claim rows and signed bodies are
not rewritten, and no projection replay or manual SQL is required.

This derived access path does not change `STORAGE_VERSION` (20),
`RULES_VERSION` (26), the claim registry, or their storage, checkpoint and
replication digests: row formats, vocabulary, shared projections and retention
semantics are unchanged. It follows the schema-on-open index installation used
for the assistant mission-run collection indexes. The first open builds the
index synchronously over retained claims. On a 16-core x86_64 benchmark host
(32 threads, Linux), exact index creation on a synthetic
1,000,000-claim real Store schema took 1.525 seconds and grew the main database
by 59,518,976 bytes. This measures index-build work, not complete daemon startup
or cold-disk restart time; local history and hardware determine first-open delay.
Subsequent opens reuse the index without changing the schema cookie. Older
binaries can ignore the additional index; SQLite continues maintaining it on writes.

Admission excludes revoked and effectively expired history in SQL while
preserving the latest-policy and malformed-fields validation semantics. SQL work
still scales with matching principal history, but inactive records no longer
hydrate into Rust. With 10,000 expired grants per principal and concurrent pairing
completions, real authenticated summary GET p95 on that benchmark host was 54.154 ms
(approval-once) and 46.173 ms (caller-bound-allow), compared with 965.184 ms and
933.049 ms on baseline `98e550cb75` (200 samples per mode). Both indexed modes
meet q39's 100 ms budget. The opt-in `daemon_cost::assistant_admission` tests use
production daemon routers over bound Unix/TCP sockets, real bearer middleware,
and distinct build directories for the baseline and indexed worktrees.

Subscription deferral counts keep their request-history index explicitly selected,
following the existing plan-sensitive history-query convention. Without `ANALYZE`,
SQLite's default estimates can select a subject/kind history scan after an unrelated
kind-leading index is added; request filtering must remain an index seek rather
than decoding every deferral for that subscription.

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

The [boot canaries](../../scripts/fractal-boot-canaries/README.md) use real Fractal processes with
stand-in providers; they do not prove real vendor login, model turns or every installed vendor
version. Native admission probes identify the exact installed build, and their exceptions expire
when the build identity changes. Follow [admission diagnosis](seat-deploys.md#harness-admission-at-the-next-launch)
rather than bypassing a failed boundary without understanding it.

### Rolling cross-build evidence

Cross-build tests pin exact sources in
[the fleet baseline](../../.github/fleet-compat-baseline.json) and
[the messaging baseline](../../.github/messaging-compat-baseline.json).
Fleet compatibility compares the **oldest deployed main source → current candidate**;
messaging compares the **previous published stable release → current candidate**.
Both pins record a release tag, its full source commit and native release archive
URLs with hex SHA256 checksums. Fleet compatibility runs the extracted published
binary; historical messaging uses the native archive from its pinned stable release.

CI prepares each pinned native archive once per workflow. The Ubuntu producer uses an
`actions/cache` entry keyed by target and the pin's SHA256 across runs, then uploads the
verified bytes as an artifact retained for seven days. Artifact names include the producer's
`github.run_attempt`, so reruns never overwrite bytes used by another attempt. The producer
exports each artifact name as a job output; a consumer-only rerun uses that retained output,
not its own attempt number. Fleet and messaging share artifacts when their pins match.
Downstream jobs download the producer-selected numeric artifact ID to a private directory,
using the bounded transfer implementation from `scripts/ci-test-archive`. They check the
authenticated ZIP identity, size and digest before opening it, then reverify the pinned
archive's SHA256 and `BUILD.json` source/target before atomically publishing consumer bytes.
Artifact metadata must match the current run ID and its event's source identity: the PR head
from the event payload on `pull_request`, or `GITHUB_SHA` on push and merge-group runs.
The PR merge-checkout SHA is not the artifact API's workflow-run head.
`COMPAT_BASELINE_ARCHIVE` supplies those bytes, never a live release download. Producer job
IDs also namespace artifact names so ordinary and projected proof producers cannot collide.
Artifacts bridge GitHub-hosted, Namespace and macOS runners without
assuming their cache backends are shared. Missing or corrupt artifacts fail closed;
producer cache hits are also reverified. Evidence-only gates do not depend on archives.

On a preparation cache miss, `scripts/compat-baseline` first downloads the pinned browser
URL. HTTP 502, 503 and 504 receive two retries after 1 and 3 seconds. Once exhausted,
or on another HTTP 5xx or a timeout, it resolves the same release asset through GitHub's
API, with at most two attempts per API request and a 60-second timeout per request.
`GH_TOKEN` (or `GITHUB_TOKEN`) authenticates API requests without forwarding credentials
on redirects. Diagnostics on stderr identify which path supplied the archive. Both paths
must pass the same SHA256 and `BUILD.json` checks; `--save-archive` atomically retains the
verified original bytes for caching. `COMPAT_BASELINE_ARCHIVE` bypasses network access
and its digest is checked even when the extracted binary is already present.

After each stable publication, release automation opens a baseline maintenance PR against
`main`. It derives the source from `RELEASE.json`, verifies the dereferenced tag and the two
archives, and advances unheld stable pins while preserving the main-source fleet pin.
It does not change the published release commit: a release cannot contain its own
source SHA. A human reviews and merges that PR before
the next stable candidate can be published. Baseline-only maintenance does not itself create
another daily release.

CI compares the stable messaging pin with the actual published stable release list,
excluding drafts, prereleases and unpublished tags. A stable pin more than one publication behind fails.
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
