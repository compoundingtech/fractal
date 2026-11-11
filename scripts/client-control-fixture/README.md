# Common client-control fixture packet

This is source preparation for **one shared fixture and seed across iOS, web and
TUI**, not permission to set anything up, expose, sign, pair, enroll, grant, open,
or tear down. No runtime evidence is shipped here. Source publication establishes
none of the acceptance evidence below.

## Inputs, identity and authorization

`expected-input.json` is immutable expected INPUT, never an actual run record.
Its generated ID fields stay null permanently. `packet.py digests` emits the exact
INPUT SHA-256 and executable packet SHA-256. The latter is SHA-256 over canonical
ASCII JSON mapping `expected-input.json`, `packet.py`, `seed.py`, `acceptance.py`,
`observer.py`, `policy.py`, and `storage-inventory.json` to their byte SHA-256s. A changed executable or INPUT requires
new consent. Tests/docs are not execution authority. Retain the reviewed
executable bytes and digest receipt. The immutable INPUT byte SHA-256 is
`d50f3c8d9a7e460d1c84f68dc4373af43e0029f7c80102fe304676393405f017`.

Every effect needs **standalone authenticated human consent bound to both exact
digests**, the intended scope and host, and the actual subject catalog when it
exists. Consent is not implied by this document, a source PR, a prior ask, saved
issuance state, or a successful binding check. This packet deliberately does not
invent a consent CLI or implement a signing substitute. Existing authenticated
human authorization and its durable receipt are prerequisites.

The producer is `agent/client-control-fixture/producer`; the person is
`person/client-control-fixture`. The exact suggested title is
`Client control fixture check`; body is `client-control fixture 0001`, body
SHA-256 `c3e53495d6bafca1a5ac1023ec1943ca3eec7c78e0517ddd86f3389827e6636f`.
Text must be one printable ASCII paragraph of 1–60 characters, no Markdown
`*_#[]\`>|`, JSON, or leading/trailing/repeated whitespace.

The root follows the shipped instance migration receipt: use
`$HOME/.fractal-instance/client-control-fixture` only when
`.fractal-migration-completed` contains exactly
`arn:lmig:smalltalk:2026-10-16-state-dirs-fractal\n` as bytes; otherwise use
`$HOME/.st-instance/client-control-fixture`. Derive this independently from the
operator's absolute HOME. No alternate root, socket fallback or symlink alias.
The privileged operator socket is exactly `ROOT/run/fractal.sock`; the client
socket is not an operator socket. `packet.py prepare` prints the derived paths
and human-only commands without executing them.

## Required source admission

Do not execute this packet until the reviewed source is landed and generated
contracts are admitted. The local-only fixture slice is #6281; grants and
item-binding resources are source dependencies #6289/#6296, canonical generation
#6274. A dedicated Fabric execution-context correction is also required: ambient
`Fabric::new()` is **not** acceptable fixture isolation. This packet does not
claim the prior source already enforces that correction. Do not hand-edit
code-generated clients, schemas or transport contracts to bypass admission.

Grant creation/revocation and test enrollment are not shipped grant CLI commands.
The reviewed daemon source defines privileged-Unix APIs:

- `POST /v1/client/control-grants`: `run_id`, `step_run`, `device`,
  `actor_agent`, `verbs`, `screens`, optional `item_id`, `max_commands`,
  `expires_at` (Unix milliseconds). For this packet require exact verbs
  `phone.inspect` and `phone.navigate`, screens `NowRoot` and `Attention`,
  actor above, and `item_id` equal to the actual **ask source subject**.
- Actual issuance resource: `grant_id`, `run_id`, `device`, `session`,
  `actor_agent`, `verbs`, `screens`, `item_id`, `max_commands`,
  `expires_at` (RFC3339), `state` plus normal resource metadata.
- `POST /v1/client/control-grants/revoke` with `{grant_id}`.
- `POST /v1/client/test-clients/revoke` with `{device}`.

There is **no dedicated grant GET/list HTTP route or CLI**. Do not invent one.
An already authenticated paired client can read the existing WebSocket
`GET /v1/client/collections/stream`, send
`{"kind":"subscribe","id":"commands","collection":"commands"}`, and retain
its `snapshot`/`changes` lifecycle resources (`kind: client-control-grant`). The
daemon, not this packet, determines live active state from trusted provenance,
current step claim binding, enrollment, pairing, budget and expiry. Generic custom
claims alone are not grant authority. Packet binding checks are explicitly
`verifies_binding_only`, not live authorization or consent.

