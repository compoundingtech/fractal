# Local GitHub observer admission

The response meter previously counted only completed resource-provider HTTP
responses in memory. An interrupted or failed send and a daemon restart could
lose usage. This change reserves each configured daemon GitHub HTTP send before
the local reqwest or authorized-RPC path runs, persists the reservation in
`STATE/github-admission.json`, and reports it separately in
`doctor` as `github-requests/admitted/CALLER/RESOURCE`.

GLOBAL-VIEW: no. This machine never consults another machine's state, takes a
distributed lock or assumes it has reserved capacity in a shared token bucket.
There is still one local repository observer and its existing per-URL conditional
REST cache. A per-process mutex serializes only this daemon's local journal. The
existing state directory and daemon singleton own the file; no second poller or
new token/credential/grant/setting is introduced.

An observer's caller is the existing `spend_as` task-local subject, assigned by
the reconciler. Neither ST_AGENT, URL nor an HTTP header selects it. Observers
share a1000/hour aggregate ceiling for each request resource and each caller has
the same per-resource ceiling. Changing observers cannot obtain another1000
allocation. Core, GraphQL and REST search are separate; these are request counts,
not a measurement of GraphQL points or a promise about the provider's separate
search/secondary limits. Admission retains failed sends and304 reservations
conservatively. A refusal appends no reservation and returns the existing typed
ProviderRateLimit so reconciliation records the actual next eligible time.

The existing REST reader retains ETags and sends If-None-Match on revalidation,
reusing complete bodies and pagination on304. First reads and endpoints that do
not return an ETag cannot send a valid conditional header. GraphQL does not offer
ETag revalidation: the shared repository check interval is now at least5minutes
rather than60seconds; settled repositories remain15minutes. Explicit refresh is
still an explicit refresh. Existing shared-URL and conditional-read controls are
retained. No new per-agent polling is added.

The journal is bounded, written to an exclusive temporary file, fsynced and
atomically renamed, then its parent fsynced before sending. Its read rejects
symlinks/nonregular files, invalid records and future timestamps rather than
resetting usage. A directory-sync failure invalidates the memory copy so the next
attempt rereads any reservation that the rename already committed. A corrupt
journal remains a visible doctor gap; no empty fallback erases prior usage.
Doctor loads prior-hour reservations at configuration, before the first new send.

Scope still missing: unscoped daemon commands (including merge gates and comment
writes) do not yet carry an enrolled per-caller identity. They are recorded as
unattributed and are **not throttled by the observer allowance**. A meter failure
on that path emits a visible gap and preserves merge availability. This is not
per-agent write admission or authenticated merge-path enrollment. Requests made
outside the configured daemon send path, standalone gh, binaries not yet installed
and protected CI-service signing are separate. No installed/running qualification
is inferred from this source.

Validation: five new Rust controls cover durable restart/expiry, aggregate-vs-caller
limits, separate GraphQL requests, refusing without spending, preserving usage
without a response, corrupt/future state and symlink refusal. They are WRITTEN,
NOT RUN locally: this host is not authorized for Cargo/native qualification.
`rustfmt` parsed the new module and `git diff --check` passed. Existing resource
controls cover shared listing reads and200/304/head-change ETag behavior. Hosted
qualification and exact-head Intake review are required before a GO.
