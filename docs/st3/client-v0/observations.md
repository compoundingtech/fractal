# Phone observations

`observations.report` posts `ObservationReport` to `POST /v1/client/observations`.
Any current authenticated paired session may report, including a read-only pairing.
Unpaired local Unix sessions, anonymous callers, expired and revoked pairings are refused.
Reports become immutable `custom.client.observation-minute` claims in the replicated graph.
The phone records and durably queues these samples in `apps/ios/observations.ts`.
The existing command and wire shape are unchanged.

The generated methods are Rust `observations_report`, Swift `observationsReport` and
TypeScript `observationsReport`. The response is `ObservationResponse`:
`{api_version:"st3.client.v0",request_id:"request/…",value:{accepted:true}}`.
It has no snapshot, fence or action receipt in the response. Acceptance means all minute
claims and their native admission receipts committed together; it does not mean every peer
has already replicated them. Retry a lost response with the identical payload and UUID,
including after restart or on another member that has received the claims.

A report has a canonical lowercase nonzero UUID `report_id` and 1–32 samples.
Each sample uses one of two discriminators:

```json
{
  "report_id": "019a1234-0000-7000-8000-000000000001",
  "samples": [
    {
      "kind": "latency", "target": "ios-connect", "carrier": "lan",
      "interval_start": "2026-10-10T15:00:00Z",
      "interval_end": "2026-10-10T15:01:00Z",
      "count": 100, "over_target": 1, "max_ms": 2000,
      "buckets": [[103, 99], [2047, 1]]
    },
    {
      "kind": "live-share", "target": "ios-live-share", "carrier": "lan",
      "interval_start": "2026-10-10T15:00:00Z",
      "interval_end": "2026-10-10T15:01:00Z",
      "foreground_ms": 10000, "live_ms": 9900
    }
  ]
}
```

Carrier is `fabric`, `tailscale` or `lan`; optional `path` is `direct` or `relay`.
Intervals must be exactly one closed minute with minute-aligned UTC `Z` timestamps.
The oldest permitted start is seven days before admission; end may be at most 120 seconds
in the future for clock skew. Future-skewed intervals enter live evaluation only after closing.
Reports must contain disjoint intervals, never cumulative summaries. One report can batch disjoint minutes of the same population, with at
most one sample per target/carrier/path/minute. Unknown fields, targets or enum values are refused;
there are no content, addresses, node identities or credential fields.

Latency count is 1–1,000,000, `over_target` is 0–count and max is 0–3,600,000 integer ms.
A sparse histogram contains 1–64 sorted unique `[inclusive_upper_ms,count]` pairs with
positive counts summing exactly to count. Bins are exact for 0–15 ms; above that they use
16 subdivisions per doubling, the same bucket arithmetic as daemon `Series` but applied
in integer milliseconds rather than microseconds. For `v >= 16`, let `e=floor(log2(v))`
and `step=2^(e-4)`; its bin upper bound is `(floor(v/step)+1)*step-1`.
For example 100 ms maps to 103 and 2000 maps to 2047. The last occupied bin must contain
`max_ms`. Percentiles use bin upper bounds clamped to the exact maximum. Exact over-target
count must agree with the minimum/maximum count allowed by bins that straddle the target,
and with the exact maximum. Work scales with reported buckets, never histogram counts.

Live share has no histogram, max or latency count. Foreground is 1–60,000 ms and live is
0–foreground. Foreground milliseconds form the denominator; disconnected foreground
(`foreground_ms - live_ms`) forms over-target. It aims for at least 99% live foreground time.
Neither background nor unreported/disconnected phone time is inferred.

The six latency objectives in `slo/objectives.kdl` are `ios-open-to-live` (3000 ms),
`ios-connect` (1500), `ios-message-ack` (500), `ios-conversation-open` (1000),
`ios-terminal-open` (1500) and `ios-recover` (10000). `ios-live-share` is a separate share
objective at 99%. These remain client-observed populations, separate from daemon response
envelopes. Native objective rows and doctor lines fold the replicated summary claims rather
than process-memory admission. Unreported minutes never imply successful phone operation.

Live 1m/5m/1h windows include only whole reported intervals wholly inside the sliding UTC
window. Boundary-straddling intervals are excluded and counted; minute resolution and
`complete_coverage:false` are explicit. The 1m window may therefore have no eligible
closed minute between minute boundaries. `reported_interval_ms` counts minute slots with
reported data, not phone uptime or complete fleet coverage. Delayed reports are placed at
original observation time. Older-than-hour reports are history only. An empty population
is unknown coverage, and doctor presents it as info. These windows do not establish a
full-day SLO. Share rows contain foreground/live milliseconds and live share, never p99.

Each immutable minute is keyed by stable source, target, carrier, path and UTC minute.
The source is derived from the authenticated pairing's person and device public-key identity,
not the receiving member, its state directory or the destination-specific grant. The exact
pairing grant is retained separately as admission provenance. Re-pairing the same phone and
person to another destination therefore preserves report/minute identity. A pairing without
a stable device identity must be renewed before reporting.

The phone persists each report's UUID and exact payload until acknowledgement. An exact
source/report retry appends no graph claims, including after store reopen, replication or a new
destination pairing. Reusing a report ID with changed content or reporting the same minute
under a different report ID returns HTTP 409. All samples validate and commit atomically.
Conflicting minute claims admitted concurrently on disconnected members are excluded from the
native population after replication rather than selecting a winner or counting both.
Equal concurrent claims collapse to one minute. Admission requires a current grant;
already accepted evidence remains readable after that grant expires or is revoked.
Only minutes with valid native phone-admission receipts occupy report replay, minute replay
or once-only cardinality keys. Merely replicated claims, including signed claims without
matching phone/person provenance, cannot block authentic admission or enter native evidence.

The graph is the only authority for native objective rows and doctor lines. Native reads seek
a numeric minute-time index for the last hour; report replay seeks the source/report index
and minute replay seeks the subject. Folds retain 60 minute slots per target and merge exact
foreground/disconnected counts and canonical sparse latency histograms. The one-billion
aggregate slot bound fails closed, never truncating evidence or presenting overflow as good.

## Frozen member-local diagnostic compatibility

The existing `GET /v1/client/request-latency` diagnostic remains under the client-v0 freeze.
Its original process-local phone window accumulator, bounded Admission cache and rotating
`state-dir/client-observations.jsonl` history remain unchanged. The endpoint commits durable
claims first, then runs that legacy diagnostic admission before acknowledging the report.
Those local files and counters are never read by native objective or doctor evaluation.
The legacy diagnostic can still be empty after restart or on another receiving member; that
does not erase durable native evidence. Its cache scope uses the old local directory/grant
key only for diagnostic compatibility, never for durable source identity.
Its grant-specific local deduplication can still refuse the same UUID after a grant change
on the same process even when durable graph admission correctly recognizes an exact retry.

Removing this legacy diagnostic path and switching its held reader to the graph requires a
separate client-v0 freeze exemption. Legacy JSONL is not imported as fleet evidence because
it lacks stable authenticated source provenance. Missing historical minutes remain unknown;
neither local history nor delayed phone reports are re-timestamped as current phone time.
