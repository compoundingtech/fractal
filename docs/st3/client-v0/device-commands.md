# Foreground phone commands

A device command is one typed, expiring request to a paired phone's foreground instance.
It is not a durable action, background job, push notification, or replay queue. This contract
adds the daemon/client boundary; a phone must implement the handlers before it can execute commands.

## Commands and responses

`device.command` posts `DeviceCommandRequest` to `POST /v1/client/device-commands`:

```json
{"api_version":"st3.client.v0","device":"device/device-7","kind":"phone.navigate","args":{"screen":"Conversation","params":{"target":"agent/worker"}},"ttl_ms":30000}
```

`device-command.get` reads `GET /v1/client/device-commands/{id}`. With `wait=true`, the
server reactively holds the request until the command is terminal (or the request is cancelled).
Use this wait or the existing collections stream for correlation, not repeated GET polling.
The path carries the entire command id, percent-encoded as one segment.

`device.result` posts `DeviceResult` to `POST /v1/client/device-results`:

```json
{"command_id":"device-command/019a0000-0000-8000-8000-000000000001","outcome":"completed","result":{"route":"Conversation"}}
```

All three operations return the normal response envelope with snapshot and `value: DeviceCommand`.
Generated TypeScript and Swift methods are `deviceCommand`, `deviceCommandGet`, and `deviceResult`;
Rust methods are `device_command`, `device_command_get`, and `device_result`.
TypeScript GET takes `{wait: true}` separately from transport cancellation options; Swift and Rust
accept a `wait` boolean. Waiting is server-side, not a client follow/poll helper.

The resource discriminator is always `kind: "device-command"`; the requested verb lives in
`command_kind`. The resource has the conventional `id`, `revision`, `updated_at` header plus
`command_kind`, `args`, `device_id`, `session_actor`, `issued_by`, `issued_at`, `expires_at`,
`state`, and optional `result` / `error_code`. Revisions change with state/results so ordinary
resource-window changes carry transitions. See [the command fixture](fixtures/device-command.json)
and [the phone result fixture](fixtures/device-command-result.json).

## Typed arguments

Every argument object is closed: unrelated parameters, identity overrides and arbitrary routes
are refused. The supported verbs and scopes are:

| Verb | Args | Issuer scope |
| --- | --- | --- |
| `phone.inspect` | `{}` | `read.projections` |
| `phone.navigate` | the restricted screen selection below | `read.projections` |
| `phone.message.read` | `{message_id: "message/…"}` | `control.messages` |

Navigation accepts only these screen/parameter combinations:

- `NowRoot`, `Agents`, `Missions`, `Usage`, `History`: no `params`.
- `Mission`, `Attention`, `Launch`: required `params: {id}`.
- `Conversation`: required `params: {target, sessionId?}`.
- `Terminal`: required `params: {terminalId}`.
- `Documents`: optional `params: {prefix?}`.
- `Document`: required `params: {reference, trail: string[]}`.

Request bodies are bounded to 16 KiB, with at most 8 KiB of serialized `args` or `result`.
Route parameter strings are nonempty, at most 4096 characters, and contain no control
characters; a document trail has at most 32 entries. `error_code` is 1–128 ASCII letters,
digits, `_`, `.` or `-`.

Inspection reads phone-local route, connection, foreground and held-window state. Navigation
changes phone-local UI only. Message-read addresses a message by its ID and must use
the phone's existing mark-read handler, scope checks and action fence/idempotency. Issuance does not grant
the phone extra authority: a limited phone can refuse a write even when its issuer may request it.
Result content is command-specific JSON, not a new resource or stream frame.

## Authorization and capability negotiation

Probe `GET /v1/client/capabilities` for `phone-commands` version 1 in state `granted` before
issuing commands or subscribing to `commands`. A paired phone receives this capability only on
the member that issued its current pairing. Issuers receive it under the existing acting-party
rules and command scopes. The feature grants no new scope. Old daemons omit the capability;
a client must not assume support. An unknown collection uses the existing stream `error` frame.

Persons can command only their own devices. Local agents may act for the person in free mode,
under the existing action rules; agent authority is not widened. A paired phone can report only
commands addressed to its own session, and only on its pairing's issuing member. A caller cannot
select a result's device, person, session or issuer in the body.

Collection visibility is session-scoped: a paired phone sees only rows addressed to its
exact pairing session; non-phone issuers see only rows they issued, subject to own-device
authority. A paired issuer commanding another phone correlates its own issued rows using GET
with `wait=true`, not its phone-delivery window. GET also permits that issuer to read its own
command under its current kind scope and original pairing grant. Knowing another person's
device or command id never grants visibility.
Revoked, expired, or remotely issued pairings cannot report outcomes as local phones.

