# Shared client views

`@fractal/views` is the phone's view model, usable by an IDE or another TypeScript
client. It exports TypeScript source, like the sibling generated `@fractal/client`.
Use a TypeScript-aware bundler, or Node 24 with `--experimental-transform-types`.
It has no React, React Native, Expo or platform key-store dependency.

The phone depends on this package through `workspace:*`, and this package uses
`workspace:*` for the generated client. Other apps in the root pnpm workspace can
use the same dependency convention.

```ts
import { conversationEntries, applyConversation, homeRows, clientName } from '@fractal/views';

const held = applyConversation(undefined, frame);
const entries = conversationEntries(held.entries, agentNames);
const rows = homeRows(attention, sessionActor);
const name = clientName('fractal-ide', version, build);
```

The root exports all functions and types. Subpaths named after the source modules
(`conversationView`, `conversationSimple`, `sessionView`, `homeView`, `requestView`,
`deviceSigning`, `clientName`, `time`, `sidebarView`) expose the same APIs individually.

Conversation parsing, simplified tool bundles, live-window/history joining, session
discovery, Home filtering/grouping, requests and structured answers live here. Home
colors are semantic tokens (`person`, `green`); renderers resolve them through their
palette. `clientName` takes the product name, version and optional build explicitly.
Signing builds canonical bytes and signature parameters; the caller owns the key,
hashing and signing. `signatureRefusal` accepts a device label (default `device`),
and the phone passes `phone` to retain its wording.

## Folder-only sidebar semantics

`@fractal/views/sidebarView` is the React-free twin of
`fractal_sidebar::projection`. Rust hosts supply `model::ArrangementInput` with
schema bodies and explicit resolved maps; the default crate has no client dependency.
`build(arrangement, inventory)` projects native version-1 arrangements using the daemon's
authoritative `resolved` maps. Its tagged result
is `Built { tree }` or `Refused { error }`; missing resolution is `NotReady`,
not a guessed parent graph. Opaque register revisions cannot establish which
concurrent edge won a cycle.

The tree retains stored `parent`, resolved `shownUnder`, position `key` and
`revision`. Folder and subject sibling orders remain separate: folders first,
then placed subjects, each ordered by key and identity; inventory-only subjects
follow in identity order. Renderers still own labels, filtering, collapse state,
host groups and the Fractal terminal UI's flat folder presentation. `nodeKey` encodes identity
without changing it. The inventory is additive: placements absent from it,
including dormant and non-agent subjects, remain in the tree. A partial
inventory never removes a placement or changes its resolved ancestry.

`canPlace` and `planMove` validate semantic `{ node, parent, before }` intents
against the full tree, including dormant placements. `before: null` means append
within the node's folder/subject order domain; cross-domain anchors are refused.
Only folders can be parents. Refusals are `_tag` unions with structured
references, not display text. Planning does not allocate fractional keys, emit
wire operations or replace fresh daemon admission. An explicit move of a lifted
row to its shown parent is not treated as unchanged.

`rollup(tree, needsYou)` counts subjects through resolved folder ancestry and
returns folder-keyed `{ needsYou, display: 'Always' }` descriptors. Presentation
copy and widgets remain consumer-owned.

Shared Rust/TypeScript vectors live in `fixtures/clients/sidebar/behavior-v1.json`,
validated by `behavior.schema.json` (JSON Schema 2020-12). `schemaVersion: 1`
versions the behavior format, independently of arrangement body versions.
Each case supplies confirmed input, catalog, ordered events, injected identities
and checkpoints. The current `ProjectAndPlanV1` event preserves the pure v1
projection/move/rollup cases; it does not dispatch wire actions or use identities,
memberships or a journal. Those fields are explicitly empty, not skipped tests.
Unknown fields and unsupported versions are rejected. New event variants arrive
with passing production-path cases. Folder identities are fixed UUIDv7 literals;
the TypeScript twin also decodes every arrangement with the strict client schema.
The Rust integration runner is `crates/fractal-sidebar/tests/behavior.rs`;
`cargo test -p fractal-sidebar --test behavior` exercises the public core against
the same validated checkpoints, including exact move refusals and empty effect/action lists.
This seam changes no storage, schema or wire format. Mixed parents, declared
defaults/reset, availability, optimistic application and revision/cycle-cut
settlement belong to format-v2 work, not this API.

