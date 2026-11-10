# Client-control web modules

Source preparation stacked on the canonical generated contract. **Application test mode remains disabled.** These modules do not change existing entrypoints, pairings or read stores, and do not create enrollment or grant authority.

## APIs

- `identity.ts`: `getOrCreateClientControlIdentity()` atomically acquires an origin-local IndexedDB P-256 identity. The private key is non-extractable; public SPKI export and supplied-byte ECDSA/SHA-256 signing remain internal capabilities. Storage failures never replace a key or fall back to ephemeral identity. Key deletion is not enrollment revocation.
- `foreground.ts`: visible AND focused. Re-evaluate before navigation and signing.
- `navigation.ts`: only NowRoot and Attention(ask id) destinations; the host supplies its authoritative subject-to-href resolver.
- `grantBoundary.ts`: decodes actual delivered commands with the generated DeviceCommand schema; uses generated ClientControlGrant permissions directly, not a local wire projection. Only granted inspect or NowRoot/Attention navigation exists, with Attention restricted to the optional grant item_id (unbound grants cannot open/inspect an ask). Checks device/grant/run binding and rechecks target, permission and foreground before signing. There is no answer/close/read-state or DOM activation callback. The later authority adapter owns enrollment, session and window admission.
- `readAdvancement.ts`: future sticky page-lifetime prohibition checked by `gateReadAdvancement` at execution time. The read-position store is absent on this branch's main base, and the Attention route does not use it. This is not a present journey blocker: wrap the entire store advance (persistence and outbound synchronization) when that writer group lands, not individual UI callers. Enrollment ending must not re-enable writes.
- `ClientControlIndicator.tsx`: controller-owned run indicator and enrollment-revoke action with pending/unconfirmed states. The component does not create authority or optimistically hide an active grant.
- `fixtureGuard.ts`: requires the exact approved dedicated loopback origin and realpath-resolved fixture instance client socket, with normal-origin/socket deny lists. This is configuration validation, not daemon attestation or enrollment authority.
- `fixtureDomEvidence.ts`: independent real Playwright-compatible DOM observation of the open native ask's title and request.question text. The digest is lowercase SHA-256 of exact UTF-8 question text. Each must render as one unsplit leaf element; altered text/digest or IDs outside the verified actual catalog fail observation. Persists unsigned observations with exact candidate commit and origin.
- `fixtureText.ts`: both title and question must be printable ASCII, at most 60 characters, no Markdown syntax characters (asterisk, underscore, hash, square brackets, backtick, greater-than or vertical bar), JSON, or leading/trailing/repeated whitespace. The title comes from the verified fixture manifest, never a hard-coded value.
- `fixtureNoAnswerGuard.ts`: install on the dedicated Playwright context before opening the app; it blocks all subject-action mutation requests and fails assertions if any were attempted, even when successfully blocked. The observer asserts the guard before and after capture; the journey must assert it again at completion and must never press answer controls.
- `wire.ts`: generated command/grant/binding/result/receipt decode/encode boundaries, generated canonical signing bytes, native identity signing and generated DeviceResult envelope encoding. Canonicalization/digest/signed-field order come only from `clients/typescript/fractal-client/ClientControl.generated.ts`; no second canonicalizer. Signature metadata is caller-supplied generated DeviceSignature data, not enrollment authority. The DOM evidence is an independent observation, distinct from generated durable receipt evidence.

Colocated tests cover crypto/storage, foreground, suppression, indicator states, grant refusals, fixture refusals and independent DOM comparisons. `conversation/alerts/AnswerAlert.integration.test.tsx` exercises the SHIPPED AnswerAlert render path and asserts exact single-element title/question rendering without pressing any answer. Protocol shapes come from generated schemas; route and independent DOM observer inputs remain local app concepts.

D-5 integration assertions in that shipped-renderer test run `DataSourceProvider -> LiveAgentAlerts -> AnswerAlert` with recording `alertActions.act` and `prompts.respond` source ports, not mocked data hooks. Both structured choices and feedback/decision-words Send controls remain in the DOM, with no activation/input/submit events or confirmation state, across inspect/navigation, wrong-item and refused answer/close/read commands. Both port request logs must remain empty after EVERY command. This is a deterministic source-port test, not a fixture journey; its unit-only fixture source never supplies the real journey UI.

The real-browser transport guard independently records/rejects attempted `/v1/client/actions` writes at `fixtureNoAnswerGuard.ts:25-36`, including successfully blocked attempts. It does not by itself prove that answer controls were never activated, so the shipped-control assertions above are separate. `useAlertAnswers.ts` is absent on this main base: equivalent coverage through that hook is tied to `37083821b6` and is added when that native-detail group lands; current tests do not claim to execute the absent hook.

## Integration dependencies

