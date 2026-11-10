# Client-control web modules

Wire-independent source preparation only. **Application test mode remains disabled.** These modules do not change existing entrypoints, pairings or read stores, and do not create enrollment or grant authority.

## APIs

- `identity.ts`: `getOrCreateClientControlIdentity()` atomically acquires an origin-local IndexedDB P-256 identity. The private key is non-extractable; public SPKI export and supplied-byte ECDSA/SHA-256 signing remain internal capabilities. Storage failures never replace a key or fall back to ephemeral identity. Key deletion is not enrollment revocation.
- `foreground.ts`: visible AND focused. Re-evaluate before navigation and signing.
- `navigation.ts`: only NowRoot and Attention(ask id) destinations; the host supplies its authoritative subject-to-href resolver.
- `grantBoundary.ts`: wire-independent handler allowing only granted inspect or NowRoot/Attention navigation, with Attention restricted to the grant-bound ask. There is no answer/close/read-state or DOM activation callback. It rechecks current target, permission and foreground before signing after navigation. Unit assertions cover nongranted screens, wrong-item and mutation refusals with no navigation/signing; generated decoding, identity/session/run/window validation and actual canonical signatures remain pending.
- `readAdvancement.ts`: future sticky page-lifetime prohibition checked by `gateReadAdvancement` at execution time. The read-position store is absent on this branch's main base, and the Attention route does not use it. This is not a present journey blocker: wrap the entire store advance (persistence and outbound synchronization) when that writer group lands, not individual UI callers. Enrollment ending must not re-enable writes.
- `ClientControlIndicator.tsx`: controller-owned run indicator and enrollment-revoke action with pending/unconfirmed states. The component does not create authority or optimistically hide an active grant.
- `fixtureGuard.ts`: requires the exact approved dedicated loopback origin and realpath-resolved fixture instance client socket, with normal-origin/socket deny lists. This is configuration validation, not daemon attestation or enrollment authority.
- `fixtureDomEvidence.ts`: independent real Playwright-compatible DOM observation of the open native ask's title and request.question text. The digest is lowercase SHA-256 of exact UTF-8 question text. Each must render as one unsplit leaf element; altered text/digest or IDs outside the verified actual catalog fail observation. Persists unsigned observations with exact candidate commit and origin.
- `fixtureText.ts`: both title and question must be printable ASCII, at most 60 characters, no Markdown syntax characters (asterisk, underscore, hash, square brackets, backtick, greater-than or vertical bar), JSON, or leading/trailing/repeated whitespace. The title comes from the verified fixture manifest, never a hard-coded value.
- `fixtureNoAnswerGuard.ts`: install on the dedicated Playwright context before opening the app; it blocks all subject-action mutation requests and fails assertions if any were attempted, even when successfully blocked. The observer asserts the guard before and after capture; the journey must assert it again at completion and must never press answer controls.