## Foreground delivery and correlation

After the capability probe, a phone subscribes on its existing foreground collections socket:

```json
{"kind":"subscribe","id":"commands","collection":"commands"}
```

The server uses only the existing `snapshot` and `changes` resource-window frames, containing
`device-command` rows. There are no new frame kinds and no second socket. The addressed session
is the delivery boundary; subscriptions cannot override it. Phone windows contain only active
`pending` / `delivered` commands; terminal transitions remove them, never replay them.
Nonpaired issuer windows include their terminal outcomes in a newest-first window of at most
32 rows (a requested collection limit can reduce this). GET retains each outcome independently
of that window for the correlation period. Phones must never execute terminal or expired rows.

The default TTL is 30000 ms; a supplied TTL must be an integer from 1 through 60000 ms.
There may be at most eight pending commands per device. Admission beyond that bound is refused;
this is not an unbounded delivery queue.

## State, clocks, and retention

States are `pending`, `delivered`, `completed`, `not-foreground`, `expired`, `refused`, and
`failed`. Only `pending` and `delivered` are nonterminal. A phone result's `outcome` must be
terminal; it cannot report delivery or pending state.

The daemon clock alone adjudicates expiry. A phone result admitted strictly before `expires_at`
wins. At or after the deadline, the daemon derives `not-foreground` when there is no open
collections stream for the addressed `session_actor`, or `expired` otherwise. Phone clocks and
client timestamps do not extend that deadline. A late report cannot replace the terminal decision;
repeated reports cannot overwrite an already accepted outcome.

Commands live only in memory on the phone pairing's issuing member. They are never claims,
replicated records, persistent receipts, event replay, or restart recovery. Terminal outcomes remain
available for five minutes of ephemeral correlation before eviction. The process retains at most
2048 rows in total and refuses further admission when retention capacity is exhausted; it does not
discard a retained outcome early to make room. A daemon restart or eviction makes outcomes
unavailable. Expired commands are never executed on reconnection. The normal envelope's snapshot
describes the response context, not durable storage of the command.

## Isolated client-control v1 signed results

The generated client-control contract uses the same `phone.inspect` and
`phone.navigate` verbs on iOS, web and TUI. `phone.message.read` is excluded from
test grants. A granted request adds `grant_id`; its command carries `grant_id`,
`run_id` and `control_grant` (including the pairing `session` actor). Ordinary
ungranted reports retain the preceding wire shape.

A granted `DeviceResult` adds `binding: ClientResultBinding` and the existing
`signature: DeviceSignature` envelope. The binding contains `purpose`,
`command_kind`, `device`, `session`, `grant_id`, `run_id`, `command_id`,
`outcome`, nullable `error_code`, and `visible_state`. Outer command, outcome and
error must agree with the binding; any outer `result` must equal `visible_state`.
The closed visible-state type has `route`, `screen`, `visible_item_ids` and
nullable `opened_item: {id, title, body}`. Arrays retain display order.

`visible_state_sha256` is lowercase SHA-256 of UTF-8 RFC 8785 JCS JSON. The
closed type admits only strings, null, arrays and fixed ASCII object keys:
there is no floating-point normalization or arbitrary-key sorting ambiguity.
No Unicode normalization is applied, lone surrogates are invalid, and slash,
non-ASCII and U+2028/U+2029 are not escaped.

Sign UTF-8 newline-joined lines below, **without a trailing newline**. Every
right-hand value is its compact JSON encoding, including quotes for strings.
The purpose is the first signed field, and the field order is normative:

```text
smallclaims-claim-fields-v1
purpose="client.device-result.v1"
command_kind="phone.inspect"
device="device/example"
session="client-session/example"
grant_id="client-control-grant/example"
run_id="mission-run/example"
command_id="device-command/example"
outcome="completed"
error_code=null
visible_state_sha256="<lowercase hex digest>"
```

Use P-256 ECDSA with SHA-256; signature encoding is 64-byte IEEE P1363 `r||s`,
base64url without padding, not ASN.1 DER. The envelope format is `fields-v1` and
`signed_fields` is exactly the ordered list above. The enrolled pairing key is
the authority, never a caller-selected envelope key. Envelope `nonce` and
`signed_at_unix_ms` are not signed or freshness authority: daemon time must be
within both command and grant windows, and a command accepts only one result.
This signature is purpose-specific and never authorizes `message.send` or a
read-state mutation; fixture pairings must not acquire `control.messages`.

