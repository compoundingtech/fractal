# Client-control web modules

Wire-independent source preparation only. **Application test mode remains disabled.** These modules do not change existing entrypoints, pairings or read stores, and do not create enrollment or grant authority.

## APIs

- `identity.ts`: `getOrCreateClientControlIdentity()` atomically acquires an origin-local IndexedDB P-256 identity. The private key is non-extractable; public SPKI export and supplied-byte ECDSA/SHA-256 signing remain internal capabilities. Storage failures never replace a key or fall back to ephemeral identity. Key deletion is not enrollment revocation.
- `foreground.ts`: visible AND focused. Re-evaluate before navigation and signing.
- `navigation.ts`: typed app destinations; the host supplies its authoritative subject-to-href resolver, not an alternate renderer or routing codec.
- `readAdvancement.ts`: sticky page-lifetime prohibition checked by `gateReadAdvancement` at execution time. Wrap the store's entire advance implementation, including persistence and outbound synchronization, rather than individual UI callers. Enrollment ending must not re-enable writes.
- `ClientControlIndicator.tsx`: controller-owned run indicator and enrollment-revoke action with pending/unconfirmed states. The component does not create authority or optimistically hide an active grant.
- `fixtureGuard.ts`: requires the exact approved dedicated loopback origin and realpath-resolved fixture instance client socket, with normal-origin/socket deny lists. This is configuration validation, not daemon attestation or enrollment authority.
- `fixtureDomEvidence.ts`: independent real Playwright-compatible DOM observation, comparing seed title, exact UTF-8 rendered innerText SHA-256 and visible item IDs against the verified actual subject catalog. Persists unsigned observations with exact candidate commit and origin; the runner must fail on `passed: false`.

Colocated tests cover crypto/storage, foreground, suppression, indicator states, fixture refusals and independent DOM comparisons. Internal module inputs are not daemon wire schemas.

## Integration dependencies

1. [Canonical daemon contract #6274](https://github.com/compoundingtech/fractal/pull/6274): inspected source head `b6c0d55d9325ec53f5285ac30b4ff38707cd22c8`, including `docs/st3/client-v0/device-commands.md`, `crates/fractal-client-codegen/templates/ClientControl.ts.in` and the fixed-key vector input template. This head is **NOT consumer-ready**: generated outputs and approved computed vectors await the announced codegen commit. Future conformance tests must read `fixtures/clients/client-control-v1.golden.json` from the repository, never duplicate its fields or bytes.
2. Subsequent enrollment readback/revoke and grant-subscription PRs: authoritative pairing/key enrollment and session-scoped grant lifecycle. Their handles remain pending. The subsequent verification/receipt PR supplies result verification and durable terminal receipts.
3. Workspace-model/routes group: `cac380ac98`, `df082cd9e8`, `349b0da2aa`, `5a107113bd`. Its `workspace/subject.ts` resolves Inbox to `/w/inbox` and Conversation to `/w/<encoded agent ref>`; `workspace/useWorkspace.ts` projects routing and `workspace/surfaces.tsx` renders it. None was cherry-picked here.
4. State-cells read-position group: `9e66194639`, `0d21c1286b`, `45710bb1d5`. Wrap `state/cells/readPositions.ts` store `advance`, used by both `onSeen` and `useMarkRead` in `web/readPosition.ts`.
5. [Read-position synchronization #6108](https://github.com/compoundingtech/fractal/pull/6108), inspected at `01fd722d1cdfc4beb916817ab9dc68f0307b437b`: its web delta adds lastProseAt projection/types, not a web synchronization caller. The eventual outbound writer belongs inside the same gated advancement boundary as browser-local persistence.

#6278 stays based on origin/main and consumes #6274's generated TS module and shared vector file after they land on main.

## Resolved source contract; consumption still blocked

#6274 now specifies the existing phone.inspect/phone.navigate vocabulary, generated visible-state and result-binding/grant types, purpose-first claim-fields canonicalization over the JCS visible-state digest, and P-256/SHA-256 IEEE P1363 signatures with unpadded base64url envelope serialization. The signing interface deliberately remains supplied-byte-only: the future adapter must consume `ClientControl.generated.ts` helpers/types rather than reproduce template fields locally. Native WebCrypto signature bytes are compatible with the specified P1363 format; envelope construction belongs to that adapter, not the identity store. These are source-contract resolutions, not conformance or consumer-readiness claims.

## Open contracts

No guessed wire fields or signing bytes are implemented. Still pending: consumer-ready generated outputs and approved computed golden vectors; authoritative enrollment readback and pairing/public-key binding adapter; grant start/end/revoke delivery and its executable authority semantics; enrollment-revoke operation; fixture local-only attestation, manifest/readback and authorized bootstrap; optional Conversation sessionId routing; and rendered-body expectations when fixture source is Markdown. The published enrollment documentation describes local authority constraints, but does not yet supply the browser readback/revoke integration. The observer does not invent Markdown normalization.

Activation, command subscription, generated capture/signing, actual store wrapping and the complete journey await those contracts. Browser flags or preferences must never enable test mode on a normal pairing.

## Proof limits

No builds, typechecks, tests, linters, formatters, Storybook, Playwright, Rust, servers or browser captures were run during source preparation. No live setup, pairing, enrollment, grant or daemon operation was performed.

Next source-regression command on the approved hosted runner, from `apps/fractal-web`:

```sh
pnpm test -- src/client-control
```

Hosted CI provides source regression evidence only, never journey acceptance. Journey acceptance requires a LOCAL exact-candidate real-fixture Playwright/DOM run after authorized setup/bootstrap/enrollment. Its runner must guard before opening connections; independently verify the fixture/local-only daemon identity; supply the exact clean commit, approved unused loopback origin, fixture-only `WF_ST_GATEWAY` client socket and approved bootstrap; create a fresh dedicated browser context; capture every pre-render HTTP/collections payload; use the actual fixture manifest and real DOM selectors; persist evidence for authorized signing; and execute grant/refusal/not-foreground/revoke/read-state/receipt checks. Neither fixtureSource/embed rendering nor this independent observer alone is journey proof.
