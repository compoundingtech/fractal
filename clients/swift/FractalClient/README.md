# FractalClient

Swift 6 package for iOS 17/macOS 14 clients of `st3.client.v0`. It is an online thin client over an
authenticated Fabric-loopback URL and includes typed capabilities, resources, timeline/events,
actions, pairing, and styled terminal screens; `terminalStream` yields each changed screen.

```swift
let client = FractalClient(fabricLoopbackURL: gatewayURL, credential: credential)
let capabilities = try await client.capabilities()
let work = try await client.workList(limit: 100)
```

After a `cursor-gap`, discard projection caches, fetch fresh first pages, and resume from the new
capabilities `eventCursor`. Never queue mutations offline. Construct actions with the snapshot and
resource fences the user actually viewed. App code uses generated named read methods and typed
action methods; collection strings, raw paths, and untyped action dictionaries are private.

## Trace context

An iOS app can connect its tracing adapter to the client with an optional synchronous,
`@Sendable` callback. Have the adapter return the **currently active span's** W3C
`traceparent` and optional `tracestate`, or `nil` when no span is active. For example,
accept the app's active-span lookup when constructing the shared client:

```swift
func makeClient(
    gatewayURL: URL,
    credential: String,
    activeSpanContext: @escaping @Sendable () -> TraceContext?
) -> FractalClient {
    FractalClient(
        fabricLoopbackURL: gatewayURL,
        credential: credential,
        traceContext: activeSpanContext
    )
}
```

The callback runs synchronously on the client actor, once for every HTTP request and
every WebSocket open (conversation, glasses, arrangements, and terminal streams), not
once per client or per stream message. Its captured tracing adapter must be safe to
access there, rather than requiring the main actor. The package has no tracing-library
dependency and does not create spans itself.

No callback, or a `nil` result, sends neither trace header. `TraceContext` is a public
`Sendable`, `Equatable` value with `traceparent: String` and `tracestate: String?`.
Before sending it, the client requires exactly the lowercase W3C shape
`^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$`, rejects version `ff`, and rejects
all-zero trace or span IDs. An invalid `traceparent` suppresses **both** headers.
For a valid parent, `tracestate` is forwarded verbatim, including existing `st`
entries: the SDK never adds or rewrites `st=c`. A daemon with trace propagation
records `st.parent.sampled` for a sampled incoming parent without an `st` tracestate
entry, so the collector can honour external app sampling decisions. Daemons without
trace propagation ignore both headers.

## Committed scenarios

`ScenarioReader` loads the shared `fixtures/scenarios/index.json` world picker and slice files
directly from a local directory. It does not connect to a gateway or implement replay scheduling.
Supply the repository fixture directory (or a copy bundled with your app):

```swift
let scenarios = ScenarioReader(directory: fixturesURL)
let worlds = try scenarios.loadIndex().worlds
let roster = try scenarios.loadSlice(
    world: "fleet-mid-refactor",
    slice: "roster",
    now: Date(timeIntervalSince1970: 1_893_499_200)
)
let state = try roster.decodeState(ScenarioRosterState.self)
let sessions = state.sessions
let devices = state.devices
```

`loadSlice(at:nowMilliseconds:)` accepts an exact integer epoch-millisecond clock; the `Date`
overloads default to load time and truncate finer clock precision. Pin the clock to the index's
anchor for stable snapshots. `ScenarioReader.rebase(_:times:anchor:nowMilliseconds:)` is also
available for in-memory JSON values.

Only the JSON pointers in `times` move by `now − anchor`, including pointers inside timeline
payloads. Timestamp inputs accept RFC 3339 offsets, lowercase `t`/`z`, and arbitrary fractional
precision, truncate to milliseconds **before** shifting, and emit exactly three fractional digits in UTC. Integer
`epoch-ms` values shift exactly. Range errors are `ScenarioError.range(pointer:)` and name the
affected location. Content, ids, durations, timeline `at_ms`, cast offsets, and diagnostic
`unknown.known_value` witnesses remain untouched.

`ScenarioSlice.state` and `.timeline` retain their kit envelopes. `ScenarioRosterState` and
`ScenarioDetailsState` are Codable envelopes with required, typed session/device and
launch/launch-variant inventories, including explicitly empty inventories. Both preserve optional
`resources`; roster also preserves `recorded_history` as original JSON changes envelopes.
Other envelopes can use `decodeState` and `decodeTimeline` with small structs containing the real wire models.
For conversation thread `items`, use `ScenarioConversationEntry`: `.known(TimelineEntry)` for
recognized types and `.unknown(JSONValue)` for genuinely future types. Recognized-but-malformed
entries throw; future entries remain opaque rather than being replaced by diagnostic witnesses.
Resource collections can use the existing `Resource` union, including its `.unknown` case.
Native readers expose only committed conversation history; they do not synthesize older seeded
pages from the optional thread `history` descriptor.

`ScenariosTests` runs every shared `_vectors/rebase.json` case and recursively discovers every
committed slice JSON file, checks index coverage, and decodes state and timeline wire payloads.
New slice/event shapes and undeclared future resource/entry kinds fail instead of being skipped.
Strict slices additionally validate their complete state/timeline root once, checking wire
object keys and known enum cases against the committed client schema. This includes standalone
timeline, server-failure and resync error codes, valid RFC 3339 timestamp instants, safe-integer
duration bounds, and the year-9999 upper bound for epoch milliseconds. Composed presentation
fields remain valid; no decoded-type switch can omit a field.
Permissive Codable decoding alone is not strict validation. Deliberately tolerant slices
continue to decode their original contaminated values.
Run `swift test --package-path clients/swift/FractalClient --filter ScenariosTests` from the
repository root on macOS with Swift 6 (macOS 14 or newer).

## Generated artifacts

The generated models and complete typed operation surfaces are refreshed from the normative schema
and machine manifest with `cargo run -p fractal-client-codegen`; `--check` renders every artifact in
memory and byte-compares it to the checked-in output.
