# Fractal crates: what is where and who may use what

Twenty packages carry what used to be the single `st3` package and its end-to-end tests:

| package | path | holds |
|---|---|---|
| `fractal-kernel` | `crates/fractal-kernel` | the library's leaf modules, which name no store, api or reconcile code: `accounts`, `archive`, `boot`, `cli_output`, `config`, `conversation_header`, `conversation_search`, `disk`, `evidence`, `fleet`, `gate_kinds`, `gate_report`, `github_admission`, `harness_events`, `hooks`, `instance`, `lane`, `memory`, `model`, `native_views`, `pricing`, `projection`, `publication`, `read_cache`, `rules`, `seat_queue`, `skill`, `terminal_binding`. fractal-daemon re-exports each one at its old path (`pub use fractal_kernel::model;`), so `crate::model` and `fractal_daemon::model` read the same. <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 --> R11 allows the dependency on `fractal-client` only for `fractal_client::product_dirs`: the standalone client owns the single resolver for the R11 state-directory migration. <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal --> |
| `fractal-telemetry` | `crates/fractal-telemetry` | the library's telemetry modules, which name no store, api, client or reconcile code: `incremental`, `otel`, `otel_client_class`, `otlp`, `reconcile_telemetry`, `recorder`, `recorder_report`, `relay_trace`, `slo`, `slo_history`, `startup_telemetry`, `telemetry`. It depends on `fractal-kernel` and `smallclaims`. fractal-daemon re-exports each one at its old path. What would name the store or the API is a trait the daemon implements (`incremental::ChangeSource`, `otlp::ObservationSource`) or a value the caller passes (`telemetry::hook`'s submit, `otlp::run_slo`'s windows reader). `recorder_receipts` writes claims through the store and stays in fractal-daemon; the unit tests that need the store, the graph or the API live in fractal-daemon's `telemetry_tests`. <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 --> R11 allows the dependency on `fractal-client` only for `fractal_client::product_dirs`: the standalone client owns the single resolver for the R11 state-directory migration. <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal --> |
| `fractal-secrets` | `crates/fractal-secrets` | the modules the store and everything above it use that name no store code: `client` (the daemon client), `startup` (startup status), `tile_registry` and `sekrets` (the credential gateway). It depends on `fractal-kernel` and `fractal-telemetry`; `fractal-store` re-exports it at its root |
| `fractal-github-base` | `crates/fractal-github-base` | the GitHub leaf modules, which name no store code: `environment` (login-shell capture), `github_http` (authenticated transport), `resource` (repository resources), `github_watch`, `checkout` and `silent`. It depends on `fractal-secrets`; `fractal-store` re-exports it at its root |
| `fractal-missions-base` | `crates/fractal-missions-base` | the mission-declaration leaf modules, which name no store code: `declaration_syntax`, `gate_parse`, `provenance` and `mission`. It depends on `fractal-secrets`; `fractal-store` re-exports it at its root. The tests that parse intents through the graph live in fractal-store |
| `fractal-delivery-base` | `crates/fractal-delivery-base` | the message-delivery leaf modules, which name no store code: `blobs`, `message_body`, `person_request` and `mailbox`. It depends on `fractal-secrets`; `fractal-store` re-exports it at its root. The mailbox tests that bind and replay through the store live in fractal-store (`store::mailbox_tests`) |
| `fractal-seats-base` | `crates/fractal-seats-base` | the seat leaf modules, which name no store code: `native_resume`, `remote_terminal` and `node_identity`. It depends on `fractal-secrets`; `fractal-store` re-exports it at its root |
| `fractal-store` | `crates/fractal-store` | the claims store and the modules it needs, as one crate (the plan's R4 shape): `store` (store.rs and its domain files), `graph`, `mission`, `mailbox`, `message_body`, `blobs`, `client`, `client_values`, `environment`, `startup`, `native_resume`, `person_request`, `provenance`, `references`, `resource`, `rollout`, `suspension`, `sekrets`, `sekrets_daemon`, `silent`, `tile_registry`, `github_http`, `github_watch`, `declaration_syntax`, `gate_parse`, `backup`, `checkout`, `external_sessions`, `recorder_receipts`, `render`, and the part of `test_support` that needs only these. It depends on `fractal-kernel` and `fractal-telemetry`; fractal-daemon re-exports each module at its old path. Everything in it is `pub`, because the daemon's API, reconciler and peer code reach into it; the unit tests that need the API, the reconciler or seat snapshots live in fractal-daemon's `store_tests` |
| `fractal-delivery` | `crates/fractal-delivery` | the delivery domain, cut out of `fractal-store` at function level (`scripts/crate_cut.py`): the parts of `store::{delivery, mailbox_changes, mailbox_lease, mailbox_wakes, message_point_qualification, message_subscriptions, agent_messages, conversation_reads, glasses, directive_notes, assistant_write_gate}` and the custom-subject and projection-divergence reads only they use, that nothing in the fold, the schema, the open path or the code still in `fractal-store` reaches, with the tests that need them (`Store::message` stays in `fractal-store`: backup restore reads it). Its `Store` methods are extension traits (`DeliveryStore`, `AgentMessagesStore`, `ConversationReadsStore`, `DirectiveNotesStore`, `GlassesStore`, `MailboxChangesStore`, `MailboxLeaseStore`, `MailboxWakesStore`, `CustomStore`, `ProjectionDivergenceStore`) that callers import (`use fractal_daemon::DeliveryStore`); a module of the same name in this crate shadows the one in fractal-store and re-exports it, so every old path still resolves. It depends on `fractal-store`; `fractal-missions` re-exports it at its root |
| `fractal-missions` | `crates/fractal-missions` | the missions domain, cut out of `fractal-store` at function level (`scripts/crate_cut.py`): the parts of `store::{missions, mission_deadline_probe, mission_eligibility, mission_ivm, mission_list, person_work, work_list, adhoc_work, delegation, lanes, arrangements, coordination, reconcile_work_context, declarations}`, `graph` and `references` that nothing in the fold, the schema, the open path or the code still in `fractal-store` reaches, with the tests that need them (`parse_intent` and the authority-pattern validators stay in `fractal-store`). Its `Store` methods are extension traits (`MissionsStore`, `MissionsStoreCore`, `PersonWorkStore`, `WorkListStore`, `MissionListStore`, `MissionEligibilityStore`, `AdhocWorkStore`, `ArrangementsStore`, `CoordinationStore`, `DeclarationsStore`, `DelegationStore`, `LanesStore`; test fixtures `OperationsFixtureStore`) that callers import (`use fractal_daemon::MissionsStore`); a module of the same name in this crate shadows the one in fractal-store and re-exports it, so every old path still resolves. It depends on `fractal-delivery` (and names `fractal-store` for its shared test modules); `fractal-seats` re-exports it at its root |
| `fractal-seats` | `crates/fractal-seats` | the seats domain, cut out of `fractal-store` at function level (`scripts/crate_cut.py`): the parts of `store::{seats, seat_status, retirements, limits, subagents, rollouts, revision_seats, roster_admission, harness_questions, usage_period, accounts, trace_principal, runtime_starts, stop_deferral}`, `external_sessions`, `rollout`, `suspension` and `sekrets_daemon` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits (`SeatsStore`, `SeatsStoreCore`, `SeatStatusStore`, `RetirementsStore`, `LimitsStore`, `SubagentsStore`, `RolloutsStore`, `RevisionSeatsStore`, `RosterAdmissionStore`, `HarnessQuestionsStore`, `AccountsStore`, `TracePrincipalStore`, `RuntimeStartsStore`, `StopDeferralStore`, `SekretsDaemonStore`, `EvidenceSeatsStore`) that callers import (`use fractal_daemon::SeatsStore`); a module of the same name in this crate shadows the one in fractal-store and re-exports it, so every old path still resolves. It depends on `fractal-missions`; `fractal-evidence` re-exports it at its root |
| `fractal-attention` | `crates/fractal-attention` | the attention domain, cut out of `fractal-store` at function level (`scripts/crate_cut.py`): the parts of `store::{attention, attention_ivm, attention_list, attention_snapshot, summary_list, owner_lists, published_list, card_fold, agent_view, work_summaries_ivm, client_summary}` and `render` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits that callers import (`use fractal_daemon::AttentionStore`); a module of the same name in this crate shadows the one in fractal-store and re-exports it, so every old path still resolves. It depends on `fractal-seats`; `fractal-evidence` sits on it |
| `fractal-evidence` | `crates/fractal-evidence` | the evidence domain, cut out of `fractal-store` at function level (`scripts/crate_cut.py`): the parts of `store::{evidence, owned_sets, publication, publication_facts, published_views, terminal_gate_evidence}` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits (`EvidenceStore`, `OwnedSetsStore`, `PublicationStore`, `PublicationFactsStore`, `PublishedViewsStore`) that callers import (`use fractal_daemon::EvidenceStore`); a module of the same name in this crate shadows the one in fractal-store and re-exports it, so every old path still resolves. It depends on `fractal-seats`; `fractal-github` re-exports it at its root |
| `fractal-github` | `crates/fractal-github` | the GitHub domain: `changes` (the git change index, service, transport and views; the index is `changes::index`), the repository-subject reads (`github`), seats' watches (`watches`) and the idle-hold reads (`idle_holds`). Their `Store` methods are extension traits (`GitChangesStore`, `GithubStore`, `WatchesStore`, `IdleHoldsStore`) that callers import; the index tables stay in `fractal-store`. It depends on `fractal-store`; `fractal-peer` re-exports it at its root |
| `fractal-peer` | `crates/fractal-peer` | the node-to-node peer relay and raw-terminal routes (`peer`). It depends on `fractal-store`; the tests that run the API on a socket live in fractal-daemon's `peer_tests` |
| `fractal-reconcile` | `crates/fractal-reconcile` | the reconciler and the modules only it and the API use: `reconcile` (with `reconcile_rollout`), `delivery_presence`, `handoff`, `native_seed`, `placement`, `repositories`, `seat_snapshot`. It depends on `fractal-peer`; four test files that serve the API keep their API tests in fractal-daemon's `reconcile_tests` |
| `fractal-api-core` | `crates/fractal-api-core` | the HTTP API without its mailbox and client-v0 route families (`api`: the route table, the client-v0 session/scope/paging helpers the rest of the API uses, the request-latency windows; it was `st3-api` until those two were cut out) and the modules only it uses: `claude_channel`, `creation`, `delivery_hold`, `gate_check`, `graph_watch_ivm`, `objectives`, `subagent_sessions`, plus the API half of `test_support`. Its router takes the route families of the crates above it (`router_with`, `fabric_router_with`; `RouteFamily`): `router` alone serves the routes this crate answers. It depends on `fractal-reconcile` |
| `fractal-api-mailbox` | `crates/fractal-api-mailbox` | the mailbox stream API (`api::mailbox`: the per-seat delivery socket, bind, receipts and attachment routes, native peer authority, and the mailbox profile), cut out of `st3-api` (now `fractal-api-core`) with `scripts/crate_cut.py`; `hold_pre_boot_mail` and `never_offered` stay in `fractal-api-core` because `api::messages` calls them. Its routes are a route family (`api::mailbox::routes`). fractal-daemon's `api` module is fractal-api-core's `api` plus this crate's `mailbox`, and its `router` and `fabric_router` add every family the daemon serves (`ROUTE_FAMILIES`), so `fractal_daemon::api::router` serves every route as before; this crate's own `api::router` (fractal-api-core's routes and the mailbox family) is for its tests. The admitted and synthetic mailbox protocol routers take the families too (`*_with`); fractal-daemon's `test_support` builds them over its router. It depends on `fractal-api-core` alone |
| `fractal-api-client-v0` | `crates/fractal-api-client-v0` | the client-v0 API (`api::client_v0`: the collections, streams and windows, the assistant reads and their authorization, arrangements, inventory, search, conversation blocks, raw-terminal leases, published missions and runtimes, and their routes), cut out of `st3-api` (now `fractal-api-core`) with `scripts/crate_cut.py`, with the tests that need them, including the fractal-api-core tests that request a client-v0 route. What the rest of the API also uses stays in `fractal-api-core`'s `api::client_v0`: the session, scope and authentication helpers (`ClientSession`, `authenticate`, `require_scope`, `validation`, ...), the native page cursors, `collection_windows`, `summary` and the mission-card readers; this crate's `api::client_v0` shadows it and re-exports it, so every old path still resolves. Its routes are a route family (`api::client_v0::routes`) that fractal-daemon serves with the mailbox family. The wire contract it embeds (`include_str!` of `docs/st3/client-v0/schemas`) resolves from the same depth, unchanged. It depends on `fractal-api-core` alone |
| `fractal-daemon` | `crates/fractal-daemon` | what is left of the library: seats' driver hooks, attach_image, malloc_trim, omp_ask_resume, service, subagents, terminal_fabric and tiles, the re-exports of every moved module at its old path, the `test_support` shim, its `hooks/`, `fixtures/` and `proptest-regressions/`, and the unit tests that need more than one of the crates above (`store_tests`, `peer_tests`, `reconcile_tests`, `kernel_tests`, `telemetry_tests`) |
| `st3` | `crates/fractal` | the `st3` binary (`main.rs` and the modules only the binary uses: `cli_help`, `completion`, `directive_note`, `doctor_cli`, `change_cli`, `presentation`, `setup`, `onboarding`), the `fractal-fixture` binary, `build.rs`, `tests/`, `examples/`, and an empty library target (`src/package_marker.rs`, see below) |
| `fractal-e2e` | `crates/fractal-e2e` | tests only: the tests that start a real daemon and drive a client or the feed against it (fractal-client's `device_signing` and `transports`, the fractal-feed tests). `fractal-client` and `fractal-feed` no longer have a dev-dependency on `fractal-daemon`; `fractal-client` had one, which closed a cycle with the daemon. |

The binary and the tests depend on the library under the name `fractal-daemon`
(`fractal-daemon = { path = "../fractal-daemon" }`), so `use fractal_daemon::...` reads the same in
the binary, the tests and every crate that already used it. The reason for the split is Cargo's: a
package has one dependency set for its library and its binary, so a library that sat next to the
binary was rebuilt whenever `fractal-tui`, `fractal-conversation-ui` or `fractal-feed` changed, although it uses
none of them.

## Domain crates, cut at function level

fractal-store held the whole claims store. At file level it is one knot (`store.rs` calls the domain
files, which call `store.rs` back), but at function level it is not: the fold (`apply_as_impl`,
`try_project_simple_replication_tx`, ...), the schema, the projector hooks and everything they reach
form the knot, and the rest only calls down. `scripts/crate_cut.py` computes that split and applies
it, one domain at a time, so a domain's code lives in a crate above fractal-store: `fractal-store` <
the domain crates < `fractal-github` < `fractal-peer` < `fractal-reconcile` < `fractal-api-core` < `fractal-api-mailbox` and `fractal-api-client-v0` (side by side: neither names the other, and fractal-daemon serves both route families).

- **Methods are extension traits.** A domain's `impl Store` fns became methods of a trait, one per
  source file (`EvidenceStore`, `OwnedSetsStore`, ...), implemented for `Store` in the same file. Call
  syntax is unchanged; import the trait (`use fractal_daemon::EvidenceStore;` outside the chain, `use
  crate::EvidenceStore;` in a crate of the chain). To add a method, add its signature to the trait and
  its body to the impl in that file.
- **A module can live in two crates.** Where a module has code on both sides of a cut, the upper crate
  has a module of the same name that re-exports the lower one's items (`pub use
  fractal_store::store::evidence::*;`) and defines its own, so `crate::store::evidence::X` finds `X` in
  whichever crate it is. The lower crate's items the upper code uses are `pub`.
- **The core stays in fractal-store.** If code in the fold, the schema or the open path starts to name a
  domain item, the build fails in fractal-store (the item is not there); move the item back down, or invert
  the call. `scripts/crate_cut.py plan` lists what is core and why (`--show core`).
- **Tests go with what they use.** A test moves to the highest crate among the items it uses; the
  helpers it calls are copied next to it (types that are `pub` are shared, not copied).
- **To cut another domain** (or redo one after a merge conflict: a cut is cheaper to redo on the merged
  tree than to merge): `scripts/crate_cut.py plan` shows the placement, `apply --layer D --crate st3-D
  --lower <crate below>` moves the layer, `fix` makes the workspace build (visibility and trait
  imports), `sweep` removes what the move left unused. The domains, their order and the files that hint
  at them are in `.st/crate-cut.json`; `scripts/crate-cut-test` checks the tool on a small workspace.
  Layers are applied top first, so every step builds.

## The public surface of fractal-daemon

It is the library surface `st3` had: the modules `fractal_daemon::{api, client, config, graph, model, store,
reconcile, ...}`, the functions `parse_intent` and `validate_mission_runtimes`, and the `cli_print`
macros. Other crates use a part of it:

| consumer | uses |
|---|---|
| the `st3` binary and its tests | everything it needs |
| `fractal-tui` (dev) | `api`, `store`, `model`, `reconcile`, `client`, `graph`, `parse_intent` |
| `fractal-client` (dev) | `api`, `model`, `graph`, `store`, `test_support` |
| `fractal-feed` (dev) | `api`, `model`, `store`, `graph` |
| `fractal-migrate` | `parse_intent`, `validate_mission_runtimes`, `model` |

## What stays true, and what pins it

- **The library does not depend on `fractal-tui` or `fractal-conversation-ui`.** Both are binary-side. If the
  library needs something from them, move it to the binary or into a crate both can use.
- **`fractal` and `fractal-daemon` have the same version.** The library reports `CARGO_PKG_VERSION` in the
  health endpoint, the published fleet endpoints and the MCP `serverInfo`; a different version
  would change what the binary prints.
- **`crates/fractal/build.rs` and `crates/fractal-daemon/build.rs` are the same.** Both read the same
  `ST3_FIXTURE_*` variables for the same `test-support` feature: the library reads them in
  `test_support.rs`, the tests read them with `env!`.
- **`cargo test -p fractal --features test-support` still works**: the feature forwards to the library.
- **The `fractal` package switches on its own `test-support` in tests.** Before the split a dev-dependency
  of the package on itself did it, and `build.rs` (the `ST3_FIXTURE_*` variables), the `fractal-fixture`
  binary and the `required-features` targets depend on it. Cargo allows that only for a package with
  a library target, so `crates/fractal/src/package_marker.rs` is an empty library (named
  `fractal_package_marker`, in a file other than `lib.rs` so that other pull requests' edits to
  `lib.rs` follow the moved library) and `fractal_package_self` is the dev-dependency.
- **The fixtures only the library's unit tests read moved with it**
  (`crates/fractal-daemon/tests/fixtures/`). The binary's tests keep `crates/fractal/tests/fixtures/`.
- The nextest overrides select a library unit test by `package(fractal)` or `package(fractal-daemon)`.
- **fractal-kernel names nothing above it.** Its modules have no path to `store`, `api`, `reconcile` or
  `test_support`; a kernel unit test that needs daemon code (intent parsing, a store constant) lives
  in fractal-daemon's `kernel_tests` and reaches the kernel through the re-exports. A module joins the
  kernel once nothing it names (in production code or its tests) is outside the kernel.
  <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 -->
  The sole R11 exception is `fractal_client::product_dirs`: the standalone client owns
  the single resolver for the R11 state-directory migration. No other client API
  is allowed, including through crate aliases or grouped root imports.
  Remove this edge and its resolver at migration contraction.
  <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal -->
- **fractal-telemetry names nothing above the kernel.** Its modules reach `st3_kernel` and `smallclaims` only.
  Two tests read source files through `include_str!`: `startup_telemetry` reads `crates/fractal/src/main.rs`
  (`../../st3/src/main.rs`) and `slo` reads `crates/fractal-api-core/src/api.rs` (`../../fractal-api-core/src/api.rs`).
  <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 -->
  The sole R11 exception is `fractal_client::product_dirs`, for the same standalone
  resolver ownership reason. `scripts/check-module-layers` enforces this exact
  namespace exception in `src/`, `tests/`, `benches/` and `examples/` of both crates;
  all other client API paths remain forbidden. Renaming `fractal-client` in any
  dependency table, including dev/build and target-specific dependencies resolved
  through `[workspace.dependencies]`, is forbidden as well.
  Remove this edge and its resolver at contraction.
  <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal -->

`scripts/check-module-layers` reports the first three.
