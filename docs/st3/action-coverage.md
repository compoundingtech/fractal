# Fractal action coverage

The inventory covers every offered CLI command, every typed client action, every terminal UI daemon effect, and the palette and local controls. It distinguishes real CLI/Unix transport tests, the live terminal UI adapter, local reducer tests, and shared implementations. A test reference describes its actual layer; a shared implementation test alone is not a claim that the entire interactive UI was driven.

[action-coverage.json](action-coverage.json) is the canonical inventory of commands,
actions, controls, their behavior and test references. Read its `cli`, `typed_actions`,
`client_commands`, `stui_effects`, `palette_actions` and `local_ui_actions` sections
for the current rows; this page does not duplicate those tables or their counts.

The [inventory guard](../../crates/fractal/tests/action_coverage.rs) compares typed
actions with the generated client contract and real dispatch builders, UI effects
and palette actions with their declarations, and all listed test references with
source files. The [CLI guard](../../crates/fractal/src/main_tests.rs) compares the
CLI rows with Clap's offered command tree. Hidden driver/replication worker commands
and help aliases are internal or alternate spellings, rather than additional
person or agent actions.

## Restart and stale-state model

The new integration fixture owns a private disk store, Unix socket, configuration, HOME/XDG directories, native transcripts and PTYs. A restart ends the listener/router, reopens the store, and constructs fresh daemon state, notification channels and action gates. It never starts or changes the user's daemon. Readiness requires a successful public capabilities response; a socket path alone is insufficient. CI retries are disabled for these action coverage tests. Runtime restart tests acknowledge signal delivery, then wait with a bounded timeout for the PTY to publish process exit. The fleet fixture runs real `st up` and replication workers in private directories and uses `--no-service` for administrative operations. Service tests use a private manager shim which launches real daemon processes and records only fixture service changes.

For the restart-matrix typed actions, the test first submits a stale applicable fence and checks that the store index does not change. It then restarts, submits the original valid fence, checks the domain result, restarts again, and replays the exact request. The replay must return the original affected IDs without duplicate domain writes or receipts. Execution-specific cases also check old runtime incarnations, stale terminal sequences, old native discovery revisions, obsolete desired revisions, changed review cards, and replaced proposal preview tokens.

The CLI drives real public command paths. Reads are compared before and after restart; writes are checked in the reopened graph. Messaging checks explicit idempotency keys, replies, attachments and lifecycle receipts. Review tests use current source cards and inspect the single attributed verdict. Launch tests ask and answer questions, submit/compare variants, preview, approve, request revision, cancel, start approved definitions, and propose a variant against one run generation. Terminal tests send real input, keys and signals to a private PTY, inspect its screen, and reject revoked attachments. Fleet tests cover creation/join/history, mode changes, revoke/remove/leave, migrations, checkpoints and repairs. The GitHub watch/unwatch/list/comment/own test drives real CLI and daemon processes against a private HTTP fixture, including restart, repeated unwatch, comment and review posting, foreign-author refusal and idempotent ownership registration.

The live terminal UI test drives send/attachment upload, discussion, retry, agent/shell/launch creation, launch revision, device revocation and mission cancellation through its real adapter across disk-store restarts. The gate fixture drives the real attention adapter after a changed card and a restart. Terminal key encoding, native attachment/reconnection, visibility/read receipts, palette/layout and local draft/navigation controls retain their own focused tests.

Suspension and new-seat readiness tests supply explicit fixture owner acknowledgements. They verify the public request, wait, native identity and durable protocol phases; native process continuation belongs to the runtime canaries and restart suite. GitHub gates use real CLI commands with a private HTTP fixture for pass/wait/broken behavior, obsolete commit refs and malformed-input refusal across daemon restarts; they make no live GitHub API calls. GitHub comment posting offers no public snapshot fence or retry token: tests prove refusal of a foreign seat actor before HTTP posting and durable ownership across restarts; they do not claim exactly-once external posting. Local-only UI operations have no graph fence. Glass base revisions intentionally report last-writer-wins replacement and are not execution fences.

Three declared typed actions are unavailable: `attention.resolve` returns `attention-migrated`; `mission.revise` and `work.publish-mission` return `unsupported-capability`. Tests verify refusal without writes on either side of a restart. Supported source actions and the legacy CLI publication/revision paths have separate successful coverage. Dismiss/remind-me-later and forgetting an unconfirmed draft are local presentation changes, not attention resolution or message archival.

## Run the checks

Run the per-PR checks through the repository's normal test and signing runner,
from the committed head you intend to push:

```sh
scripts/fractal-test --all --push origin
```

This runs the checks in a clean worktree with isolated test state and publishes
the signed results. See [gate evidence](gate-evidence.md) for the runner and host
requirements. The inventory and CLI guards above, the integration action-coverage
suite, fleet restart controls and terminal UI tests remain part of their owning
packages' test suites.