The source templates live in `crates/fractal-client-codegen/templates`; regenerate
all outputs with `cargo run -p fractal-client-codegen`. Rust exports canonical
helpers through `fractal_schema::client_control` and `fractal_client`; TypeScript
uses `ClientControl.generated.ts`, and Swift uses `ClientControl.generated.swift`.
All three test suites read the same
`fixtures/clients/client-control-v1.golden.json`, including fixed-public-test-key
signatures and negative bindings for grant/run/command/purpose/kind/device/session,
outcome, error and visible-state tampering. The fixture scalar and nonce are public
test data, never enrollment keys.

## Fixture enrollment and revocation

Enrollment is a privileged local Unix-socket operation on the local-only
`client-control-fixture` instance owned by `person/client-control-fixture`.
The socket peer must have the daemon's UID. A remote bearer credential,
request header, ordinary pairing, or main-instance operator cannot enroll.
Each isolated client first generates its own P-256 key and uses the existing
pairing flow. Enrollment binds the exact existing pairing subject, device and
public key; it never generates keys or grants principal signing authority.
Pairing scopes must include `read.projections` and `control.commands`, with
only `read.*` scopes otherwise; `control.messages` is forbidden.

The enrollment request records `device`, `pairing_subject`, `public_key`,
`kind` (`ios`, `web`, `tui`), `host`, `instance`, `consent_ref` and
`packet_digest`. The instance is exactly `client-control-fixture`.
`consent_ref` must name the native ask `step-run/ask-…/ask`; `packet_digest`
is the lowercase 64-hex SHA256 of the reviewed enrollment packet and input
manifest. These become `consentRef` and `packetDigest` in the persisted
`custom.client.test-enrolled` claim alongside the bound host, instance and kind.
Invalid ask IDs return `consent-ref-invalid`; malformed digests return
`consent-packet-invalid`; reuse across devices, kinds or re-enrollment returns
`consent-ref-reused`. Re-enrollment or changed scope requires a fresh ask.

Approval verification is an external mission gate: before the local call,
verify the native ask was answered by `person/schickling` with explicit approval
of the matching packet digest, kind, host and instance. Silence is not approval.
The privileged operator attests that verified gate; the fixture daemon never
connects to the main daemon or imports its person/ask claims. A signed exported
consent receipt is not part of this protocol.

The exact paired client's revoke control, a local agent, or the daemon-host
operator appends `custom.client.test-revoked` naming the enrollment claim ID.
This persisted withdrawal ends every grant bound to that enrollment, regardless
of timestamps; new enrollment cannot resurrect old grants. Grant issue, command
admission and result acceptance reload enrollment and pairing authority.
Operators can revoke even after the pairing expires.

## Grant lifecycle on the commands collection

The existing session-scoped `commands` subscription carries generated
`client-control-grant-lifecycle` resources alongside device commands, using the
same snapshot/changes frames and typed `Resource` decoder. Each lifecycle row
has the ordinary resource header plus `grant_id`, optional `item_id`, `run_id`,
`device`, `session`, `state` (`started`, `ended`, `revoked`), `reason` and a
monotonic integer `sequence` in `1...9007199254740991`. All client decoders
reject zero or out-of-range sequences before lifecycle ordering checks; the
shared zero-sequence trace expects `refuse-invalid-frame`, not a stale start.

These rows drive the visible indicator and termination only. They do not carry
verbs, screens or budget policy; policy remains in each
`DeviceCommand.control_grant`. A command without an active, nonterminal
lifecycle grant for its `grant_id` must produce no navigation or signature.
Attention grants require an `item_id` naming an existing native ask in this
instance; missing or unknown items are `screen-not-granted`. NowRoot-only grants
do not require an item. Every lifecycle row preserves the bound item exactly.
Clients ignore started rows whose sequence is not greater than the last
observed sequence for the grant, and treat ended/revoked as permanently final
for that grant ID: a later started row cannot reactivate it.
The native-change watcher is fixture-scale only: each change reconciles grants
under the shared admission lock and reloads mission-run authority.

`fixtures/clients/client-control-lifecycle-v1.json` is shared decoder and
acceptance-trace data for all clients, covering each state, stale/reordered
started rows, missing/wrong item binding, command-before-start and
command-after-terminal. Context-dependent lifecycle checks belong to the
client handler; decoding a resource never grants authority.

TypeScript canonical hashing supports Hermes without WebCrypto through an
injected `ClientControlSha256 = (bytes: Uint8Array) => Promise<Uint8Array>`.
Pass it as the second argument to `visibleStateDigest` or
`clientResultCanonicalBytes`; omitting it uses generated
`defaultWebCryptoSha256`. Canonical encoding remains generated and unchanged;
the injected implementation returns the raw 32 SHA-256 bytes, not hex or text.
Invalid Unicode is refused with the exported TypeScript
`ClientResultCanonicalizationError` and its `code: "invalid-unicode"`, including
through `clientResultCanonicalBytes`; consumers must not match error messages.