## Human-only preparation and setup order

The commands here are **prepared, not executed**. Run preparation from a clean
operator shell, not an inherited `ST_INSTANCE`/`FRACTAL_INSTANCE` environment.
Keep evidence outside source files; never overwrite INPUT with runtime IDs.
Before effects retain the normal service readback:

```sh
fractal --json service status
python3 scripts/client-control-fixture/packet.py digests
python3 scripts/client-control-fixture/packet.py prepare
```

First obtain the required standalone setup/seed consent. Record proof that this
fixture root is fresh and no client has opened the new ask. Do not reuse an old
fixture and call it fresh.

### Dedicated Fabric node, separately scoped

The only permitted Fabric home is `ROOT/fabric`, control socket
`ROOT/fabric/run/control.sock`. Every invocation must pin both `FABRIC_HOME` and
global `--home` to that exact home; no independent control selector exists in
pinned Fabric `19e7f896`. `packet.py prepare` renders fully derived commands:

```text
env FABRIC_HOME=ROOT/fabric fabric --home ROOT/fabric up --foreground
env FABRIC_HOME=ROOT/fabric fabric --home ROOT/fabric status --json
env FABRIC_HOME=ROOT/fabric fabric --home ROOT/fabric down
```

These are notation with `ROOT` replaced by the independently derived absolute
root, not shell commands to copy unchanged. Start **and stop** need the separately
reviewed authenticated Fabric-mesh scope. Never use production's Fabric daemon,
HOME, NodeID or control socket. `status --json` supplies the actual `node_id`
after start; do **not** use `fabric id` as a purported read-only query (it can
create identity). The dedicated Fabric NodeID and Fractal daemon node identity
are recorded separately and must not be substituted for each other.

Retain a `fabric-evidence.json` object with `home`, `control_socket`, actual
`status` response, and nonempty `scope_receipt` referencing that authenticated
scope. This is an evidence wrapper, not a fabricated native API response.

### Fixture setup, health, then seed

The exact shipped setup invocation is:

```sh
fractal --instance client-control-fixture setup --person client-control-fixture
```

Choose service-managed setup. Record installed/running exact daemon unit
`st3-instance-client-control-fixture.service` and fixture replication unit status;
normal units are `st3.service` and `st3-replication.service`. A daemon started
without a service does not satisfy this packet's teardown precondition.

Save the outputs of these read-only commands externally; `SOCKET` below denotes
the exact independently derived `ROOT/run/fractal.sock` printed by preparation:

```sh
python3 scripts/client-control-fixture/packet.py services
python3 scripts/client-control-fixture/packet.py --socket "$SOCKET" health
```

The `services` output is `service-evidence.json` (`fixture` and `normal`, each the
actual shipped JSON service status report). Compare normal-unit status against
the pre-setup readback. Health is actual `GET /v1/health` over that socket and must
report ready, network policy `local-only`, and false fleet join, replication and
peer relay. Retain path, local-only and unit evidence **before** seeding. Refuse
normal or client sockets, aliases, escaped roots and network-enabled health.

Only after those gates and seed authorization, the prepared seed invocation is:

```sh
python3 scripts/client-control-fixture/seed.py --socket "$SOCKET" seed \
  --idempotency-key client-control-fixture-0001
```

Save its JSON as `seed-receipt.json`, record actual creation time and own seed
claim IDs, and retain the complete initial durable claim feed. Native asks have
**no unread bit**: output is always
`unread_bit=not-supported-for-native-asks`, never an unread PASS.

## Separate actual run record before journeys

After independently authorized pairing/enrollment/grant effects, retain each
client's complete actual issuance resource captured from the fixture operator
API. `grants.json` is an evidence object with exactly `ios`, `web`, `tui` keys,
each containing that client's **real** resource. Never invent generated IDs.
Clients require distinct device/session/grant identities for the same actual
run. Retain original issuance/enrollment request/response and authenticated
consent provenance alongside the record; the JSON wrapper is not provenance by
itself.

