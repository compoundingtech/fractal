# Client-control web modules

Wire-independent source preparation only. **Application test mode remains disabled.** These modules do not change existing entrypoints, pairings or read stores, and do not create enrollment or grant authority.

## APIs

- `identity.ts`: `getOrCreateClientControlIdentity()` atomically acquires an origin-local IndexedDB P-256 identity. The private key is non-extractable; public SPKI export and supplied-byte ECDSA/SHA-256 signing remain internal capabilities. Storage failures never replace a key or fall back to ephemeral identity. Key deletion is not enrollment revocation.
- `foreground.ts`: visible AND focused. Re-evaluate before navigation and signing.
- `navigation.ts`: only NowRoot and Attention(ask id) destinations; the host supplies its authoritative subject-to-href resolver. Conversation is outside the journey and refused locally.
- `grantBoundary.ts`: wire-independent handler allowing only granted inspect or NowRoot/Attention navigation. There is no answer/close/read-state or DOM activation callback. It rechecks permission and foreground before signing after navigation. Unit assertions cover Conversation and mutation refusals with no navigation/signing; generated decoding, identity/session/run/window validation and actual canonical signatures remain pending.
- `readAdvancement.ts`: sticky page-lifetime prohibition checked by `gateReadAdvancement` at execution time. Wrap the store's entire advance implementation, including persistence and outbound synchronization, rather than individual UI callers. Enrollment ending must not re-enable writes.
- `ClientControlIndicator.tsx`: controller-owned run indicator and enrollment-revoke action with pending/unconfirmed states. The component does not create authority or optimistically hide an active grant.
- `fixtureGuard.ts`: requires the exact approved dedicated loopback origin and realpath-resolved fixture instance client socket, with normal-origin/socket deny lists. This is configuration validation, not daemon attestation or enrollment authority.
- `fixtureDomEvidence.ts`: independent real Playwright-compatible DOM observation of the open native ask's title and request.question text. The digest is lowercase SHA-256 of exact UTF-8 question text. Each must render as one unsplit leaf element; altered text/digest or IDs outside the verified actual catalog fail observation. Persists unsigned observations with exact candidate commit and origin.
- `fixtureText.ts`: both title and question must be printable ASCII, at most 60 characters, no Markdown syntax characters (asterisk, underscore, hash, square brackets, backtick, greater-than or vertical bar), JSON, or leading/trailing/repeated whitespace. The title comes from the verified fixture manifest, never a hard-coded value.
- `fixtureNoAnswerGuard.ts`: install on the dedicated Playwright context before opening the app; it blocks all subject-action mutation requests and fails assertions if any were attempted, even when successfully blocked. The observer asserts the guard before and after capture; the journey must assert it again at completion and must never press answer controls.

Colocated tests cover crypto/storage, foreground, suppression, indicator states, grant refusals, fixture refusals and independent DOM comparisons. `conversation/alerts/AnswerAlert.integration.test.tsx` exercises the SHIPPED AnswerAlert render path and asserts exact single-element title/question rendering without pressing any answer. Internal module inputs are not daemon wire schemas.

## Integration dependencies