Real-process fleet tests require a process ancestry without an inherited Fractal agent identity. Use an isolated test runner when invoking them from a Fractal seat; do not remove the daemon's authority checks. All actors and machines in these tests are invented fixtures.

## Typed client actions

The accepted/refused result and replay invariants above apply to the restart-matrix rows. `arrangement.edit` instead references focused real API-handler and store regression tests: stale layout fences intentionally merge; fenced retirement compares the current revision and refuses concurrent edits without writes, while unfenced retirement remains unconditional; stale authority and launch fences refuse without edits; receipts replay and recover a dispatch gap without accepting changed action identity, including fenced retirement; agent attribution retains person ownership; and projected heads survive store reopen, upgrade backfill and replication. These tests do not claim full CLI/Unix transport restart coverage.

Conditional-retirement evidence: `arrangement_retire_fences_refuse_concurrent_edits_and_replay_receipts`
and `arrangement_fenced_retire_recovers_receipt_gap` in
[`crates/fractal-api-client-v0/src/api/client_v0/arrangements.rs`](https://github.com/compoundingtech/fractal/blob/main/crates/fractal-api-client-v0/src/api/client_v0/arrangements.rs).

Create/folder races return typed non-retryable refusals rather than internal errors:
`arrangement_create_race_returns_typed_exists_and_exact_retry_receipt` and
`arrangement_folder_race_and_validation_refusals_stay_typed` exercise the actual
HTTP error envelope, rejected-write atomicity and receipt replay.
`selected_arrangement_survives_byte_window_edits_and_retirement` verifies the selected
stream window stays outside owner-prefix truncation, rejects wrong owners, follows
edits and removes retired subjects without changing unfiltered reads.

## Diagnostic client commands

`client_commands` records `observations.report` separately from fenced graph actions.
The real paired HTTP router control in `api/client_observations.rs` covers read-only pairing
authorization, unpaired/revoked refusal, identical retry, conflicting reuse, body limit,
unchanged graph index and reader/doctor population. Admission/history controls cover
atomic validation, overlap, filesystem failure, rotation, expiry and retained files across
fresh process-memory admission state. This is local diagnostic acceptance, without a
replicated acknowledgement or a full-day live-phone SLO claim.

The sidebar's existing optional `resources.list` read is covered by
`observed_resources::optional_resource_read_lists_typed_prs_repositories_and_other_subjects_over_unix`
(real Unix socket and arrangement filing), the daemon filter/fence and read-scope tests, and
`sidebar_graph_resources_open_tabs_and_finder_selection_only_previews_filing` (test backend navigation and explicit filing).

The optional `inventory-collections` capability follows clients, usage, machines,
devices, native sessions and terminal lists on the collection socket. The
`collection_refresh_tests::*_inventory_*` real-socket tests cover source changes,
frame schema and typed decoding, reconnect snapshots, invalid subscription
filters/limits, active-device retirement and native filesystem/process events.

The phone uses the same `collections.inventory` reads, covered by the real-socket
tests above, while the screen displaying each inventory is focused. Its Node
`inventoryFeed.test.mjs` checks the client layer: shared slots, selected usage
periods, ordered updates, explicit refresh snapshots, reconnect, background and
grace cleanup, and 72 seconds of protocol-heartbeat idle with no request.
`collectionSocket.test.mjs` checks authentication and control-event delivery at
the native bridge boundary; it does not claim an iOS simulator was driven.

The Fractal TUI consumer is covered by `crates/fractal-e2e/tests/feed/inventory.rs` over an isolated
Unix daemon: all six windows, typed client/usage adaptation, period replacement,
unsubscription and authoritative reconnect snapshots.
`crates/fractal/tests/client_only.rs::the_fleet_inventory_screen_sends_no_request_after_its_first_frames_for_more_than_a_minute`
and `…::the_usage_inventory_screen_sends_no_request_after_its_first_frames_for_more_than_a_minute`
drive the actual binary with `scripts/fractal-ui-test-client` and `STUI_TIMING_LOG`.
Fleet and Usage each remain idle for 72 seconds; their first snapshots must occur
once, every inventory HTTP read is forbidden, and no request follows ready frames
for longer than the old 60-second interval.

The usage inventory empty-graph socket test verifies that an open period uses the same wall-clock end as the HTTP read, then reconnects without a graph write.
`st ui` opens the terminal interface; plain `st` does so only with terminal input and output outside an agent seat. The typed-key and paired-device gates in `crates/fractal/tests` exercise this entry point. Pair with `st devices complete`. `st ui open` asks that interface to show an agent, mission or document; `crates/fractal/tests/ui_open.rs` drives it from a seat process against the real interface under a PTY. `st demo play` plays a scripted demo in that interface, in its own instance; `crates/fractal/tests/demo_play.rs` plays one under a PTY in a throwaway home.
