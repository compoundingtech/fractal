# st3 crates: what is where and who may use what

Twenty packages carry what used to be the single `st3` package and its end-to-end tests:

| package | path | holds |
|---|---|---|
| `st3-kernel` | `crates/st3-kernel` | the library's leaf modules, which name no store, api or reconcile code: `accounts`, `archive`, `boot`, `cli_output`, `config`, `conversation_header`, `conversation_search`, `disk`, `evidence`, `fleet`, `gate_kinds`, `gate_report`, `github_admission`, `harness_events`, `hooks`, `instance`, `lane`, `memory`, `model`, `native_views`, `pricing`, `projection`, `publication`, `read_cache`, `rules`, `seat_queue`, `skill`, `terminal_binding`. st3-daemon re-exports each one at its old path (`pub use st3_kernel::model;`), so `crate::model` and `st3::model` read the same. <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 --> R11 allows the dependency on `st3-client` only for `st3_client::product_dirs`: the standalone client owns the single resolver for the R11 state-directory migration. <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal --> |
| `st3-telemetry` | `crates/st3-telemetry` | the library's telemetry modules, which name no store, api, client or reconcile code: `incremental`, `otel`, `otel_client_class`, `otlp`, `reconcile_telemetry`, `recorder`, `recorder_report`, `relay_trace`, `slo`, `slo_history`, `startup_telemetry`, `telemetry`. It depends on `st3-kernel` and `smallclaims`. st3-daemon re-exports each one at its old path. What would name the store or the API is a trait the daemon implements (`incremental::ChangeSource`, `otlp::ObservationSource`) or a value the caller passes (`telemetry::hook`'s submit, `otlp::run_slo`'s windows reader). `recorder_receipts` writes claims through the store and stays in st3-daemon; the unit tests that need the store, the graph or the API live in st3-daemon's `telemetry_tests`. <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 --> R11 allows the dependency on `st3-client` only for `st3_client::product_dirs`: the standalone client owns the single resolver for the R11 state-directory migration. <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal --> |
| `st3-secrets` | `crates/st3-secrets` | the modules the store and everything above it use that name no store code: `client` (the daemon client), `startup` (startup status), `tile_registry` and `sekrets` (the credential gateway). It depends on `st3-kernel` and `st3-telemetry`; `st3-store` re-exports it at its root |
| `st3-github-base` | `crates/st3-github-base` | the GitHub leaf modules, which name no store code: `environment` (login-shell capture), `github_http` (authenticated transport), `resource` (repository resources), `github_watch`, `checkout` and `silent`. It depends on `st3-secrets`; `st3-store` re-exports it at its root |
| `st3-missions-base` | `crates/st3-missions-base` | the mission-declaration leaf modules, which name no store code: `declaration_syntax`, `gate_parse`, `provenance` and `mission`. It depends on `st3-secrets`; `st3-store` re-exports it at its root. The tests that parse intents through the graph live in st3-store |
| `st3-delivery-base` | `crates/st3-delivery-base` | the message-delivery leaf modules, which name no store code: `blobs`, `message_body`, `person_request` and `mailbox`. It depends on `st3-secrets`; `st3-store` re-exports it at its root. The mailbox tests that bind and replay through the store live in st3-store (`store::mailbox_tests`) |
| `st3-seats-base` | `crates/st3-seats-base` | the seat leaf modules, which name no store code: `native_resume`, `remote_terminal` and `node_identity`. It depends on `st3-secrets`; `st3-store` re-exports it at its root |
| `st3-store` | `crates/st3-store` | the claims store and the modules it needs, as one crate (the plan's R4 shape): `store` (store.rs and its domain files), `graph`, `mission`, `mailbox`, `message_body`, `blobs`, `client`, `client_values`, `environment`, `startup`, `native_resume`, `person_request`, `provenance`, `references`, `resource`, `rollout`, `suspension`, `sekrets`, `sekrets_daemon`, `silent`, `tile_registry`, `github_http`, `github_watch`, `declaration_syntax`, `gate_parse`, `backup`, `checkout`, `external_sessions`, `recorder_receipts`, `render`, and the part of `test_support` that needs only these. It depends on `st3-kernel` and `st3-telemetry`; st3-daemon re-exports each module at its old path. Everything in it is `pub`, because the daemon's API, reconciler and peer code reach into it; the unit tests that need the API, the reconciler or seat snapshots live in st3-daemon's `store_tests` |
| `st3-delivery` | `crates/st3-delivery` | the delivery domain, cut out of `st3-store` at function level (`scripts/crate_cut.py`): the parts of `store::{delivery, mailbox_changes, mailbox_lease, mailbox_wakes, message_point_qualification, message_subscriptions, agent_messages, conversation_reads, glasses, directive_notes, assistant_write_gate}` and the custom-subject and projection-divergence reads only they use, that nothing in the fold, the schema, the open path or the code still in `st3-store` reaches, with the tests that need them (`Store::message` stays in `st3-store`: backup restore reads it). Its `Store` methods are extension traits (`DeliveryStore`, `AgentMessagesStore`, `ConversationReadsStore`, `DirectiveNotesStore`, `GlassesStore`, `MailboxChangesStore`, `MailboxLeaseStore`, `MailboxWakesStore`, `CustomStore`, `ProjectionDivergenceStore`) that callers import (`use st3::DeliveryStore`); a module of the same name in this crate shadows the one in st3-store and re-exports it, so every old path still resolves. It depends on `st3-store`; `st3-missions` re-exports it at its root |
| `st3-missions` | `crates/st3-missions` | the missions domain, cut out of `st3-store` at function level (`scripts/crate_cut.py`): the parts of `store::{missions, mission_deadline_probe, mission_eligibility, mission_ivm, mission_list, person_work, work_list, adhoc_work, delegation, lanes, arrangements, coordination, reconcile_work_context, declarations}`, `graph` and `references` that nothing in the fold, the schema, the open path or the code still in `st3-store` reaches, with the tests that need them (`parse_intent` and the authority-pattern validators stay in `st3-store`). Its `Store` methods are extension traits (`MissionsStore`, `MissionsStoreCore`, `PersonWorkStore`, `WorkListStore`, `MissionListStore`, `MissionEligibilityStore`, `AdhocWorkStore`, `ArrangementsStore`, `CoordinationStore`, `DeclarationsStore`, `DelegationStore`, `LanesStore`; test fixtures `OperationsFixtureStore`) that callers import (`use st3::MissionsStore`); a module of the same name in this crate shadows the one in st3-store and re-exports it, so every old path still resolves. It depends on `st3-delivery` (and names `st3-store` for its shared test modules); `st3-seats` re-exports it at its root |
| `st3-seats` | `crates/st3-seats` | the seats domain, cut out of `st3-store` at function level (`scripts/crate_cut.py`): the parts of `store::{seats, seat_status, retirements, limits, subagents, rollouts, revision_seats, roster_admission, harness_questions, usage_period, accounts, trace_principal, runtime_starts, stop_deferral}`, `external_sessions`, `rollout`, `suspension` and `sekrets_daemon` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits (`SeatsStore`, `SeatsStoreCore`, `SeatStatusStore`, `RetirementsStore`, `LimitsStore`, `SubagentsStore`, `RolloutsStore`, `RevisionSeatsStore`, `RosterAdmissionStore`, `HarnessQuestionsStore`, `AccountsStore`, `TracePrincipalStore`, `RuntimeStartsStore`, `StopDeferralStore`, `SekretsDaemonStore`, `EvidenceSeatsStore`) that callers import (`use st3::SeatsStore`); a module of the same name in this crate shadows the one in st3-store and re-exports it, so every old path still resolves. It depends on `st3-missions`; `st3-evidence` re-exports it at its root |
| `st3-attention` | `crates/st3-attention` | the attention domain, cut out of `st3-store` at function level (`scripts/crate_cut.py`): the parts of `store::{attention, attention_ivm, attention_list, attention_snapshot, summary_list, owner_lists, published_list, card_fold, agent_view, work_summaries_ivm, client_summary}` and `render` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits that callers import (`use st3::AttentionStore`); a module of the same name in this crate shadows the one in st3-store and re-exports it, so every old path still resolves. It depends on `st3-seats`; `st3-evidence` sits on it |
| `st3-evidence` | `crates/st3-evidence` | the evidence domain, cut out of `st3-store` at function level (`scripts/crate_cut.py`): the parts of `store::{evidence, owned_sets, publication, publication_facts, published_views, terminal_gate_evidence}` that nothing in the fold, the schema or the open path reaches, with the tests that need them. Its `Store` methods are extension traits (`EvidenceStore`, `OwnedSetsStore`, `PublicationStore`, `PublicationFactsStore`, `PublishedViewsStore`) that callers import (`use st3::EvidenceStore`); a module of the same name in this crate shadows the one in st3-store and re-exports it, so every old path still resolves. It depends on `st3-seats`; `st3-github` re-exports it at its root |
| `st3-github` | `crates/st3-github` | the GitHub domain: `changes` (the git change index, service, transport and views; the index is `changes::index`), the repository-subject reads (`github`), seats' watches (`watches`) and the idle-hold reads (`idle_holds`). Their `Store` methods are extension traits (`GitChangesStore`, `GithubStore`, `WatchesStore`, `IdleHoldsStore`) that callers import; the index tables stay in `st3-store`. It depends on `st3-store`; `st3-peer` re-exports it at its root |
| `st3-peer` | `crates/st3-peer` | the node-to-node peer relay and raw-terminal routes (`peer`). It depends on `st3-store`; the tests that run the API on a socket live in st3-daemon's `peer_tests` |
| `st3-reconcile` | `crates/st3-reconcile` | the reconciler and the modules only it and the API use: `reconcile` (with `reconcile_rollout`), `delivery_presence`, `handoff`, `native_seed`, `placement`, `repositories`, `seat_snapshot`. It depends on `st3-peer`; four test files that serve the API keep their API tests in st3-daemon's `reconcile_tests` |
| `st3-api` | `crates/st3-api` | the HTTP API (`api`, the client-v0 contract, the request-latency windows) and the modules only it uses: `claude_channel`, `creation`, `delivery_hold`, `gate_check`, `graph_watch_ivm`, `objectives`, `subagent_sessions`, plus the API half of `test_support`. Its router takes the route families of the crates above it (`router_with`, `fabric_router_with`; `RouteFamily`): `router` alone serves the routes this crate answers. It depends on `st3-reconcile` |
| `st3-api-mailbox` | `crates/st3-api-mailbox` | the mailbox stream API (`api::mailbox`: the per-seat delivery socket, bind, receipts and attachment routes, native peer authority, and the mailbox profile), cut out of `st3-api` with `scripts/crate_cut.py`; `hold_pre_boot_mail` and `never_offered` stay in `st3-api` because `api::messages` calls them. Its routes are a route family (`api::mailbox::routes`). st3-daemon's `api` module is st3-api's `api` plus this crate's `mailbox`, and its `router` and `fabric_router` add every family the daemon serves (`ROUTE_FAMILIES`), so `st3::api::router` serves every route as before; this crate's own `api::router` (st3-api's routes and the mailbox family) is for its tests. The admitted and synthetic mailbox protocol routers take the families too (`*_with`); st3-daemon's `test_support` builds them over its router. It depends on `st3-api` alone |
| `st3-api-client-v0` | `crates/st3-api-client-v0` | the client-v0 API (`api::client_v0`: the collections, streams and windows, the assistant reads and their authorization, arrangements, inventory, search, conversation blocks, raw-terminal leases, published missions and runtimes, and their routes), cut out of `st3-api` with `scripts/crate_cut.py`, with the tests that need them, including the st3-api tests that request a client-v0 route. What the rest of the API also uses stays in `st3-api`'s `api::client_v0`: the session, scope and authentication helpers (`ClientSession`, `authenticate`, `require_scope`, `validation`, ...), the native page cursors, `collection_windows`, `summary` and the mission-card readers; this crate's `api::client_v0` shadows it and re-exports it, so every old path still resolves. Its routes are a route family (`api::client_v0::routes`) that st3-daemon serves with the mailbox family. The wire contract it embeds (`include_str!` of `docs/st3/client-v0/schemas`) resolves from the same depth, unchanged. It depends on `st3-api` alone |
| `st3-daemon` | `crates/st3-daemon` | what is left of the library: seats' driver hooks, attach_image, malloc_trim, omp_ask_resume, service, subagents, terminal_fabric and tiles, the re-exports of every moved module at its old path, the `test_support` shim, its `hooks/`, `fixtures/` and `proptest-regressions/`, and the unit tests that need more than one of the crates above (`store_tests`, `peer_tests`, `reconcile_tests`, `kernel_tests`, `telemetry_tests`) |
| `st3` | `crates/st3` | the `st3` binary (`main.rs` and the modules only the binary uses: `cli_help`, `completion`, `directive_note`, `doctor_cli`, `change_cli`, `presentation`, `setup`, `onboarding`), the `st3-fixture` binary, `build.rs`, `tests/`, `examples/`, and an empty library target (`src/package_marker.rs`, see below) |
| `st3-e2e` | `crates/st3-e2e` | tests only: the tests that start a real daemon and drive a client or the feed against it (st3-client's `device_signing` and `transports`, the st3-feed tests). `st3-client` and `st3-feed` no longer have a dev-dependency on `st3-daemon`; `st3-client` had one, which closed a cycle with the daemon. |

The binary and the tests depend on the library under the name `st3`
(`st3 = { package = "st3-daemon", path = "../st3-daemon" }`), so `use st3::...` reads the same in
the binary, the tests and every crate that already used it. The reason for the split is Cargo's: a
package has one dependency set for its library and its binary, so a library that sat next to the
binary was rebuilt whenever `stui`, `st3-conversation-ui` or `st3-feed` changed, although it uses
none of them.

## Domain crates, cut at function level

st3-store held the whole claims store. At file level it is one knot (`store.rs` calls the domain
files, which call `store.rs` back), but at function level it is not: the fold (`apply_as_impl`,
`try_project_simple_replication_tx`, ...), the schema, the projector hooks and everything they reach
form the knot, and the rest only calls down. `scripts/crate_cut.py` computes that split and applies
it, one domain at a time, so a domain's code lives in a crate above st3-store: `st3-store` <
the domain crates < `st3-github` < `st3-peer` < `st3-reconcile` < `st3-api` < `st3-api-mailbox` and `st3-api-client-v0` (side by side: neither names the other, and st3-daemon serves both route families).

- **Methods are extension traits.** A domain's `impl Store` fns became methods of a trait, one per
  source file (`EvidenceStore`, `OwnedSetsStore`, ...), implemented for `Store` in the same file. Call
  syntax is unchanged; import the trait (`use st3::EvidenceStore;` outside the chain, `use
  crate::EvidenceStore;` in a crate of the chain). To add a method, add its signature to the trait and
  its body to the impl in that file.
- **A module can live in two crates.** Where a module has code on both sides of a cut, the upper crate
  has a module of the same name that re-exports the lower one's items (`pub use
  st3_store::store::evidence::*;`) and defines its own, so `crate::store::evidence::X` finds `X` in
  whichever crate it is. The lower crate's items the upper code uses are `pub`.
- **The core stays in st3-store.** If code in the fold, the schema or the open path starts to name a
  domain item, the build fails in st3-store (the item is not there); move the item back down, or invert
  the call. `scripts/crate_cut.py plan` lists what is core and why (`--show core`).
- **Tests go with what they use.** A test moves to the highest crate among the items it uses; the
  helpers it calls are copied next to it (types that are `pub` are shared, not copied).
- **To cut another domain** (or redo one after a merge conflict: a cut is cheaper to redo on the merged
  tree than to merge): `scripts/crate_cut.py plan` shows the placement, `apply --layer D --crate st3-D
  --lower <crate below>` moves the layer, `fix` makes the workspace build (visibility and trait
  imports), `sweep` removes what the move left unused. The domains, their order and the files that hint
  at them are in `.st/crate-cut.json`; `scripts/crate-cut-test` checks the tool on a small workspace.
  Layers are applied top first, so every step builds.

## The public surface of st3-daemon

It is the library surface `st3` had: the modules `st3::{api, client, config, graph, model, store,
reconcile, ...}`, the functions `parse_intent` and `validate_mission_runtimes`, and the `cli_print`
macros. Other crates use a part of it:

| consumer | uses |
|---|---|
| the `st3` binary and its tests | everything it needs |
| `stui` (dev) | `api`, `store`, `model`, `reconcile`, `client`, `graph`, `parse_intent` |
| `st3-client` (dev) | `api`, `model`, `graph`, `store`, `test_support` |
| `st3-feed` (dev) | `api`, `model`, `store`, `graph` |
| `st3-migrate` | `parse_intent`, `validate_mission_runtimes`, `model` |

## What stays true, and what pins it

- **The library does not depend on `stui` or `st3-conversation-ui`.** Both are binary-side. If the
  library needs something from them, move it to the binary or into a crate both can use.
- **`st3` and `st3-daemon` have the same version.** The library reports `CARGO_PKG_VERSION` in the
  health endpoint, the published fleet endpoints and the MCP `serverInfo`; a different version
  would change what the binary prints.
- **`crates/st3/build.rs` and `crates/st3-daemon/build.rs` are the same.** Both read the same
  `ST3_FIXTURE_*` variables for the same `test-support` feature: the library reads them in
  `test_support.rs`, the tests read them with `env!`.
- **`cargo test -p st3 --features test-support` still works**: the feature forwards to the library.
- **The `st3` package switches on its own `test-support` in tests.** Before the split a dev-dependency
  of the package on itself did it, and `build.rs` (the `ST3_FIXTURE_*` variables), the `st3-fixture`
  binary and the `required-features` targets depend on it. Cargo allows that only for a package with
  a library target, so `crates/st3/src/package_marker.rs` is an empty library (named
  `st3_package_marker`, in a file other than `lib.rs` so that other pull requests' edits to
  `lib.rs` follow the moved library) and `st3_package_self` is the dev-dependency.
- **The fixtures only the library's unit tests read moved with it**
  (`crates/st3-daemon/tests/fixtures/`). The binary's tests keep `crates/st3/tests/fixtures/`.
- The nextest overrides select a library unit test by `package(st3)` or `package(st3-daemon)`.
- **st3-kernel names nothing above it.** Its modules have no path to `store`, `api`, `reconcile` or
  `test_support`; a kernel unit test that needs daemon code (intent parsing, a store constant) lives
  in st3-daemon's `kernel_tests` and reaches the kernel through the re-exports. A module joins the
  kernel once nothing it names (in production code or its tests) is outside the kernel.
  <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 -->
  The sole R11 exception is `st3_client::product_dirs`: the standalone client owns
  the single resolver for the R11 state-directory migration. No other client API
  is allowed, including through crate aliases or grouped root imports.
  Remove this edge and its resolver at migration contraction.
  <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal -->
- **st3-telemetry names nothing above the kernel.** Its modules reach `st3_kernel` and `smallclaims` only.
  Two tests read source files through `include_str!`: `startup_telemetry` reads `crates/st3/src/main.rs`
  (`../../st3/src/main.rs`) and `slo` reads `crates/st3-api/src/api.rs` (`../../st3-api/src/api.rs`).
  <!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032 -->
  The sole R11 exception is `st3_client::product_dirs`, for the same standalone
  resolver ownership reason. `scripts/check-module-layers` enforces this exact
  namespace exception in `src/`, `tests/`, `benches/` and `examples/` of both crates;
  all other client API paths remain forbidden. Renaming `st3-client` in any
  dependency table, including dev/build and target-specific dependencies resolved
  through `[workspace.dependencies]`, is forbidden as well.
  Remove this edge and its resolver at contraction.
  <!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal -->

`scripts/check-module-layers` reports the first three.