`node scripts/check-sidebar-fixtures.mjs` gates every sidebar JSON fixture and
seed snapshot recursively before publication. The TypeScript runner invokes the
same gate before replay. `node --test scripts/check-sidebar-fixtures.test.mjs`
exercises the public-data policy and schema checks. Subjects use `person/test`,
`agent/test/*`, or numbered `machine/test-*` / `host/test-*` identities; the
retained legacy `machine/retained` is synthetic. Folder IDs use the fixed
zero-prefixed UUID family. New case names use `synthetic-case-N`, labels use
`Synthetic Folder N` / `Synthetic Resource N`, and revisions use `claim/test-N`.
Unknown schema keywords fail closed, so extending the behavior format requires
updating its validator and public-identity policy together.

The web runner `apps/fractal-web/src/folders/sidebarBehavior.unit.test.ts` loads
these vectors through `scripts/sidebar-behavior-fixtures.mjs`, which runs the
shared schema/public-data gate before returning a fixture. It exercises the
production arrangement conversion, sidebar adapter and optimistic editor.
Its explicit divergence table pins both golden and native outcomes where
legacy web catalog-seat visibility, drop targets or Unfiled ordering differ;
no dormant seats are synthesized to make the goldens pass. The pure semantic
rollup remains covered by the shared-view runner, not the web adapter.
Run the focused web suite in the web dev shell with
`CI=1 pnpm --dir apps/fractal-web exec vitest run src/folders/sidebarBehavior.unit.test.ts`.

From the repository root with Node 24 and Corepack (or `nix develop .#web`):

```sh
corepack enable # Outside the Nix web shell
pnpm install --frozen-lockfile
node scripts/check-sidebar-fixtures.mjs
pnpm --filter @fractal/views test
pnpm --filter @fractal/views typecheck
```

The moved tests run without phone dependencies. Transcript tests read the original
`fixtures/clients/transcripts/*.json` and compare with the Rust model's expected
entries. Signing tests verify `fixtures/clients/device-signing-v1.json`. Fixtures are
shared, never copied into this package. CI also typechecks and tests the phone against
the package. The root frozen install covers every workspace package; `bash scripts/ci-typescript-client`
runs the combined checks.

### Quick access

`quickAccess.ts` selects the discovered person's reserved Pins arrangement and
projects its server-ordered memberships as navigation shortcuts. Supported stable
destinations are agents, missions, documents, glasses, and resources. An agent
shortcut addresses the agent, so opening it follows the current session. Document
shortcuts address their name rather than an immutable version. Inbox items and
session/PTY identities are not shortcut targets. Missing local inventory does not
remove a membership or prove that opening its stable destination will fail.

`planQuickAccessPlace` plans an insertion or reorder before an observed member,
with `null` meaning the end. The host supplies its canonical fractional-key
function. Equal concurrent keys or an exhausted bounded gap can be rebalanced in
one atomic edit while retaining the observed order. A missing anchor or invalid
collection produces a refusal without an edit.

`quickAccessWrite.ts` is the internal delivery engine for one immutable compound
edit. It retains the original gateway, request bodies, identities and fences in
memory after an unconfirmed response. An explicit identical check is required;
reads never settle ambiguous writes. Only a matching completed action receipt
settles a replayed stage. A safe canonical `arrangement-exists` response, including on an explicit
create check, starts validation of the exact live owner/v2 identity. This satisfies
the shared-container precondition without proving that our CREATE was applied. A failed adoption read retains
validation; explicit recovery repeats only GET, then sends the original membership.
Unknown or replay-unsafe errors never start adoption. Incompatible validation
stays locked with an explicit reason; it never recreates the reserved identity. The shared follow below owns its read lifecycle and explicit delivery checks.


`followQuickAccess` follows the exact reserved root Pins arrangement and all
membership pages. Whole-container invalidation includes changes outside the first
window. It is transport-only, without Effect schemas or native platform APIs.
The host supplies immutable action IDs, keys and fences. Creation requires distinct
create/member identities and explicit coordinated `v2ReadersReady`; server support
alone does not establish that all readers were upgraded. Retirement is terminal.

Refresh explicitly reopens an interrupted stream without polling or edit replay.
Retain the same follow instance across ordinary disconnects. Its `unconfirmed`
phase blocks fresh edits through reads and reconnects. **Check delivery** calls
`checkEdit()` with the same original request stage; matching completed receipts
settle ambiguous writes. Safe canonical create conflicts, including on explicit create replay, may
validate the exact live owner/v2 identity without proving the original CREATE applied; failed validation retains an explicit
GET-only recovery step. Other statuses do not guarantee nonapplication. A replay
refusal cannot settle an originally ambiguous attempt. Recovery is in memory only;
closing the follow or exiting the process discards it.

Quick access is one flat root collection. The planner refuses folder memberships.
A rebalance preserves the observed order; concurrent edits may require a later reorder.
