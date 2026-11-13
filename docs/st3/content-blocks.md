# Rich content blocks

This document specifies shared rich content for structured person requests and updates. It builds on the [structured-request contract](mission-graph-runtime.md#structured-requests), [client v0 contract](client-v0/README.md), [attachment contract](attachments.md), and [rolling compatibility requirements R49](compatibility.md#required-rolling-window).

## Status

Draft. This is a design contract, not an assertion that these capabilities are available.

## Scope

Defines authored, stored and projected content, exact text checks, content-bound approvals, compatibility, and rendering. The initial content profile (v1) has `text_change` and `image`, on `StructuredRequest` and `PersonUpdate`. Updates remain non-answerable information cleared by the existing read action. Blocks do not change that action into an approval.

Does not define automatic edits, commits, post-commit verification, a layout language, arbitrary HTML, or a replacement for harness `TimelineBlock`. Resource cards and sandboxed apps have separate extension boundaries below. Existing question, summary, reasons, subjects, answers, about and speech semantics remain in force; blocks are not answer buttons or notification speech.

## Model and ownership

```text
sender intent -> strict admission -> immutable canonical blocks in carrier claim
                       |                         |
                 upload completion        authorized revision checks
                                                 |
client renderer <- negotiated ContentBlockView <- derived observations
       |
native answer + rendered digest + client-reported state -> durable approval record
```

`blocks[]` is a flat, ordered array. A block has a carrier-local stable `id`, `kind`, per-kind integer `version`, required plain-text `fallback`, and typed `data`. An ID is not an array or subject index. Preserve IDs within an unchanged request; a changed request receives a new authored-content digest. Subjects remain relational metadata for navigation; a block target is self-contained. Conflicting pins for the same reviewed target in subjects and blocks are refused.

Fractal owns the repository-local wire namespace. Field names and kinds are lowercase snake_case; capability names are lowercase kebab-case. IDs match `^[a-z][a-z0-9_-]{0,47}$`, are unique per carrier, and have no global meaning. Kind names match `^[a-z][a-z0-9_]{0,47}$`; versions are positive integers. The public profile admits only the pairs `text_change/1` and `image/1`. A new kind is assigned centrally in this contract; private or interoperable adapters must negotiate a separately specified kind rather than insert unregistered names. `kind` selects semantics; MIME types describe image bytes, not a second block discriminator.

| Example | Interpretation |
| --- | --- |
| `{"id":"wording-r06","kind":"text_change","version":1,...}` | Valid envelope for a known pair, subject to data validation |
| `{"id":"panel","kind":"image","version":1,...}` | Valid image envelope; upload/blob protocol remains unchanged |
| `{"id":"card","kind":"resource","version":2,...}` | Unknown read pair: fallback, no execution; refused on v1 input |
| `{"id":"private","kind":"vendor_widget","version":1,...}` | Unregistered private kind: refused on input; unknown-pair behavior on read |
| `{"id":"Bad ID","kind":"TextChange","version":0,...}` | Invalid envelope |

## Input, stored and view schemas

The following structural schemas use `?` for optional fields and `T[]` for arrays. Objects are closed unless explicitly described as unknown read envelopes. All cross-field constraints and limits below are normative. Shared `$defs.ContentBlockInput`, `$defs.ContentBlockStored` and `$defs.ContentBlockView` belong in the client-v0 canonical schema and generated Rust, Swift and TypeScript models; application-local copies are not wire authorities.

```typescript
type Block<K, D> = {
  id: string; kind: K; version: 1; fallback: string; data: D
}
type Target = { repo: string; path: string; revision: string }
type Hunk = { start_byte: number; before: string; after: string; label?: string }
type TextChange = { label: string; target: Target; hunks: Hunk[] }
type UploadImage = {
  source: { type: "upload"; blob: string; media_type: ImageMedia; name?: string }
}
type ImageMedia = "image/png" | "image/jpeg" | "image/gif" | "image/webp"
type ContentBlockInput = Block<"text_change", TextChange> | Block<"image", UploadImage>
// Optional on StructuredRequest and PersonUpdate; omitted when empty:
// blocks?: ContentBlockInput[]

type StoredImage = {
  source: {
    type: "blob"; sha256: string; media_type: ImageMedia;
    name?: string; size: number; origin: string
  }
}
type ContentBlockStored = Block<"text_change", TextChange> | Block<"image", StoredImage>
// StoredImage uses the existing completed MessageAttachment descriptor fields.
// Its authorized read view additionally includes blob: "blob/<sha256>".

type Check = {
  checked_at: string;
  base_revision: string; observed_revision?: string; observed_file_sha256?: string
} & (
  { state: "applies"; reason: "exact_base_and_current_match" } |
  { state: "stale"; reason: "base_mismatch" | "file_changed" | "path_missing" } |
  { state: "unverifiable"; reason:
      "inaccessible_target" | "unresolved_revision" | "unsupported_target" |
      "non_text" | "file_too_large" | "origin_unavailable" | "ambiguous_authority" }
)
type ContentBlockView =
  | (Block<"text_change", TextChange> & { check: Check })
  | Block<"image", StoredImage> // source adds the authorized blob reference
  | UnknownBlockView
// UnknownBlockView preserves id, kind, positive version, fallback and opaque data.
// It excludes the two known kind/version pairs; it cannot swallow malformed known data.
```

`repo` is an existing authorized canonical Fractal repository identity, not a filesystem path or arbitrary fetch URL. `revision` is a full immutable Git commit object ID in that repository's object format. CLI shorthand must resolve before canonical submission; unresolved full pins can still be displayed as unverifiable. `path` is a UTF-8, repository-relative slash-separated path with no empty, `.` or `..` component, leading slash, backslash or NUL. Resolve Git tree blobs, not checkout symlinks or submodule destinations. Never follow a target outside repository authority.

Hunks address the original pinned file. `start_byte` is a nonnegative safe integer at a UTF-8 boundary; its end is start plus the UTF-8 byte length of `before`. Hunks are sorted, non-overlapping and have distinct starting offsets, including empty-before insertions. An insertion may be at EOF. Labels explain, never locate, text. Both sides cannot be empty or equal. Exact UTF-8 bytes, including line endings and whitespace, are authoritative; no fuzzy search, normalization or patch repair. Out-of-file offsets or before mismatches in readable text produce `stale/base_mismatch`, not a substituted edit. Ill-formed types, ordering or arithmetic are admission errors.

Sender input cannot supply `check`, observations, completed blob descriptors, provider facts or arbitrary server metadata. The daemon completes held uploads after ownership and magic-byte checks, canonicalizes once, and stores only immutable intent/completed image references in `work.person-asked`. It never embeds binary bytes. Derived checks are not authored fields and do not rewrite signed historical claims. Timestamps are UTC RFC 3339 strings; digests are lowercase hex SHA-256.

### Secret hygiene and disclosure

The existing shared text policy is `fractal_runtime::transcript::normalize_conversation_text` in `crates/fractal-runtime/src/transcript.rs`, used by native driver content publication in `crates/fractal-drivers/src/harness_timeline.rs` and by terminal exit evidence. It detects sensitive token/credential forms and reports redacted byte counts while returning normalized, bounded text. This is not an established universal scanner on ask prose or arbitrary claim admission; the current structured-request parser and person-ask persistence path do not call it.

At admission, before durable persistence or external disclosure, apply that same detector to every `text_change` hunk's `before` and `after`, every block fallback, and all block/hunk labels. On a match, refuse the ask/update with a clear field-specific secret-detected validation error that does not echo the matched text. Never store a redacted or truncated copy: the authored hunks must match file bytes exactly and approval must retain exactly those hunks. P2 must expose/reuse one shared detection implementation from the existing transcript policy if its normalizing API is not suitable; do not create a per-feature regex/pattern list. Scan the complete admitted fields, without adopting the normalizer's truncation as a way to omit unscanned text.

Authorized pinned/current-file reads may inspect bounded file bytes internally for checks but must not expose file content beyond the authored hunk spans in projections, errors, logs or retained approval evidence. Do not publish whole resolved files or extra context from those reads. Retained answer records, including exact hunks and check provenance, follow the carrier's retention policy; a content digest is not permission to retain a separate perpetual copy.

Read decoders dispatch on the pair `(kind, version)`. Unknown pairs retain the common envelope and show fallback with “Rich content not rendered”; they never discard the whole structured request. Malformed known data is visibly unavailable and cannot be marked checked or accepted. Unknown data is untrusted, non-executable and available for inspection only under carrier disclosure permissions.

## Limits and admission

| Boundary | Limit |
| --- | --- |
| Blocks per carrier | 7 total; at most 3 non-image and 4 image blocks |
| Text-change hunks | 1–8 per block; at most 8 across the carrier |
| Before/after | At most 2,000 Unicode scalar values per side; 8,000 total across the carrier |
| Fallback | 1–2,000 Unicode scalar values, non-whitespace plain text |
| Labels, repo, path, revision | Nonempty, at most 600 Unicode scalar values each |
| Encoded canonical blocks | At most 64 KiB stored per carrier, including completed descriptors |
| Projected blocks | At most 128 KiB per carrier |
| Checked target file | At most 1 MiB UTF-8 text, with no NUL bytes |
| Images | Existing four-image, 10 MiB/image, MIME, filename, actor quota and retention limits in attachments.md |

Existing prose-field limits apply independently. Validate encoded size after image completion, before persistence. Reject oversized authoring with a field-specific validation error; never truncate a proposed hunk. A known block that cannot fit a bounded read projection is visibly unavailable and not accept-capable, not silently shortened. Future kinds consume the same non-image and byte budgets. Required fallbacks summarize/identify content; they need not duplicate exact diff bytes.

## Daemon text checks

```text
resolve authorized pin/current target
  | unavailable/unsupported/non-text/oversize -> unverifiable
  | resolved revision lacks path            -> stale/path_missing
  | before spans mismatch pinned text       -> stale/base_mismatch
  | current file differs from pinned file   -> stale/file_changed
  ` exact pinned spans + unchanged file     -> applies
```

1. Resolve the pinned commit and authorized current repository/worktree target using existing document/repository access. Current means the target's active worktree bytes, including uncommitted edits when a worktree exists; otherwise the authorized current ref's blob. Record the observed commit when available and file-byte digest. Never claim current-file verification from the pinned blob alone.
2. Read bounded text and compare each before span at its exact offset. A demonstrably absent path in an accessible resolved tree is stale; an inaccessible or unresolved tree is unverifiable. If multiple failures occur, inaccessible required evidence wins, then path_missing, base_mismatch, file_changed. Never leak inaccessible target details in a reason.
3. Require the current target file bytes to equal the pinned file bytes. A different target file is stale even if the individual before spans still match. Commits touching only other files do not stale the block. Record the check time, base and available current evidence. `applies` means exact textual applicability at that check, not semantic safety or that anything has been applied.
4. Recompute on target-file/current-ref changes using repository observation invalidation and publish an updated attention view. Checks run outside the pure `client_view` transformation and outside per-card render loops. Derived observations are keyed by authored-content digest, target identity and observed file identity; unavailable reads have explicit states, not synthetic success. No polling loop or arbitrary network fetch is introduced by list rendering.

Each v1 text-change view supplies a check; a legacy/temporarily missing check is displayed as “Not checked”, never applies. Stale and unverifiable remain answerable on a capable renderer, with an explicit warning visible both on the collapsed header and at acceptance. An offline attention card remains subject to the existing no-action rule.

## Approval record and answer fencing

```text
view(authored-content digest + checks)
  -> render exact hunks and provenance
  -> native accept carries rendered digest + client-reported shown states
  -> compare digest, check current targets and append approval to existing answer
```

An accept is the native decision answer whose declared outcome is `accept`; other answers and update read actions do not approve hunks. Accept approves all `text_change` blocks on that request, not an implicit selected subset. It authorizes the agent to proceed and records the exact proposal. It does not edit a file, commit, verify a later commit, or promise that approved hunks eventually land.

The negotiated answer input adds `content_review: {content_digest, client_reported_checks: [{block_id, state}]}`. The digest identifies the canonical stored `blocks` array rendered by the client: SHA-256 over RFC 8785 canonical JSON, excluding checks and other ephemeral view fields. Each text-change block has one client-reported state: `applies`, `stale`, `unverifiable`, or `not_checked`. The daemon validates existing source episode/person authority and refuses the answer with stale-fence behavior if the supplied digest does not match the current request. A changed request requires a fresh view. Client-reported states are unverified statements about what the client showed, not daemon evidence of applicability or proof of inspection.

A successful accept atomically appends an approval alongside existing answer history with: content digest, every text-change block ID, exact canonical target and hunks, the daemon's own check result at answer time (state, observed revision when available, and time), and the state the client reports it showed, explicitly labelled `client_reported`. The daemon checks current targets using the same semantics as the read check; stale and unverifiable remain answerable with warning. Preserve both states if the file changes between display and acceptance; do not relabel the client-reported state as verified. There is no observation-reference protocol, validation of which snapshot the client displayed, or retained observation store. Ordinary answer idempotency prevents duplicate records.

An accept without the rendered-content digest is refused for a request containing text changes. A client without the exact kind/version renderer cannot accept that request through UI, CLI, delegated or alternate action paths; it may read fallback, decline or request changes under existing rules. Renderer support is a client conformance obligation, not a new grant of authority or proof the person read every line. Historical answers without this binding remain ordinary historical answers. Never fabricate rich-review bindings for old answers, delegated answers or timeouts; a delegated acceptance must supply the same explicit digest and labelled client report under its existing authorization.

## Capabilities, images and compatibility

| Axis | Required negotiated support |
| --- | --- |
| Admission | `content-blocks` v1, carrier `person-ask-blocks` or `person-update-blocks` v1, and each emitted `content-block-text-change` / `content-block-image` v1 |
| Read shape | `content-blocks` v1 for the shared unknown-envelope decoder; carrier support selects block-aware projection |
| Rich accept | `person-answer-content-review` v1 plus local `text_change/1` renderer support and the rendered authored-content digest |

These repository-local names follow the serving daemon's existing capability version/state contract: only `granted` support permits a new field. Keep `StructuredRequest.version = 1`; unknown input fields are not safely ignored by old strict decoders. Unknown kinds/versions are refused on writes. Breaking kind semantics require a newly negotiated per-kind version. Do not ship the prototype-only `previews[]` name or `provenance`/`resolved` wire sketch.

Enable durable block writes only after every active member advertises compatible retention, replay and projection behavior under R49. Verify N/N-1 replay/restart/forwarding, not only producer negotiation. Where the active floor is not met, withhold admission capabilities. Do not drop unknown optional claim fields, the whole structured request, or its named answers during retention or replay. All four clients (TUI, web, CLI and iOS) implement v1 text-change rendering before rich review is enabled; fallback-only acceptance is not a rollout stage.

Image compatibility is a bounded adapter, not two editable models:

1. Keep `--attach`, existing upload/fetch routes and blob retention. A block-capable CLI turns uploaded ask images into image blocks. Use supplied alt text as fallback; without it identify filename/media type honestly, never invent a visual description.
2. Accept legacy `attachments[]` or new image blocks, never both image authoring forms. Legacy attachments may accompany text-change blocks and normalize once at admission. Canonical new storage uses blocks only. Historical claims remain byte-for-byte intact; derive image IDs deterministically from attachment order and hash.
3. Block-aware projection emits image blocks without duplicate attachments. Legacy projection synthesizes the exact existing `attachments[]` descriptors and omits blocks. It must withhold accept for text-change requests and expose an upgrade-required explanation rather than silently authorize unseen rich content.
4. Every blob authorization/read, retention reference and delivery traversal that inspected attachments must discover canonical image sources before activation. Knowledge of a hash is never read authority; carrier permissions, origin relay, 512 KiB chunks, hash verification, seven-day retention and explicit expiry remain unchanged. Expired images retain fallback/reference; no silent re-upload or new blob store.
5. Both asks and updates admit the shared blocks through their distinct carrier capability. This explicitly extends the older update-without-images rule. Other carriers require their own domain validation/capability. Retain historical image decoding for as long as historical claims remain readable; remove live legacy adapters only outside their supported compatibility window.

## Rendering contract

All clients preserve block order directly after the question (after the update summary when no question), before reasons/“more”. Render the rich representation or fallback, not both. Show target pin, check state and timestamp/provenance with text changes, including collapsed state. Preserve exact selectable/copyable before and after text; visual folding never changes the approved payload. Images reuse authorized blob behavior, alt text and explicit unavailable states; web must render ask images too.

| Client | Text-change behavior |
| --- | --- |
| TUI | Collapsed one-row summary by default; `d` opens/closes. Compact word diff uses a bar gutter, folded unchanged context, underlined green insertions and struck-through red deletions. `n`/`p` navigate hunks; `+`/`-` adjust context. Hide line numbers for prose. For code at card widths ≥60 columns, use delta-style removed/added rows with word emphasis and line numbers; narrower cards use compact wrapping. Add-only hunks remain single-column. Layout uses card width, not terminal width. Opening a preview on a narrow card gives it the full available terminal content width, hiding the list as needed; restore the prior layout on close. Headers wrap or drop secondary location detail rather than cut off check state. `NO_COLOR`/monochrome uses `[-removed-]{+added+}`, not colour-only marks. |
| Web | Eager lightweight disclosure header; lazy-loaded `@pierre/diffs` body only on expansion. Unified word diff, wrapping and folded context by default; optional split view. Use Pierre as is: a pure insertion within a paragraph may show the full −/+ paragraph pair; do not create an insertion-only custom renderer. Word/tokenization limits cover the full supported hunk text. Share `@pierre/highlights` 0.4.0, upgraded in its own preceding change. Keep Pierre outside the eager attention/detail graph. The production CSP proof must pass; Storybook's development CSP is not proof. |
| iOS | Native exact before/after renderer with word emphasis for prose, distinct removed/added code rows, folding/disclosure, check warning and accessible labels. Native image rendering reuses existing blob authorization. No fallback-only acceptance and no embedded web renderer prerequisite. |
| CLI | Human read-back prints target/pin, check/time and every exact before/after hunk with removal/addition markers; never summary alone. Machine-readable output preserves blocks/checks and approved records. `NO_COLOR` has unambiguous text markers. Native accept passes the digest of the content rendered and labels its reported shown check states as client-reported; the daemon separately records its answer-time checks. |

### Diff colour

Additions are industry-standard green; deletions are red in every client and both supported colour schemes. Non-colour markers/emphasis remain required. This is a diff-only exception to the older “no green” palette constraint: update `diff.added`, addition washes/emphasis, the palette test, `DiffTint` story and README design note together. Do not change success, attention or other status colours. Existing DiffPanel and new Pierre styling use the same semantic diff tokens; palette assertions must distinguish diff additions from unrelated status tokens.

## Extension profiles

| Profile | Contract boundary |
| --- | --- |
| v2: live resource cards | Typed GitHub PR/issue references first, resolved through authorized existing provider routes. View carries normalized static card, explicit result state, checked-at time and observed provider revision. Refresh derives new view data without mutating authored pins or approvals. Approval remains bound to the reviewed immutable revision, never whichever revision is live when an action arrives. A moved PR head requires renewed review to authorize that head. |
| v2: static links | After GitHub, allowlisted providers only. Typed references, bounded async resolution and sanitized static metadata/image refs; never arbitrary fetch, provider HTML or browser-side credentials. Revalidate authorization on disclosure. Prevent private/loopback/link-local/metadata destinations and redirect/DNS bypasses; bound bytes, time and media types. No general resolver plugin framework is required for one provider. |
| v3: apps | Follow MCP Apps semantics: registered `ui://` template identity, immutable version/digest, bounded public data, separate host-only context, negotiated host bridge and sandbox. No HTML string in a content block, main-origin script privileges, session/provider tokens, raw file handles or ambient native authority. Consequential actions use host-mediated existing authorized commands and confirmation/audit, never an app-owned ask accept. Web sandbox/CSP/origin/message validation and lifecycle/resource limits require their own specification. TUI/CLI use fallback plus safe external open; iOS embedding requires a separate reviewed host contract. |

Messages, notices and transcript adapters may reuse these definitions through explicit carrier adoption. Do not reinterpret harness tool/reasoning/source events as authored content, widen assistant disclosure grants, or put rich detail in push/speech transport. New profiles have independent capabilities; reserving an extension point does not advertise implementation.

## Conformance

Contract fixtures and focused tests cover strict input/forged metadata, unknown read pairs versus malformed known data, UTF-8 boundaries, repeated text and empty-before insertions, overlap/size refusal, exact and stale/unverifiable checks, uncommitted file changes, unrelated commits, unavailable/missing paths, image ownership/expiry/carrier access, legacy normalization without duplicates, and N/N-1 retention/replay. Answer tests prove exact hunks, the daemon's answer-time checks and separately labelled client-reported shown states are retained; changed-content digests fail; stale stays answerable with warning; fallback-only accept is refused; no rich bindings are fabricated for old/delegated answers or timeouts; and no apply/post-commit action occurs. Each client covers prose insertion/replacement/deletion, code, narrow layout, unknown fallback, provenance and no-colour/accessibility. Web additionally requires lazy graph, palette and production CSP proofs.
