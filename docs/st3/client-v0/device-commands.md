# Foreground device commands

A device command is one typed, expiring request to a paired client's foreground instance.
It is not a background job, push notification, or replay queue. The same protocol addresses
enrolled iOS, web and TUI test clients; ordinary person-operated phone commands retain their wire shape.

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
`device-command` rows and test-client grant lifecycle resources. There are no new frame kinds
and no second socket. The addressed session is the delivery boundary; subscriptions cannot
override it. Client command windows contain only active `pending` / `delivered` commands;
terminal transitions remove them, never replay them.
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

Command correlation rows live only in memory on the pairing's issuing member; they are not
replicated or replayed after restart. Granted commands and fixture admission refusals have
separate durable receipt claims described below. Terminal correlation outcomes remain
available for five minutes before eviction. The process retains at most 2048 rows and refuses
further admission when capacity is exhausted rather than discarding retained outcomes early.
A restart or eviction removes command correlation, not persisted receipts. Expired commands
are never executed on reconnection. The normal envelope's snapshot describes response context.

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

## Grant admission and durable receipts

`POST /v1/client/control-grants` binds a grant to the issuing agent's currently
claimed step of a running mission run. The exact claim generation, attempt and
incarnation remain authoritative: claim replacement, claim/run end, enrollment
revocation and deadlines end the grant. The default budget is 50 commands, spent
durably before admission; grants last at most two hours. Only `phone.inspect`
and `phone.navigate` to `NowRoot` or `Attention` can be granted. The journey opens
the seeded native ask using `Attention {params: {id}}`, then inspects it. Optional
grant `item_id` binds that screen to the seed; another ask is `screen-not-granted`.
`Conversation` is always `screen-not-granted`. Answer, close, read-state and
`message.send` verbs are `verb-not-granted`; no grant authorizes a mutation.

The existing session-scoped `commands` collection includes generated
`kind: "client-control-grant-lifecycle"` resources before the first command.
Their states are `started`, `ended` and `revoked`; the stable resource ID and
strictly increasing sequence identify the latest durable transition per grant.
Idle subscribers receive revocation and run/claim/deadline termination without
issuing a command or resubscribing. Lifecycle drives only the control indicator
and termination: policy remains in each command's `control_grant` record.
Commands without an active matching lifecycle grant fail closed. Clients ignore
duplicate/out-of-order starts, preserve the grant's `item_id` binding, and never
reactivate a terminal grant ID. A command binds `grant_id` and `run_id`; no second
command transport exists.

Granted result acceptance verifies the generated canonical bytes directly with
the enrolled P-256 key, not principal message authority. The outer outcome,
error and optional result must agree with the signed binding. The daemon checks
the command and grant windows, pairing session, run, command, verb and visible
screen again, and rejects terminal-command replay. Signature-envelope nonce and
timestamp metadata do not establish freshness; the daemon's clock does.

Every terminal granted command has one durable
`custom.client.command-receipt` claim. The resource's `receipt_claim_id` identifies
it. Completed signed results record `verifiedResultSignature` and `signedResult`;
unsigned terminal states such as expiry and grant-end refusal record nulls.
Command correlation rows themselves remain ephemeral.

An authenticated agent's well-formed command refused for a D-3/D-4 reason also
receives one durable rejected-attempt receipt, but only in the local-only fixture
owned by `person/client-control-fixture`. It records an assigned `commandId`,
issuer, device, optional grant/run, verb/screen, typed reason, terminal state
`refused` and no signature. The typed error details include `command_id` and
`receipt_claim_id`. The existing 2,048-row, five-minute command/refusal retention
bound limits receipt admission; saturation returns `receipt-capacity` without a
receipt. Persisted receipt claims are not removed when this window expires.
Unauthenticated or malformed requests, and requests outside the fixture, return
typed errors without receipt writes.