```sh
python3 scripts/client-control-fixture/packet.py --socket "$SOCKET" capture \
  --seed-receipt seed-receipt.json --grants grants.json \
  --service-evidence service-evidence.json --fabric-evidence fabric-evidence.json \
  --observer-policy observer-policy.json
```

Save output once as `actual-run.json` **before any journey**. Its separate full
catalog records actual daemon and Fabric node IDs, person, actor, run, ask source,
attention projection, navigation item, all durable claim IDs/subjects,
grant/device/session IDs, and `catalog_sha256` over canonical JSON. Full raw
claims, attention, grant resources and service/Fabric evidence are retained.
The expected manifest remains unchanged. The navigation `item_id` is the native
ask source (`seed-receipt.ask_id`), **not** its `attention_id` projection key.

## Independent before/after for every client

Before opening each client, run its independent check, retain output, and confirm
its actual grant binds **both** `NowRoot` and `Attention` and the exact recorded
ask ID. The same check runs immediately after that journey; neither another
client's check nor a combined final check substitutes. For each of `ios`, `web`,
and `tui`, substituting the actual client name:

```sh
python3 scripts/client-control-fixture/packet.py --socket "$SOCKET" check \
  --record actual-run.json --client ios --phase before
# Independently consented iOS journey, then:
python3 scripts/client-control-fixture/packet.py --socket "$SOCKET" check \
  --record actual-run.json --client ios --phase after
```

Each check reads current complete attention plus all durable claim pages,
requires one open native ask with exact title/body/person/producer, rejects
answer/read/acknowledge/close/cancel/done and read-position mutations, compares
seed receipts and own ask claim IDs to the frozen baseline, and verifies saved
grant item/screens/run/verbs/expiry binding. A saved resource is not proof of
live authority; the daemon enforces that on every actual command. Do not
request extra verbs or screens to work around refusal.

### Acceptance: initially unseen and unmutated

Three independent evidence sets are REQUIRED. Missing evidence keeps the client
row **OPEN**, and source preparation alone accepts none of them:

- **E1 creation provenance:** fresh fixture creation evidence; exactly one ask
  with actual recorded creation time; before the first journey, complete durable
  person-relevant claims contain only the seed's own claim IDs. Unsupported unread
  is explicit. Mere open attention is not proof that a person has never seen it.
- **E2 daemon state:** each client's separately retained before and after checks,
  equal claim/receipt ID sets and no daemon mutations as above.
- **E3 client-local state:** each profile is created new **for the fixture before
  pairing/enrollment**, never reset between journeys. Web starts as a new browser
  context with empty storageState; iOS uses an isolated test-app data container;
  TUI uses a dedicated fixture state directory. Baseline is captured immediately
  **after enrollment**; every journey reuses that enrolled key, pairing and session,
  without re-pairing. Use only declared isolated profile paths under
  `ROOT/client-profiles/CLIENT`, never normal browser/app/TUI profiles. Retain fresh
  creation and enrollment provenance separately from snapshots. The adjacent
  `observer.py` captures that baseline and each after-journey state plus a diff.
  Classify source-inventoried keys as identity, expected churn, or everything
  else, using explicit concrete per-client selectors. Identity is preserved by
  the primary daemon proof below; named expected churn is still diffed and scanned
  for markers. Unexplained, identity, read/seen/acted/closed or opaque/unscanned
  changes are FINDING, not normal churn. No blanket cache exclusions.
  Web snapshots are actual Playwright storageState exports, iOS snapshots actual
  copied test-app container files, TUI snapshots the dedicated state directory.
  An offline copy does not itself prove a fresh original container. Do not reset
  the profile, erase local markers, change identity, or re-pair to obtain a clean
  comparison. Acceptance says **E2 claim and receipt comparison unchanged, and E3
  diff recorded**.

A persisted read or seen marker is a **FINDING**, reported to Fractal Product
Lead, never a pass and never grounds for changing normal read policy. If source
proves the client has no local read state, retain exact `file:line` evidence as
an explicit declared limitation rather than manufacturing a clean observation.

### Named inventory and concrete observation policy