1. [Canonical daemon contract #6274](https://github.com/compoundingtech/fractal/pull/6274): inspected source head `b6c0d55d9325ec53f5285ac30b4ff38707cd22c8`, including `docs/st3/client-v0/device-commands.md`, `crates/fractal-client-codegen/templates/ClientControl.ts.in` and the fixed-key vector input template. This head is **NOT consumer-ready**: generated outputs and approved computed vectors await the announced codegen commit. Future conformance tests must read `fixtures/clients/client-control-v1.golden.json` from the repository, never duplicate its fields or bytes.
2. Subsequent enrollment readback/revoke and grant-subscription PRs: authoritative pairing/key enrollment and session-scoped grant lifecycle. Their handles remain pending. The subsequent verification/receipt PR supplies result verification and durable terminal receipts.
3. Workspace-model/routes group: `cac380ac98`, `df082cd9e8`, `349b0da2aa`, `5a107113bd`. Its `workspace/subject.ts` resolves Inbox to `/w/inbox` and Alert to `/w/inbox/<encoded ask id>`; `workspace/useWorkspace.ts` projects routing and `workspace/surfaces.tsx` renders it. None was cherry-picked here.
4. State-cells read-position group: `9e66194639`, `0d21c1286b`, `45710bb1d5`. Wrap `state/cells/readPositions.ts` store `advance`, used by both `onSeen` and `useMarkRead` in `web/readPosition.ts`.
5. [Read-position synchronization #6108](https://github.com/compoundingtech/fractal/pull/6108), inspected at `01fd722d1cdfc4beb916817ab9dc68f0307b437b`: its web delta adds lastProseAt projection/types, not a web synchronization caller. The eventual outbound writer belongs inside the same gated advancement boundary as browser-local persistence.

#6278 stays based on origin/main and consumes #6274's generated TS module and shared vector file after they land on main.

## Resolved source contract; consumption still blocked

#6274 now specifies the existing phone.inspect/phone.navigate vocabulary, generated visible-state and result-binding/grant types, purpose-first claim-fields canonicalization over the JCS visible-state digest, and P-256/SHA-256 IEEE P1363 signatures with unpadded base64url envelope serialization. The signing interface deliberately remains supplied-byte-only: the future adapter must consume `ClientControl.generated.ts` helpers/types rather than reproduce template fields locally. Native WebCrypto signature bytes are compatible with the specified P1363 format; envelope construction belongs to that adapter, not the identity store. These are source-contract resolutions, not conformance or consumer-readiness claims.

## Open contracts

No guessed wire fields or signing bytes are implemented. Still pending: consumer-ready generated outputs and approved computed golden vectors; authoritative enrollment readback and pairing/public-key binding adapter; grant start/end/revoke delivery and its executable authority semantics; enrollment-revoke operation; fixture local-only attestation, manifest/readback and authorized bootstrap. The published enrollment documentation describes local authority constraints, but does not yet supply the browser readback/revoke integration.

Activation, command subscription, generated capture/signing, actual store wrapping and the complete journey await those contracts. Browser flags or preferences must never enable test mode on a normal pairing.

## Final seed and route ruling

The seed is one OPEN native ask (attention_kind person-step, presentation.kind request), with request.question carrying the seeded text. The journey is NowRoot -> that ask's Attention screen -> inspect; its grant screens are NowRoot and Attention ONLY. Opening is the real AlertsTab local expansion through AgentAlerts/AnswerAlert, never an answer. The independent observer compares the manifest-supplied ask title and exact question-text digest using `[data-ask-title]` and `[data-ask-question] p` inside the ask's `data-item-id`. Fixture setup owns proving the ask remains open/unread before and after; the web journey never answers, closes or advances read state.

## Proof limits

No builds, typechecks, tests, linters, formatters, Storybook, Playwright, Rust, servers or browser captures were run during source preparation. No live setup, pairing, enrollment, grant or daemon operation was performed.

Next source-regression command on the approved hosted runner, from `apps/fractal-web`:

```sh
pnpm test -- src/client-control src/conversation/alerts/AnswerAlert.integration.test.tsx
```

Hosted CI provides source regression evidence only, never journey acceptance. Journey acceptance requires a LOCAL exact-candidate real-fixture Playwright/DOM run after authorized setup/bootstrap/enrollment. Its runner must guard before opening connections; independently verify the fixture/local-only daemon identity; supply the exact clean commit, approved unused loopback origin, fixture-only `WF_ST_GATEWAY` client socket and approved bootstrap; create a fresh dedicated browser context; capture every pre-render HTTP/collections payload; use the actual fixture manifest and real DOM selectors; persist evidence for authorized signing; and execute grant/refusal/not-foreground/revoke/read-state/receipt checks. Neither fixtureSource/embed rendering nor this independent observer alone is journey proof.