Colocated tests cover crypto/storage, foreground, suppression, indicator states, grant refusals, fixture refusals and independent DOM comparisons. `conversation/alerts/AnswerAlert.integration.test.tsx` exercises the SHIPPED AnswerAlert render path and asserts exact single-element title/question rendering without pressing any answer. Internal module inputs are not daemon wire schemas.

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
| [#6274 generated contract and shared vectors](https://github.com/compoundingtech/fractal/pull/6274) | KEEP | Inspected source head `6ad7f674d4503ce4dddd2186c66943c9a1b08751`; `crates/fractal-client-codegen/templates/ClientControl.ts.in` is source, not consumer-ready output. Consume generated `ClientControl.generated.ts` and repository `fixtures/clients/client-control-v1.golden.json` after they land, never copy vector fields/bytes. |
| Later enrollment readback/revoke PR | KEEP | Authoritative pairing/key enrollment and enrollment-ending revoke are required before activation; browser flags cannot supply authority. Handle/readback module not yet published. |
| Later grant-subscription PR | KEEP | Commands and grant lifecycle must be session/run/key/item bound; the internal grant permission is not authoritative subscription input. Handle/module pending. |
| Later verification/receipt PR | KEEP for journey acceptance | Actual canonical result verification and durable terminal receipts cannot be established by DOM observations. Handle/module pending. |

No file:line can be cited for unpublished later PR APIs; their absence remains an explicit integration blocker, not an assumed implementation.

#6278 stays based on origin/main and consumes #6274's generated TS module and shared vector file after they land on main.

## Resolved source contract; consumption still blocked

#6274 now specifies the existing phone.inspect/phone.navigate vocabulary, generated visible-state and result-binding/grant types, purpose-first claim-fields canonicalization over the JCS visible-state digest, and P-256/SHA-256 IEEE P1363 signatures with unpadded base64url envelope serialization. The signing interface deliberately remains supplied-byte-only: the future adapter must consume `ClientControl.generated.ts` helpers/types rather than reproduce template fields locally. Native WebCrypto signature bytes are compatible with the specified P1363 format; envelope construction belongs to that adapter, not the identity store. These are source-contract resolutions, not conformance or consumer-readiness claims.

## Open contracts

No guessed wire fields or signing bytes are implemented. Still pending: consumer-ready generated outputs and approved computed golden vectors; authoritative enrollment readback and pairing/public-key binding adapter; grant start/end/revoke delivery and its executable authority semantics; enrollment-revoke operation; fixture local-only attestation, manifest/readback and authorized bootstrap. The published enrollment documentation describes local authority constraints, but does not yet supply the browser readback/revoke integration.

Activation, command subscription, generated capture/signing and the complete journey await the KEEP dependencies. Store wrapping is conditional on a read-position writer landing; that guard must land with the writer, not delay the current ask-only journey on a nonexistent store. Browser flags or preferences must never enable test mode on a normal pairing.

## Final seed and route ruling

The seed is one OPEN native ask (attention_kind person-step, presentation.kind request), with request.question exactly `client-control fixture 0001` and its title supplied by the fixture manifest. The journey is NowRoot -> that grant-bound ask's Attention screen -> inspect; its grant screens are NowRoot and Attention ONLY. Native detail is `/w/inbox/<id>`, using the attention feed and in-memory answer-state reads, never an answer action. The independent observer compares the manifest-supplied title and exact question-text digest using `[data-ask-title]` and `[data-ask-question] p` inside the ask's `data-item-id`. Fixture setup owns proving the ask remains initially unseen, open and unmutated before and after; the web journey never answers, closes or advances read state. The dedicated browser profile is retained across journeys, never reset to manufacture an unseen state.

## E3 secondary identity readback (FPL f1b5a6ac)

`readExistingPublicKeyPoint()` opens `fractal-client-control-identity-v1` without a version, aborts `onupgradeneeded`, and reads `identity.get('p256')` in a readonly transaction. It returns typed absence or the 65-byte uncompressed SEC1 public point extracted from exported P-256 SPKI. It never calls get-or-create, exports a private key, creates a committed database/store/record, or deletes storage. Pairing id is not locally available: this is a declared limitation.

The FPL f1b5a6ac fingerprint rule is lowercase hex SHA-256 of exactly that 65-byte point (`0x04 || X || Y`), not SPKI bytes. No separate fingerprint canonicalizer is introduced here; consume generated vector coverage when published. PRIMARY proof remains daemon-side: signed results verify against the one enrolled key and the enrollment record remains unchanged. Browser readback is SECONDARY only.

The E3 exclusion keys are exactly: IndexedDB `fractal-client-control-identity-v1/identity/p256`, cookies `fractal_device` and `fractal_pair`, and `wf.c/session.person`. Preserve the dedicated profile across journeys; these exclusions do not authorize deleting or resetting any other persisted state.

## Proof limits

No builds, typechecks, tests, linters, formatters, Storybook, Playwright, Rust, servers or browser captures were run during source preparation. No live setup, pairing, enrollment, grant or daemon operation was performed.

Next source-regression command on the approved hosted runner, from `apps/fractal-web`:

```sh
pnpm test -- src/client-control src/conversation/alerts/AnswerAlert.integration.test.tsx
```

Hosted CI provides source regression evidence only, never journey acceptance. Journey acceptance requires a LOCAL exact-candidate real-fixture Playwright/DOM run after authorized setup/bootstrap/enrollment. Its runner must guard before opening connections; independently verify fixture/local-only daemon identity; supply the exact clean commit, approved loopback origin, fixture-only `WF_ST_GATEWAY` client socket and approved bootstrap; use the dedicated isolated browser profile/context without resetting it per journey; capture every pre-render HTTP/collections payload; use the actual manifest and real DOM selectors; persist evidence for authorized signing; and execute grant/refusal/not-foreground/revoke/read-state/receipt checks. Neither fixtureSource/embed rendering nor this independent observer alone is journey acceptance or a signed receipt.