`storage-inventory.json` retains source-owner names and limitations, not a
fabricated runtime policy. Web's final source inventory supersedes its earlier
window-key guess: `wf.window-id` and `wf.clear@1` are **not** expected churn.
The named migration keys are conditional first-load migration writes, not blanket
allowances for user-action writes. Expand source descriptors such as gateway,
deployment or iOS member suffixes only to **actually observed** keys. Keep the
fixture layout free of restored Conversation panes: conversation read-position
keys are not allowed churn. Server-side glass writes are not covered by a browser
storage diff; any such effect needs separate scope and daemon evidence.

`policy.py` converts reviewed actual selector bindings to the observer policy:

```sh
python3 scripts/client-control-fixture/policy.py --bindings policy-bindings.json
```

Save its stdout as `observer-policy.json` before `capture`. The bindings wrapper
contains `inventory_sha256` (exact source inventory byte digest) and
`clients.CLIENT` with actual `profile`, `bindings`, `raw_omit`, `unobserved`,
and `substitutions`. For each source placeholder (`gw`, `deployment`, `node`,
`service-hex`), substitutions names the actual `value`, fixture/enrollment
`record_id`, source `field`, and `capture_provenance`. Read these values from the
fixture run and its enrollment records, not the normal client. Every descriptor
is resolved before baseline capture, including descriptors for unavailable keys.
Each binding and limitation names the **resolved** source name, never `<gw>` or
another template. The compiled policy freezes resolved names and provenance;
the run record freezes that full policy under `observer_policy_sha256`.
Snapshots refuse any remaining `<…>` anywhere in their policy.
Each binding contains a resolved source-inventoried `name`, `class` (`identity` or
`expected_churn`), an exact observed `selector`, and nonempty
`capture_provenance`. Selectors are relative captured filenames plus `#` and a
JSON pointer, or an explicitly named churn file. JSON pointer escaping is `~0`
for `~` and `~1` for `/`. No globs, guessed hashed filenames, broad cache
exclusions, nonexistent selectors, or whole-file identity exclusions. Operators
must review the mapping against the actual capture and source inventory; the
compiler validates bindings, it does not discover or attest their provenance.

Every inventoried identity name must be bound, or appear under `unobserved` with
`name`, `reason`, `capture_provenance`, and `source_evidence`. This includes absent
pending cookies and inaccessible iOS Keychain records: do not manufacture their
contents or selectors. Missing concrete identity policy hard-refuses capture.

Expected-churn stores must be **read-only JSON exports**, not raw binary
AsyncStorage/IndexedDB databases. Export through the client's existing read-only
path (web storageState plus read-only IndexedDB/sessionStorage exports; iOS
read-only test-app export) and retain capture provenance. Never call a
get-or-create API or alter a profile to obtain a snapshot. If no supported
read-only JSON export is available, that client's baseline remains OPEN:
compilation and capture refuse opaque churn rather than guaranteeing a FINDING
on every journey or silently approving unscanned state. Other changed opaque
files remain unscanned FINDINGs. TUI's named feed cache model snapshot, drafts,
glasses and prefs are expected churn only at their actual concrete JSON paths;
no whole cache subtree is exempted.

For TUI, capture the **whole isolated config/state/cache trees**, including
feed cache, prefs, glasses and drafts. The private profile's actual relative
filename must be listed in `raw_omit`; Python never opens it. The TUI-owned
read-only helper, source #6294 `1f8c81a51bd06e8df914b07eb783bd16b792354d`, is:

```text
fractal tui client-control identity --profile DEDICATED_PROFILE --json
```

`DEDICATED_PROFILE` denotes the actual isolated profile, never the normal
`stui-devices.json`. Save the helper's safe stdout export under an explicit
filename inside the captured tree. Its fields are `public_key`, `fingerprint`,
`device_id`, `session_actor`, `profile`; the embedded profile replaces only
`devices[].signing_key.pkcs8` and `devices[].session.credential` with
`{redacted:\"[REDACTED]\",sha256:<original UTF-8 SHA-256>}`. Never emit those original
secrets or parse the original private-key file in the packet. The binding and
identity artifacts require `sanitized_profile_evidence`; bind only the actual
safe export's signing-key and session JSON subtrees. No `devices complete`
invocation: that would pair.

Observer invocation is source-prepared, with actual evidence paths and IDs:

