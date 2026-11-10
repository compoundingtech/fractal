# Client-control web source preparation

## Scope and status

Wire-independent source preparation only, based on origin/main d7aa0be1db. The parent explicitly deferred application activation and end-to-end integration until the authoritative contracts and route/store dependencies below exist. **Test mode remains disabled in the application.** No existing application entrypoint, pairing flow, normal session or read store is modified. No pairing, enrollment, grant, fixture instance or live daemon was created or touched. This is not a claimed mission step.

## Daemon handle and required dependencies

Canonical daemon source: [compoundingtech/fractal#6274](https://github.com/compoundingtech/fractal/pull/6274), branch `schickling-assistant/2026-11-10-client-control-daemon`, planning head `ceb85d30001faaaee311ac6dd145e10899490e6b`. The PR body was read. Its ordered plan is:

1. #6274: D-1/D-2 vocabulary, D-4 grant types, D-6 canonical purpose-specific results and shared fixed-key positive/negative golden-vector JSON; A-1 generated contracts and A-2 canonical signature tests. Canonical form/vector source was not yet published when this preparation was handed off. Browser conformance tests must READ the eventual repository vector file, never duplicate its fields or bytes.
2. Enrollment readback/revoke PR (handle pending): D-3 enrollment of an existing client-generated pairing/key and authoritative revocation.
3. Grant subscription PR (handle pending): D-4 mission/claimed-owner binding, active grant lifecycle delivered on the session-scoped commands subscription, admission restrictions.
4. Verification/receipt PR (handle pending): D-6 result verification and D-7 exactly one durable terminal receipt.
5. Workspace-model/routes group: `cac380ac98`, `df082cd9e8`, `349b0da2aa`, `5a107113bd`. Read-only inspection of the integration source found `workspace/subject.ts`: Inbox -> `/w/inbox`; Conversation -> `/w/<encoded agent ref>`; Alert -> `/w/inbox/<encoded id>`. `workspace/useWorkspace.ts` projects subject routing; `workspace/surfaces.tsx` renders it. No commits were cherry-picked.
6. State-cells read positions: `9e66194639`, `0d21c1286b`, `45710bb1d5`. The integration source writes browser-local positions through `state/cells/readPositions.ts` (`advance`), called from both `onSeen` and `useMarkRead` in `web/readPosition.ts`. Suppression must wrap that store boundary, not UI call sites.
7. #6108 read-position synchronization: branch `schickling-assistant/2026-11-08-read-position-sync`, inspected at `01fd722d1cdfc4beb916817ab9dc68f0307b437b`. Its web delta only adds lastProseAt projection/types; no web readPosition.ts or daemon synchronization caller exists there. The eventual daemon sync writer must remain inside the same gated advancement boundary as local persistence.

Earlier searches found only the old `agent/client-controls` branch (behind main, no new delta); #6274 supersedes that initial NOT FOUND result.

## Implemented source and integration contracts

All modules are under `apps/fractal-web/src/client-control/`:

- `identity.ts`: real WebCrypto P-256 private key, non-extractable, persisted via IndexedDB structured clone. Atomic readwrite read/conditional-add resolves concurrent acquisition without replacing a committed key. No ephemeral fallback. `getOrCreateClientControlIdentity()` exposes public SPKI export and ECDSA/SHA-256 signing of supplied canonical bytes; neither public-key wire encoding nor result serialization is invented. Key access is internal, never attached to window. Clearing site data loses identity and requires fresh pairing/enrollment; deleting a key is not revocation.
- `foreground.ts`: visible AND document focus; callers must re-evaluate before navigation and signing.
- `navigation.ts`: typed app destinations plus a host-supplied authoritative subject-to-href resolver. No alternate renderer or second routing codec.
- `readAdvancement.ts`: a sticky page-lifetime prohibition and execution-time `gateReadAdvancement`. The eventual authority adapter prohibits writes before test enrollment permits rendering; store advance wraps local persistence and outbound synchronization together. Ending enrollment never re-enables writes in that isolated page. This boundary is not yet wired because the store does not exist on this branch's base.
- `ClientControlIndicator.tsx`: run id, explicit enrollment-revoke action, pending and unconfirmed-revoke states. Controller owns authority and async state; the component does not optimistically hide the grant. Not mounted before authoritative enrollment/grant delivery exists.
- `fixtureGuard.ts`: strict exact dedicated IPv4 loopback origin plus independently approved, realpath-resolved fixture instance client socket and normal-origin/socket deny lists. Configuration guard only, not daemon attestation or enrollment authority.
- `fixtureDomEvidence.ts`: independent real Playwright-compatible DOM observer, never client-reported state. Compares seed title and exact UTF-8 rendered innerText SHA-256 and checks visible data-item-id values against verified actual subject catalog. Awaits evidence persistence for both comparison outcomes; records full exact-candidate commit, origin, time, comparisons and unsigned observation status. Caller must fail the journey on `passed: false`.
- Colocated tests cover real key signing/verification and storage acquisition, foreground matrix, local+sync suppression, revoke UI states, fixture refusals and independent DOM comparison. No assumed signing bytes are pinned or presented as conformance.

## ASSUMPTIONS / unresolved contracts

No guessed wire fields are implemented. All following mappings remain unresolved until their owners publish canonical source:

- Pairing-bound authoritative test-enrollment readback and key/public-key binding; browser requests or flags cannot enable mode on a normal pairing.
- Grant start/end/revoke event envelope and command device/session/grant/run/id/kind/window fields.
- Generated visible-state schema and the exact purpose-first claim-fields signing lines, ordering/escaping, JCS vectors and signature serialization.
- device.result submission envelope and enrollment-revoke operation.
- Fixture local-only identity attestation, actual manifest/readback interfaces and authorized bootstrap flow.
- Mapping an optional Conversation sessionId through the integration router; no guessed query parameter is added.
- Rendered body text expectation when fixture source is Markdown: observer hashes exact DOM innerText, never invents normalization of source Markdown.

Local types in this directory are internal module/observer inputs, NOT daemon wire schemas. There is no enabled test-client adapter, command subscription, generated visible-state capture, JCS/result conformance test or complete `.pw.test.ts` journey yet; those were explicitly deferred by the parent rather than fabricated.

## Proof policy and next executable step

NOT RUN: builds, typechecks, unit/integration tests, linters, formatters, Storybook, Playwright, Rust, runtime servers, live setup and browser captures. Source regression evidence belongs to hosted CI only; hosted CI is NEVER journey acceptance. The parent opens the draft PR and arranges hosted checks.

Exact next executable source-regression step on the approved hosted runner, from `apps/fractal-web`:

```sh
pnpm test -- src/client-control
```

Then hosted typecheck/build/format gates apply normally. Do not run those on the source-editing machine.

Journey acceptance remains a LOCAL exact-candidate Playwright/DOM run against the approved real fixture after authorized setup/bootstrap/enrollment, not a fake fixtureSource/embed path. The future runner must supply: exact clean commit; approved unused dedicated `http://127.0.0.1:<port>` origin; `WF_ST_GATEWAY` equal to the independently approved fixture instance CLIENT socket; realpath-resolved instance root/socket; normal-origin/socket deny lists; approved fixture-only bearer/bootstrap; verified actual fixture manifest; selectors for real visible seed title/body; durable evidence writer for later authorized signing. It must guard BEFORE server/browser connections, verify daemon fixture/local-only identity independently, create a fresh dedicated browser context, capture every pre-render HTTP/collections payload and execute the full grant/refusal/not-foreground/revoke/read-state/receipt checks. The observer alone is not a journey or signed receipt.

No `.pw.test.ts` with invented protocol, no fixture setup, no pairing credentials and no normal data are included. Canonical vectors and the dependent application/store/authority contracts are the exact next source prerequisites.
