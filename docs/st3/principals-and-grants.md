# Principals and grants

Status: Draft acceptance contract for [#867](https://github.com/compoundingtech/fractal/issues/867).

This document specifies the acceptance boundary for delegated person authority.
It builds on the [resource graph gist][gist], the [October 1 decisions][oct1],
the [namespace clarification][namespaces], the [October 3 restriction cases][restriction],
the [October 11 service requirement][service], and the [ownership decision][ownership].
The Fractal Product Lead (FPL) rulings in `message/ba6d372e66b9e97e` are normative;
where they differ from the gist, these rulings govern acceptance.

The detailed subsystem contracts remain [accounts](accounts.md), [Sekrets](sekrets.md),
[device signing](device-signing.md), [person delegation](person-delegation.md), and
[plugins](plugins.md). Existing operation-specific delegation is not a universal grant.
No team namespace, permission system parallel to grants, or machine-operator privilege
is introduced here.

## Reading the matrix

Milestones group acceptance evidence; they are not implementation instructions:

| Milestone | Acceptance boundary |
| --- | --- |
| M1 | This matrix accepted by FPL, plus the read-only service principal (#5019). |
| M2 | R1 keyed writers (#5099, then #5102), legacy history, and #5064. |
| M3 | Universal grants (#882), namespace/account enforcement, opt-in personal-fleet restrictions. |
| M4 | Credential adoption through Sekrets: gh before model logins. |
| M5 | Three-person Linux shared-box acceptance. |
| Deferred | Explicitly excluded from #867 closure below. |

Current state is a source/merge snapshot of origin/main
`bdff4e839bf` (2026-11-07), not a fleet-deployment certification.
Review amendments for gateway locks, tiles and native work leases are checked
against origin/main `beb9bb6dfc`; other state entries retain the snapshot above.
DONE means the specified implementation slice is present, PARTIAL means only part
is present, and NOT STARTED means no implementation of that criterion was found.
UNKNOWN explicitly marks unverified runtime behavior or an unresolved decision;
it never counts as acceptance. Verification methods below are required proofs,
not claims that those proofs were run for this documentation change.
PR numbers identify evidence or outstanding work, not presumed merge status.

Source abbreviations are repository-relative:

- **P**: `crates/smallclaims/src/principal.rs` (families, signatures, key grants).
- **J**: `crates/smallclaims/src/store/principals.rs` and `principals/judging.rs` (admission/cache).
- **R**: `crates/smallclaims/src/rules.rs`, `crates/fractal-kernel/src/rules.rs`.
- **A**: `crates/fractal-kernel/src/accounts.rs`, [accounts](accounts.md).
- **S**: `crates/fractal-secrets/src/sekrets/`, [Sekrets](sekrets.md).
- **D**: [device signing](device-signing.md), `fixtures/clients/device-signing-v1.json`.

## Identity, admission, and history

These criteria derive from gist §§1–4, its deterministic-sync prerequisites,
October 1's actor/on-behalf distinction, and FPL's legacy-history ruling.

**Pre-M2 gate:** if the #5102 author demonstrates that R05–R06 cannot be met
without a protocol change, the concrete case MUST go to FPL **before M2 starts**.
The gate's verification evidence is the author's concrete case and FPL's recorded
resolution; the availability ruling alone does not authorize a protocol change.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R01 | FPL accepts this matrix as the #867 closure basis, including explicit exclusions. | M1 | PARTIAL: this draft; FPL acceptance UNKNOWN. | Record FPL acceptance at the reviewed head. |
| R02 | Person, agent, node and plugin identities retain distinct actors; agent claims name the person on whose behalf they act, never impersonate that person. Keyless steps retain their initiator. | M3 | PARTIAL: P has four families and `ClaimSignature.on_behalf`; universal runtime propagation UNKNOWN. | Trace seat, mission-step and plugin claims through local and replicated writes; reject person impersonation. |
| R03 | Canonical claim bytes and signer, key and delegation references are verified on local and replicated admission. | M2 | PARTIAL: #1045, P/J implement signatures; R1 #5099/#5102 remain open. Fleet-wide enforcement UNKNOWN. | Valid/altered signature vectors; forged actor, key, chain and replay probes on every write path. |
| R04 | New ordinary writes require authorized keyed writers; warn/audit modes never silently become unsigned authority in enforce mode. | M2 | PARTIAL: #1051 supplies modes (R); #5099 then #5102 are unmerged. | Refuse fresh unsigned/unauthorized writes locally and after sync; test mode transitions. |
| R05 | Reachable pre-keying history remains legacy authority under existing admission rules; old history is not re-signed or rewritten. | M2 | PARTIAL: existing unsigned verdicts in J; #5102 STOP acceptance not established (UNKNOWN). | Replay a copied pre-keying graph; compare old envelope bytes, projections and authority before/after upgrade. |
| R06 | A fresh node unable to retrieve legacy history joins with the history it has and exposes a typed, visible “history unavailable” state; it neither rejects nor rewrites missing history. | M2 | NOT STARTED: FPL ruling; #5102 outstanding. Runtime state UNKNOWN. | Join with unavailable legacy source; assert successful partial join, typed CLI/UI state and later recovery. |
| R07 | A missing or held grant holds its dependents with a reason; no dependent becomes final while its authority is held. | M2 | PARTIAL: J has dependency/requeue state; #5064 remains open. | Deliver dependent before grant, hold parent, then release/revoke it; inspect every dependent verdict. |
| R08 | Cache and uncached admission agree, including revocation propagation, corruption, cold restart and adversarial arrival order; local cache is not replicated authority. | M2 | PARTIAL: J has content-keyed verdicts and invalidation; full differential proof UNKNOWN. | Random/adversarial cached-versus-recomputed tests; edit/delete/inject cache rows and restart. |
| R09 | Identical envelopes in canonical order yield identical projections/digests across three members after shuffled sync, checkpoints and restarts, within fleet-scale read/admission budgets. | M2 | PARTIAL: smallclaims sync exists (#1014); complete #867 convergence/performance proof and numeric budgets UNKNOWN. | Deterministic three-member fixture plus hundreds-of-thousands-of-claims benchmark; FPL accepts budgets (DQ8). |
| R10 | Person private keys stay on personal devices, absent from the shared box; daemon-held seat keys do not reach seats; rotation/revocation are verifiable. | M5 | PARTIAL: P/D, #1054/#1126/#1353; shared-box key absence and full lifecycle UNKNOWN. | Inspect shared host/seat access, sign from phone, rotate/revoke and test old/new keys without exporting secrets. |
| R38 | Claiming work, lease renewal and expiry are graph primitives usable by mission and plugin controllers; expiry never leaves a step both ready and working. | M3 | PARTIAL: missions already use native claim/renew (`crates/fractal-missions/src/store/work_renew_tests.rs`); the gap is generic reuse and the ready/working expiry regression, whose acceptance is UNKNOWN. | Exercise the same claim/renew/expire primitive from mission and plugin controllers; expire a working lease, restart/reorder sync, and assert one consistent non-working/ready state, never both. |

## Grants, namespaces, and accounts

These criteria derive from gist §§3, 9, 12 and the October 1/3 comments.
Free mode is the one-person case, not a competing authority mechanism.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R11 | A person issues grants identifying subject, verbs, resource kind, path scope, expiry and delegability; delegation can only attenuate parent authority. | M3 | NOT STARTED: universal #882 remains open; P key delegation and R rules are narrower mechanisms. | Issue/use an attenuated chain; refuse broader verb/kind/path/expiry/delegation and wrong signer. |
| R12 | An agent requests a grant and the person approves or refuses it on phone or stui; the approved grant expires. | M3 | NOT STARTED: #882; device signing alone is not grant approval. | Request from real agent, approve/refuse in both clients, then deny a claim signed after expiry. |
| R13 | Expiry uses signed claim time, not arrival/local clock; revocation invalidates dependent authority without backdating or replay widening it. | M3 | PARTIAL: P/J key lifecycle exists; universal grant semantics absent, stolen-key boundary UNKNOWN (DQ7). | Skew clocks, reorder/replay, revoke parent and attempt backdated claims against agreed DQ7 rules. |
| R14 | One person starts and works with existing commands, no key/grant prompts; unrestricted agents act as themselves with their person's authority. | M3 | PARTIAL: #887 free mode landed; fresh-install no-ceremony regression UNKNOWN. | Fresh-machine getting-started run; assert no new setup/prompt and unchanged workflow review gates. |
| R15 | Each person owns a personal namespace; own-namespace work is free by default, cross-person work/profile/terminal use requires the owner's grant; no team/group namespace exists. | M3 | PARTIAL: R namespace patterns; cross-person enforcement UNKNOWN, #882 open. | Three owners: allow own work, deny cross-namespace start/apply/attach/profile use, then allow exact grants. |
| R16 | A person can opt into restricted seats/services within their own namespace without sharing-person identity widening authority. | M3 | PARTIAL: R off/audit/enforce; complete native restriction UNKNOWN (#882, October 3 cases). | Bind restricted seat, graph applier and artifact publisher to one person; test their distinct allow/deny scopes. |
| R17 | One operation/subject decision governs CLI aliases, raw HTTP, client-v0, typed claims, nested KDL, command descendants and sync; unknown mutations deny for restricted principals. | M3 | PARTIAL: rules/admission exist; complete entrypoint parity UNKNOWN. | Exercise each October 3 entrypoint, mixed/run-local payloads and replicated forbidden claims; assert no effect. |
| R18 | Caller identity cannot be upgraded by environment, person headers, absolute binaries, descendants or loss of seat binding. | M3 | PARTIAL: S authenticates gateway callers; general native protection UNKNOWN. Same-user limitations remain documented in S. | Spoof ST_AGENT/ST_ACTOR and headers, detach children, remove binding; remain same principal or refuse. |
| R19 | Server-owned document/graph/run ownership controls writes, including retired history; names and edited desired owners confer no authority. | M3 | PARTIAL: operation-specific person delegation exists; full ownership/artifact binding UNKNOWN. | Allow own checkpoint; deny other's/shared bindings; reject forged commit+renderer/input+hash artifact replacement. |
| R20 | Preview performs no claim, binding or runtime write; refused mutations return operation, target and reason/grant failure. Restart-with-resume requires runtime authority, not apply authority. | M3 | PARTIAL: R verdicts exist; end-to-end preview/refusal/runtime-capability coverage UNKNOWN. | Compare store/runtime before/after previews; probe CLI/HTTP/client-v0 refusal and running-revision reporting. |
| R21 | Seats bind explicit accounts or same-owner pools; same-provider accounts run side by side, with per-account usage and quota switching. | M3 | DONE (implementation): #1084, A. Live multi-person behavior UNKNOWN. | Run two accounts concurrently, inspect usage attribution and force pool switch without transcript/login crossover. |
| R22 | Account-use grants govern who starts work on an account and under which namespace; no account selection bypasses owner authority. | M3 | NOT STARTED: A bindings are not universal account-use grants (#882). | Deny foreign explicit/default/pool account use, allow scoped grant, then revoke/expire and retry. |

## Sekrets and credential adoption

These criteria derive from gist §§10–11. Model-login adoption does not imply
approval or implementation of the deferred authenticating model proxy.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R23 | Opt-in Linux gateway runs as a separate credential user, checks kernel caller/daemon revision/person authority, and returns results, never credentials. | M4 | PARTIAL: #1092/#1742/#1752, S; local attestation exists, universal person grant chain absent. | Real gh PR call under authorized seat; forge each identity layer; attempt credential reads from seat and tile. |
| R24 | Brokered calls record actor/profile/command/result claims; global/person lock and broker failure deny loudly without fallback credentials. | M4 | PARTIAL: S call audit exists, but `gateway.rs::connection_from_peer` dispatches `Request::Authorized` to `authorized()`, whose `usable()`/token/HTTP path omits `lock_for()`; only shell `run()` checks locks. Authorized-HTTP lock enforcement is outstanding; live fail-closed proof UNKNOWN. | Inspect successful/refused audit claims; lock one person or all persons and require refusal on BOTH shell and authorized-HTTP paths; stop gateway and assert refusal without fallback. |
| R25 | gh adoption moves credentials only on request, proves broker access before deleting originals, is idempotent, repairable and reversible. | M4 | NOT STARTED: #1753 closed unmerged; no current adoption command established. | Fresh-machine gh adoption, real broker call, second no-op run, interrupted migration, repair and rollback. |
| R26 | Model-login adoption follows gh; adopted credentials are unreadable to seats, remaining credential paths are tool-sandbox denied and doctor reports leftovers. | M4 | NOT STARTED: #1753 did not implement model adoption; A still uses login directories. Sandbox sufficiency UNKNOWN (DQ6). | Adopt supported model logins on fresh machine; run real harness, read/exfiltration probes, repeat and reverse adoption. |

## Read-only service principal

The October 11 requirement and #5019/#3268 define this boundary. Read-only device
pairing or an unauthenticated local socket is not a substitute for service identity.

The claim-backed external-grant core is
`crates/fractal-api-core/src/api/client_v0/external_grants.rs`. Its closed profiles
keep `assistant/chatgpt` and `assistant/mcp` in the same assistant profile; service
issuance is disabled and service has no assistant route or disclosure rights.
Several active grants of a principal's own profile are allowed, but a different
active profile is refused. Historical `custom.client.assistant.v1.*` claim kinds
and absent-principal legacy ChatGPT grants retain their existing meaning.
Claims persist credential/code digests, never bearer tokens or pairing codes;
grant projections expose neither credential material nor its digests.

Service read admission uses a closed GET/scope registry on both the credential
gateway and bearer-authenticated native router. `read.claims` admits
`/v1/client/claims` and `/v1/client/claims/by-id/{id}` with the native readers'
1–500 bounds and numeric `after`/`before` cursors. `read.work` admits exact
`/v1/client/work-items/{subject}` reads without acknowledging wakes. Service work
responses are a separate typed DTO: subject, run/generation/step identifiers,
attempt, status, claimant, claim/submission incarnations, claim expiry, readiness
epoch, worker-reported flag, creation/update timestamps and typed execution clock
timestamps/counters only. Goals, constraints, titles, person answers, summaries,
blocked reasons and other native free-text subviews are excluded on both aliases.
`read.documents` remains a closed scope and its routes remain registered, but its
document namespace allowlist is empty: every service list/content/history request
is refused until a consumer namespace is declared. Native aliases use the same
service decision; the gateway continues to refuse raw routes.

Service claim bodies are deny-by-default, not field-name redaction. Only
`gate.result` and `work.claimed`, `work.renewed`, `work.progress`,
`work.submitted`, `work.failed`, `work.released`, `work.extended` disclose
schema-listed scalar receipt/state fields. Gate bodies expose `verdict`,
`operation`, `request`, `baseline`, `token_usage`, `stage`; the literal gate name
is excluded. Work bodies
expose `attempt`, `status`, `worker_reported`, `claimant`, `claim_incarnation`,
`claim_expires_at_unix_ms`, `readiness_epoch`, `extend_ms`, `clock`,
`wait_until_unix_ms`, `submitted_incarnation`, with the receipt/state claim's actor.
These support independent gate receipt and work-state validation without arbitrary
payload, summary, reason or handoff objects.
Every other kind returns only `store_index` and the fixed `withheld: true`
marker in pages, with no subject, id, actor, origin, kind, timestamp or body digest.
This includes declarations, grants and pending device/assistant pairings: even
the deterministic pairing subject can be a verifier and is not disclosed.
Nonallowlisted by-id requests return the same not-found response as nonexistent
ids. Service by-id admission first reads only the kind through the id index;
hidden and absent rows return before loading or parsing a body, avoiding a
body-size/JSON-parse existence oracle. Withheld rows retain page positions without gaps.
Service claim selectors (`subject`, `subject_prefix`, `kind` and `owner_run`, the
complete selector set) never match withheld rows: filtered store queries intersect
every selector with the disclosure kind allowlist in SQL before paging. Filtered pages contain only allowlisted claims,
never withheld markers or indexes; empty filtered pages retain the ordinary empty
page/cursor shape. Unfiltered paging still includes withheld positions.
Service `subject` selectors match literal canonical subjects only; services must
supply the stored canonical subject, not an alias. No identity-reference or
lifecycle resolution runs for service selectors. `owner_run` remains a literal
ownership comparison. Non-service reference resolution is unchanged.
The new client claim/work aliases admit only services; device, assistant and
browser clients gain no new routes. Existing native routes retain non-service
semantics. No declaration, assistant, action, upgrade, HEAD or unknown route is admitted.

The daemon writes only `custom.client.service.v1.read-window` as
`daemon/{origin}`, at most once per grant per 60-second wall-clock window.
Admission-refused requests write no audit claim. Service identity is never an acting party,
and browser ingress refuses a browser cookie or launch token combined with
Authorization. Browser admission requires a single credential source. Service-grant fixtures
are inside `#[cfg(test)]` modules; production
grant append still rejects service issuance. This is not an OS sandbox:
same-UID callers dropping the credential on an unrestricted socket remain
outside credential enforcement (R18).

Production issuance remains disabled pending owner CLI/rotation, admission
atomicity with concurrent-admission proof, bounded/indexed profile admission,
fleet-wide mixed-profile exclusion, mixed-version/downgrade safety, and auth-path
p95 under concurrent completions. No claim is made that M1 lifecycle or the
real merge consumer acceptance is complete.

New assistant BEGUN claims bind their keyed `code_hash` to the stored full
`assistant-grant/<hex>` grant ID, with `verifier: "hmac-sha256-v1"` and
`verifier_key_id` identifying the member-local secret. The secret is excluded
from replicated claims and logical claim backups; bearer credential digests are
unchanged. Unmarked legacy challenges remain usable only while unexpired and
with a lifetime no longer than 300,000 ms from BEGUN acceptance. Unknown/malformed
markers or a lost or replaced verifier key require fresh pairing, not legacy
fallback. See [assistant verifier recovery](external-assistants.md#pairing-verifier-and-recovery).

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R27 | A host user service outside a harness authenticates as a stable non-person subject provisioned by a person or a node, not as its invoking agent. | M1 | NOT STARTED: #5019/#3268; plugin/assistant primitives do not prove this consumer contract. | Restart unattended service and invoke from two agents; every read retains the same service subject. |
| R28 | Its credential file rotates and revokes without changing the stable subject; retired/revoked credentials cease to authenticate. | M1 | NOT STARTED: #5019; general device pairing is insufficient. | Rotate, use old/new credentials, revoke, retry after daemon restart; audit subject continuity. |
| R29 | Scoped native routes allow claims by id and paged claims, work-item reads and document reads at admission, queue proof and final merge. | M1 | PARTIAL: closed receipt/state claim bodies and exact work reads implemented; document reads: allowlist empty until a consumer namespace is declared. Issuance remains disabled and real merge-consumer acceptance is UNKNOWN (#5019). | Real merge consumer reads all four families, traverses pages without gaps and validates exact receipts at all three stages. |
| R30 | Every write is refused under the service credential, including free mode, raw routes, client actions, claims, documents and runtime/admin operations. | M1 | PARTIAL: central GET/scope admission and both-router refusal fixtures implemented; production lifecycle acceptance remains UNKNOWN (#5019). | Enumerate mutation routes/aliases; negative-write suite asserts structured refusal and zero store/runtime effects. |

## Shared box and messaging

These criteria derive from gist §12 and October 1's shared-box and open-message decisions.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R31 | Three people each run a mission on one fresh Linux shared fleet, in personal namespaces and on their own accounts/profiles. | M5 | PARTIAL: A/S/[tiles](tiles.md) exist; three-person acceptance UNKNOWN. | Three real client identities, three missions, inspect claim/seat/account/profile attribution. |
| R32 | Other people's namespaces/accounts/profiles/optional signed-attach terminals refuse access without grant; node operation adds no graph authority beyond delegated rights. | M5 | NOT STARTED: full #882/shared-box boundary; operator remains trusted for machine access, not person signing. | Cross-person negative/positive grant matrix, expiry/revoke; operator cannot forge absent person key. |
| R33 | Personal machines connect as clients, not shared-fleet participants; the server has no reverse fabric exec grant, and daemons launch only own-host seats. | M5 | UNKNOWN: the gist defines the requirement, not implementation evidence; concrete shared-topology enforcement and negative-exec proof have not been verified. | Inspect membership/grants; attempt reverse exec and foreign-host launch from shared box, assert refusal. |
| R34 | Messaging between all people/agents in the fleet is open by default, independent of cross-namespace work grants; refusal is opt-in. | M5 | PARTIAL: #887 preserves messaging; three-person default/opt-out proof UNKNOWN. | Pairwise messages among three people and their agents without work grants; test explicit refusal separately. |

## Explicit deferrals

These exclusions are FPL decisions, not missing prerequisites silently counted as DONE.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R35 | Plugin-defined principal kinds, generic status/UI and generic verbs support arbitrary kinds without kind-specific code. | Deferred | PARTIAL: P hard-codes families; schema registry/plugins exist; arbitrary-principal end-to-end behavior UNKNOWN. | Future plugin-kind acceptance; not a #867 closure gate. |
| R36 | Resource graph and Sekrets build independently without agent/mission dependencies or behavior changes. | Deferred | PARTIAL: #907/#917/#1014 extracted smallclaims; #1751 standalone Sekrets closed unmerged; independent-build proof UNKNOWN. | Future independent builds and behavior comparison; not a #867 closure gate. |
| R37 | An authenticating model proxy holds/refreshes real tokens while harnesses receive placeholders. | Deferred | NOT STARTED: no proxy established; provider-terms answer UNKNOWN (DQ6). | Provider-terms approval plus proxy/credential isolation probes; not a #867 closure gate. |
| R39 | Opt-in Linux seats run under separate Unix users with per-seat scopes; a seat cannot read the person's files. | Deferred | PARTIAL: [tiles](tiles.md) implements separate tile UIDs, namespaces and root-owned cgroups; S's “What is not built yet” leaves harness-in-tile integration outstanding. Real-seat isolation acceptance UNKNOWN. | Future real-harness isolation test: inspect UID/scope, deny protected person-file reads and preserve isolation-off behavior; not a #867 closure gate. |

R35–R37 are deferred because they are **not needed for delegated person authority**.
R37 additionally waits for a provider-terms answer. Existing schema/plugin and crate
boundaries remain useful evidence, not completion of these larger gist phases.
R39 is deferred because **no per-seat isolation inside one person's fleet is needed
for delegated person authority** (FPL, `message/178ff3845421e66d`); it is not part
of M5 acceptance. R38 remains in M3.
M5 tests isolation between persons, not between seats of one person.
Cross-fleet message forwarding (“shovel”) is subsequent scope in the gist, not a
replacement for R34's within-fleet open messaging.

## Governance and release rules

FPL accepts each milestone against this matrix. That acceptance replaces Nathan's
sign-off, including #5019; Nathan's later review is welcome but is not a gate.
FPL acceptance of this PR is the M1 **acceptance basis**, not acceptance of the
unimplemented service principal or evidence that M1 behavior already passes.
The driver owns specification/integration; existing PRs remain with their authors.
Compounding Manager reviews and lands changes after the current main freeze,
with an independent reviewer on every PR.

Grants, credentials, Sekrets adoption and service-principal PRs require a
**security-reviewer pass**. Grant-admission and credential-adoption PRs are
high-risk and additionally require **Johannes's acceptance**.
Johannes owns the q32 live-grant freeze; Manager (`agent/dotfiles/manager`) files
the native ask before M3, and no matrix approval itself releases that freeze.

Each of M2, M3, M4 and M5 ships in its **own tag**, with a release note and a
fleet-compat run against **v0.3.29 and the then-current tag**. Compatibility evidence
includes old/new members, join/restart, copied legacy history, coordinated
upgrade/rollback and offline-member rejoin; incompatible versions remain visible
as last-seen rather than silently syncing incompatible authority.
No device-scope or pairing changes are allowed until phone pilot **P2** is done,
unless FPL explicitly approves an exception. Existing phone signing does not prove
P2 completion (UNKNOWN here).

The gist's non-deferred rollout prerequisites gate **M2 rollout** (risky write-path
changes reaching fleet hosts), not the landing of M2 code behind its existing
off/audit modes. The order is measured CI capacity, gated deployment and rollback,
then the fleet-size CI benchmark after FPL accepts DQ8 budgets. All three receipts
MUST exist before M2 rollout and remain applicable to subsequent risky write-path
rollouts; code landing behind off/audit modes need not wait for them.

| ID | Criterion (observable and testable) | Milestone | Current state and evidence | Verification method |
| --- | --- | --- | --- | --- |
| R40 | Measured CI queue and completion capacity is sufficient for the risky admission/signing workload before M2 rollout; no increase is required if measurement is sufficient. | M2 | UNKNOWN: no qualifying queue/completion capacity receipt is verified here. | Run representative concurrent required jobs and record queue/completion capacity; FPL accepts sufficient measured capacity before R41 and M2 rollout, not off/audit code landing. |
| R41 | Deployment is mission-driven, checks health after deployment and automatically rolls back an unhealthy release before M2 rollout proceeds. | M2 | UNKNOWN: no qualifying copied-fleet health-gate/automatic-rollback rehearsal is verified here. | After R40, rehearse on a copied fleet using the actual deployment mission; the existing deploy controller's health gate and rollback may satisfy this gate if the rehearsal demonstrates them. Inject failed health, observe automatic rollback and restored health, and retain evidence before R42 and M2 rollout. |
| R42 | A fleet-size read/admission benchmark runs in CI and meets FPL-accepted DQ8 budgets before M2 rollout; the benchmark gate waits for those budgets. | M2 | UNKNOWN: R09/DQ8 record the missing complete proof and numeric budgets; no qualifying CI receipt is verified here. | After R41 and FPL's DQ8 budget acceptance, run the hundreds-of-thousands-of-claims fixture in CI; publish each read-path/admission timing and budget comparison before M2 rollout, not off/audit code landing. |

## Open decisions

Owners below are decision accountability, not claims that a decision was made.
FPL owns product/security acceptance; the driver supplies implementation evidence.

| ID | Still-open question and resolution evidence | Decider |
| --- | --- | --- |
| DQ1 | Linux/macOS node/person key storage, presence and recovery: which storage meets R10 without a seat-readable person key? Device signing settles only the paired-device slice. | FPL, with security-reviewer recommendation. |
| DQ2 | Universal grant representation: native claim kind or Biscuit-in-claims? Decide one format and attenuation/admission contract for R11–R13. | FPL, with driver proposal and security review. |
| DQ3 | Exact checkpoint/provenance and late-join verification for pre-keying history? FPL has settled availability semantics in R05–R06, not this remaining encoding/proof design. | FPL for contract; driver for implementation. |
| DQ4 | Nix/Home Manager ownership of users, scopes and Sekrets services? Existing installations do not establish the complete shared-box declaration contract. | Compounding Manager proposes; FPL accepts isolation boundary. |
| DQ5 | Principal-aware read surface beyond R29: actor, grant provenance and “what may I do here” API parity? | FPL, with client/driver proposal. |
| DQ6 | What do provider subscription terms permit, and is the interim harness tool sandbox sufficient? No provider authorization is inferred from working logins. | Johannes for provider-terms approval; FPL with security review for sandbox acceptance. |
| DQ7 | Exact stolen-key/backdating/quarantine and root-recovery rules, including canonical revocation boundary? | FPL, with security-reviewer pass and Johannes acceptance for admission changes. |
| DQ8 | Numeric fleet-scale read/admission budgets and acceptable benchmark fixture? Gist requires budgets but gives no numbers. | FPL, with driver measurements. |

The gist's seventh question (clients versus server-only fleet members) is resolved
by its shared-node contract: personal machines are clients; the shared server owns
its fleet (R33). Team ownership and operator privilege were resolved on October 1;
they are not open design questions.

[gist]: https://gist.github.com/myobie/18a1d1f2848b2b0b1cc71b5d3eb769e2
[oct1]: https://github.com/compoundingtech/fractal/issues/867#issuecomment-5932004880
[namespaces]: https://github.com/compoundingtech/fractal/issues/867#issuecomment-5932043503
[restriction]: https://github.com/compoundingtech/fractal/issues/867#issuecomment-5970171476
[service]: https://github.com/compoundingtech/fractal/issues/867#issuecomment-6113966382
[ownership]: https://github.com/compoundingtech/fractal/issues/867#issuecomment-6707173182
