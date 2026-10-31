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
| `phone.message.read` | `{attention_id: "attention/…"}` | `control.messages` |

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
changes phone-local UI only. Message-read addresses the phone's held attention card and must use
its existing mark-read handler, scope checks and action fence/idempotency. Issuance does not grant
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