```sh
python3 scripts/client-control-fixture/observer.py snapshot --client tui \
  --profile \"$ISOLATED_PROFILE_TREE\" --item-id \"$ACTUAL_ASK_ID\" \
  --policy observer-policy.json --identity identity-evidence.json
python3 scripts/client-control-fixture/observer.py diff \
  --before enrolled-baseline.json --after after-journey.json
```

Snapshots/diffs print JSON to stdout; there is no `--output` flag. The first
snapshot is immediately after enrollment. Reuse that baseline, profile, key,
pairing and session for all subsequent journeys; never reset to obtain clean data.

### Primary and secondary identity evidence

Primary proof is daemon-side for every client: retain actual authoritative
`custom.client.test-enrolled` records before and after, with exactly unchanged
record and ID, and every journey's authoritative
`custom.client.command-receipt` containing `verifiedResultSignature` and
`signedResult`. Each verified signature's key must exactly match the single
enrolled `publicKey`; device/session/grant/run/command bindings must match.
Fingerprint is lowercase hex SHA-256 of the **65-byte uncompressed SEC1 point**
`0x04 || X || Y` decoded from the `p256:` public key.

Generic user-authored custom claims do not establish this proof. Retain actual
read-only fixture-store extraction and trusted operation/receipt provenance.
There is no invented enrollment GET, receipt GET, or offline verification CLI.
`acceptance.py` consumes authoritative **daemon-verified** receipt artifacts and
checks their bindings; it does not independently verify ECDSA or promote a
signature-looking JSON value to cryptographic evidence. If independent offline
verification is required, the reviewed source APIs are
`fractal_schema::client_control::client_result_canonical_bytes` and
`smallclaims::principal::verify_key_signature`; use the admitted real library
implementation, not a substituted signature format. Until authoritative
provenance and admitted source are available, identity proof remains OPEN.

`clients.CLIENT.e3.primary_identity` contains `store_readback_provenance`,
`trusted_receipt_provenance`, raw `enrollment_before`, raw `enrollment_after`,
actual `enrolled_fingerprint`, `journey_command_ids`, and complete raw
`verified_receipts`. Every actual journey command must be represented exactly
once. Before/after enrollment is the actual durable row, including `id`, `kind`
and `body.fields`, not a hand-made subset.

Local readback is secondary. `secondary_identity` contains `mode`, `provenance`,
and exact `source_evidence` file:line references:

- Web mode `readonly-local`: use `readExistingPublicKeyPoint()` from
  #6278 `4619dc87`, or open the existing
  `fractal-client-control-identity-v1` IndexedDB without upgrade (abort
  `onupgradeneeded`), readonly `identity.get('p256')`, export public SPKI and take
  the SEC1 point. Never call `getOrCreate`.
- TUI mode `readonly-local`: use the safe helper above. Map its actual
  `fingerprint` to observer field `device_key_fingerprint` in the evidence
  wrapper, retaining provenance; this mapping is not a new CLI output field.
  Correlate public key/device/session/enrollment against the primary store row.
- iOS mode `declared-unavailable`: the local API is JS-only, not XCUITest-readable.
  Do not add a fake screen/readback, invoke key creation, or claim local key
  extraction ran. The authoritative daemon proof covers identity.

Pairing ID is not persisted locally by **any** client. Every row must carry a
`pairing_limitation` with `reason` and exact `source_evidence` file:line list.
Observer `pairing_id` and enrollment ID come only from the actual read-only
fixture-store record correlated by fingerprint, never from a fabricated local
API. The identity `evidence` field records whether each value came from the
secondary local readback or primary store.

### Retained artifact validation

`acceptance.py` validates real retained artifacts offline. It does not create
observations, attest freshness, replace authenticated provenance, or authorize
a journey:

```sh
python3 scripts/client-control-fixture/acceptance.py \
  --record actual-run.json --evidence acceptance-evidence.json
```

The evidence wrapper has `catalog_sha256`, `e1`, and `clients` with exactly the
three client rows. All timestamps below are actual Unix milliseconds:

- `e1`: `root`, `fresh_fixture:true`, `creation_provenance`,
  `fixture_created_at_unix_ms`, `ask_created_at_unix_ms`,
  `before_first_journey_unix_ms`, `ask_count:1`, actual `ask_id`,
  `complete_claim_feed:true`, `seed_claim_ids`, `person_relevant_claim_ids`,
  `claim_feed_provenance`, and
  `unread_bit:"not-supported-for-native-asks"`. Both claim ID lists must equal
  the actual seed's own claim IDs; retain the original complete feed and exact
  person-relevance query/capture provenance, not merely an asserted list.