Dependency decisions below refer to the read-only integration snapshot `ff956ae72b26d92be285b3d7ca245f2afb9e4f0e`; file paths are relative to `apps/fractal-web/src`. KEEP means required behavior, not permission to cherry-pick. No dependency was cherry-picked here.

| Source | Decision | Source reason |
| --- | --- | --- |
| `cac380ac98` workspace routes | KEEP | `workspace/surfaces.tsx:121-128` renders Inbox and native Alert detail; `workspace/useWorkspace.ts:34-37` opens the URL subject. |
| `df082cd9e8` typed subjects | KEEP | `workspace/subject.ts:81-82,140` maps Inbox and bound Alert to `/w/inbox` and `/w/inbox/<encoded id>`. |
| `349b0da2aa` kit-workspace migration | DROP as an independent journey blocker | `workspace/useWorkspace.ts:9-10,34` uses the host's store; the client-control interface accepts route targets, not kit store APIs. The chosen host may bring this transitively; no separate kit migration is required by the ask. |
| `5a107113bd` split panes | DROP as an independent journey blocker | `workspace/surfaces.tsx:121-128` requires only Inbox and Alert surfaces, not split commands or multiple panes. |
| `37083821b6` native decision-card/detail group | KEEP (newly identified) | `web/AlertDetailsPane.tsx:39-45,78-81` selects the open bound ask from the attention feed and renders shipped AnswerAlert. `conversation/alerts/useAlertAnswers.ts:25-30` reads attention/grants and observes the in-memory answer store; `workspace/registry.ts:76,89-90` assigns both surfaces only the shared attention feed. Reconcile the shipped-renderer DOM markers/tests when this group lands. |
| `9e66194639` read-position cells | DROP from current journey blockers | `state/cells/readPositions.ts:12-17,36-39` stores per-stream prose read positions. Native ask detail reads attention/answer state instead (`web/AlertDetailsPane.tsx:39-45`). |
| `0d21c1286b` read-position clock-skew/remount fix | DROP from current journey blockers | `web/readPosition.ts:45,125` exposes stream seen/mark-read hooks; native ask detail does not invoke them (`web/AlertDetailsPane.tsx:37-60`). |
| `45710bb1d5` last_prose_at unread fix | DROP from current journey blockers | The relevant write boundary remains `state/cells/readPositions.ts:36-39`; neither native ask surface requires it (`workspace/registry.ts:89-90`). The suppression integration must land with any introduced writer, before test-mode rendering can reach it. |
| [#6108 read-position sync](https://github.com/compoundingtech/fractal/pull/6108) | DROP from current journey blockers | Inspected head `01fd722d1cdfc4beb916817ab9dc68f0307b437b` adds lastProseAt projection/types, not a web sync caller. A future writer must share the gated store boundary above. |
| [#6274 generated contract and shared vectors](https://github.com/compoundingtech/fractal/pull/6274) | MERGED prerequisite | Signed no-ff merge of generated head `36592f7090093ae6fe94a87706e319fca78fca17` (source parent `9ac7024e`). Imported `ClientControl.generated.ts`, generated Effect schemas and repository `fixtures/clients/client-control-v1.golden.json`; no generated file hand-edited or copied. |
| Later enrollment readback/revoke PR | KEEP | Generated enrollment/revocation schemas and `FractalClient.clientTestRevoke` now exist, but authoritative browser enrollment readback/controller and live revoke confirmation remain integration prerequisites. |
| Later grant-subscription PR | KEEP | Commands and grants now have generated schemas; authoritative enrollment/session/run/key/window admission and grant lifecycle subscription remain unconnected. |
| Later verification/receipt PR | KEEP for journey acceptance | Actual canonical result verification and durable terminal receipts cannot be established by DOM observations. Handle/module pending. |

No file:line can be cited for unpublished later PR APIs; their absence remains an explicit integration blocker, not an assumed implementation.

#6278 is STACKED on #6274's branch `schickling-assistant/2026-11-10-client-control-daemon`, merged at generated head `36592f7090093ae6fe94a87706e319fca78fca17`. This supersedes the earlier main-first decision.

## Generated-contract consumption and vector coverage

`wire.unit.test.ts` imports the shared JSON directly. It covers BOTH published vectors (`inbox`, native Attention `unicode-opened-item`), their generated decode/encode roundtrips, exact JCS/digest/signing bytes, WebCrypto verification of fixed signatures, all TWENTY negative bindings, and both message/result purpose-confusion directions. It also signs both vector bindings with a real independently generated non-extractable browser identity and verifies the generated result envelope. These are conformance assertions in source, NOT executed evidence.

The shared JSON contains no grant/command/envelope/receipt or fingerprint golden vectors. Optional item_id present/absent and delivered native Attention command coverage therefore uses generated-schema roundtrip examples, not invented golden bytes; receipt examples likewise use the generated evidence schema. No published web-relevant vector is skipped. The fixed test scalar is public test material only, never an enrollment key. Unicode cryptographic compatibility does not relax the fixture's ASCII-only rendered-text rule.

## Open contracts

No guessed wire fields or signing bytes are implemented. Generated command/grant/visible-state/result/receipt schemas, canonical helpers, signed vectors and revoke SDK are now consumed or available. Still pending: authoritative enrollment readback and pairing/public-key binding controller; grant lifecycle delivery with session/window/budget admission; live revoke confirmation; fixture local-only attestation, manifest/readback and authorized bootstrap.

The generated schema currently has delivered command rows with optional control_grant, but no generated start/end/revoke lifecycle event type. That lifecycle remains owned by the daemon's existing commands collection work. No custom event envelope, second socket, handwritten lifecycle shape or permissive fallback is introduced: unsupported command/lifecycle input is refused by generated decoding; application activation stays disabled.

Application activation, command subscription, route-state capture and the complete journey await the remaining KEEP dependencies. Store wrapping is conditional on a read-position writer landing; that guard must land with the writer, not delay the current ask-only journey on a nonexistent store. Browser flags or preferences must never enable test mode on a normal pairing.

## Final seed and route ruling

The seed is one OPEN native ask (attention_kind person-step, presentation.kind request), with request.question exactly `client-control fixture 0001` and its title supplied by the fixture manifest. The journey is NowRoot -> that grant-bound ask's Attention screen -> inspect; its grant screens are NowRoot and Attention ONLY. Native detail is `/w/inbox/<id>`, using the attention feed and in-memory answer-state reads, never an answer action. The independent observer compares the manifest-supplied title and exact question-text digest using `[data-ask-title]` and `[data-ask-question] p` inside the ask's `data-item-id`. Fixture setup owns proving the ask remains initially unseen, open and unmutated before and after; the web journey never answers, closes or advances read state. The dedicated browser profile is retained across journeys, never reset to manufacture an unseen state.

## E3 secondary identity readback (FPL f1b5a6ac)

`readExistingPublicKeyPoint()` opens `fractal-client-control-identity-v1` without a version, aborts `onupgradeneeded`, and reads `identity.get('p256')` in a readonly transaction. It returns typed absence or the 65-byte uncompressed SEC1 public point extracted from exported P-256 SPKI. It never calls get-or-create, exports a private key, creates a committed database/store/record, or deletes storage. Pairing id is not locally available: this is a declared limitation.

The FPL f1b5a6ac fingerprint rule is lowercase hex SHA-256 of exactly that 65-byte point (`0x04 || X || Y`), not SPKI bytes. No separate fingerprint canonicalizer is introduced here; consume generated vector coverage when published. PRIMARY proof remains daemon-side: signed results verify against the one enrolled key and the enrollment record remains unchanged. Browser readback is SECONDARY only.

The E3 exclusion keys are exactly: IndexedDB `fractal-client-control-identity-v1/identity/p256`, cookies `fractal_device` and `fractal_pair`, and `wf.c/session.person`. Preserve the dedicated profile across journeys; these exclusions do not authorize deleting or resetting any other persisted state.

## Proof limits

No builds, typechecks, tests, linters, formatters, Storybook, Playwright, Rust, servers or browser captures were run during source preparation. No live setup, pairing, enrollment, grant or daemon operation was performed.

#6278 stays DRAFT. Source readiness requires focused LOCAL tests and the package's real typecheck at the exact candidate head, through an approved gate-slot ticket, plus independent exact-head review. Hosted CI is regression evidence only, never the reason to mark the source ready. Official generated TS tests reported by the owner at the generated head are not web integration evidence.

The prepared local runner uses the same `nix develop .#web` shell as web CI (`.github/workflows/workspace-ci.ts:998-999`). It records each exit status and retains outputs: frozen install; `pnpm --dir apps/fractal-web run typecheck` (the package script runs changelog-registry generation and `tsc --noEmit`); focused Vitest including `src/client-control`, shipped AnswerAlert integration and `wire.unit.test.ts`; official generated TS golden tests. The script is an external task artifact, not run during this source pass.

Hosted CI provides source regression evidence only, never journey acceptance. Journey acceptance requires a LOCAL exact-candidate real-fixture Playwright/DOM run after authorized setup/bootstrap/enrollment. Its runner must guard before opening connections; independently verify fixture/local-only daemon identity; supply the exact clean commit, approved loopback origin, fixture-only `WF_ST_GATEWAY` client socket and approved bootstrap; use the dedicated isolated browser profile/context without resetting it per journey; capture every pre-render HTTP/collections payload; use the actual manifest and real DOM selectors; persist evidence for authorized signing; and execute grant/refusal/not-foreground/revoke/read-state/receipt checks. Neither fixtureSource/embed rendering nor this independent observer alone is journey acceptance or a signed receipt.