- Each `clients.CLIENT.e2`: `before` and `after` are that client's complete
  `packet.py check` receipts, not hand-authored replacements. The validator
  compares their client/phase, actual catalog digest, stable receipt digest,
  complete seed state/claim ID sets, exact item/screens/grant binding and time
  ordering.
- Each `clients.CLIENT.e3`: actual isolated `profile`, `fresh_for_fixture:true`,
  `never_reset:true`, `reused_enrolled_session:true`, `freshness_provenance`,
  `enrollment_provenance`, `capture_provenance`,
  `profile_created_at_unix_ms`, `enrolled_at_unix_ms`,
  `baseline_at_unix_ms`, `after_at_unix_ms`, and raw observer artifacts under
  `baseline`, `after`, `diff`. Baseline immediately follows enrollment and
  precedes journeys; profile creation precedes enrollment. Those provenance
  references must establish the actual original client/container, not only a
  local copy or attestation flag. An optional `declared_limitation` has a
  nonempty `reason` and `source_evidence` array of exact `file:line` references.

Observer snapshots retain `version`, `client`, `profile`, `item_id`,
`identity:{device_key_fingerprint,pairing_id,evidence}`, exact named
`exclusions`, `expected_churn`, `raw_omit`, `policy_sha256`, all safe `entries`
hashes, `opaque_files`, and `read_seen_markers`. The actual run freezes the entire
concrete policy and `observer_policy_sha256` before journeys. Diff output is
recomputed from the raw artifacts during acceptance; a substituted diff is refused.
Diffs retain `identity_changes`, `expected_churn_changes`, `unexplained_changes`,
`unscanned_changes`, marker findings and identity invariants. Strings containing
JSON (such as storageState values) are scanned too. Opaque changes cannot be called
clean even if the filename is expected churn.

Output rows are OPEN for missing/inconsistent evidence; FINDING for any marker,
identity, unexplained or opaque/unscanned changes (report to Fractal Product Lead);
or EVIDENCE_COMPLETE for a complete retained artifact set. Named expected-churn
changes alone can produce DIFF_RECORDED, but are never hidden. EVIDENCE_COMPLETE
validates the artifact contract only: it is not consent, live grant authority,
an unread PASS, or a claim that an observer was actually run.


## Human-only teardown, fresh consent required

Retain evidence first. Generate (do not automatically execute) exact prepared
revocations and fixture stop commands:

```sh
python3 scripts/client-control-fixture/packet.py --socket "$SOCKET" teardown-plan \
  --record actual-run.json
```

After fresh authenticated teardown consent bound to the packet/INPUT/catalog,
revalidate the exact fixture path and current unit evidence. Revoke every recorded
run grant via the prepared privileged-Unix request; revoke every recorded fixture
test enrollment by exact device; retain all receipts. Then stop/remove **only**
fixture services, preserving fixture state:

```sh
fractal --instance client-control-fixture service uninstall
```

Stop the dedicated fixture Fabric daemon only with its exact prepared pinned-home
`down` command and the separately scoped Fabric-mesh authorization. Never stop
production Fabric. Compare normal-unit readbacks immediately before/after:

```sh
python3 scripts/client-control-fixture/packet.py services --record actual-run.json
# Separately authorized fixture service uninstall and own Fabric stop only.
python3 scripts/client-control-fixture/packet.py services --record actual-run.json --expect-stopped
```

The after readback requires fixture services uninstalled/stopped and unchanged
normal units. There is no raw `systemctl` packet and no guessed unmanaged-daemon
stop. Normal roots, stores, sockets, sessions, grants/enrollments, profiles and
production Fabric are excluded. **State deletion is NOT AUTHORIZED**:
`uninstall --yes` is not a stop substitute and would require retained evidence
and separate later approval.

## Source checks for the main agent

No checks or effects were run while preparing this source. Main should run the
fixture Python unit suite once after all source owners finish, including
manifest/text, grant-binding, dedicated Fabric, normal-path/socket/symlink,
service-management, observer and acceptance-evidence refusal tests.
