# Typed custom subjects

A kind registration adds field validation and a bounded, data-only projection to the existing
`custom/NAMESPACE/NAME` subjects and `custom.NAMESPACE.NAME` claims. The graph stores the
manifest, facts and answers. The daemon interprets a descriptor; it runs no extension code.

## Register and answer a question

The complete example manifest is [custom-review.json](../../examples/fractal/custom-review.json).
All identities below are invented. Run the commands against your own daemon:

```sh
fractal schema register examples/fractal/custom-review.json --as agent/garden/seed
fractal claim custom/garden/review/v1/review-001 custom.garden.review.v1.requested \
  --actor agent/garden/seed \
  --field 'title=Retain the seed history?' \
  --field 'detail=Choose Keep or Discard.' --field recipient=person/lichen
fractal subject show custom/garden/review/v1/review-001
fractal alerts ls --as person/lichen
```

The claim pins its registration hash automatically. One card appears for Lichen. The terminal UI displays
its declared fields and source ID. Open the reply box and enter `keep`, or enter a JSON object
such as `{"selection":"keep","text":"Retain the provenance."}`. The generic client action
is `custom.reply`; its parameters come from the card's `action_parameters` and `custom_form`.
It requires `control.attention` and the addressed person's authority.

For a CLI reply, copy the `registration`, `revision` and attention `episode` from `subject show`,
write the reply fields to a JSON file, and run:

```sh
fractal subject reply custom/garden/review/v1/review-001 \
  --registration HASH --revision REVISION --episode EPISODE \
  --fields-file reply.json --idempotency-key garden-review-answer-001 --as person/lichen
```

The answer records Lichen's actor and the immutable request claim ID, removes the card, and
survives restart and replication. Stale card parameters write nothing. An exact retry returns
its previous claim; changing the request under the same key is rejected. Concurrent answers on
isolated members remain competing immutable facts, visible as `reply_conflicts` and in history.

## Manifest contract

`fractal schema registrations` lists selected registrations; `fractal schema registration KIND@HASH`
reads an exact registration. Registration is a trusted local configuration operation and uses
existing rules. Paired devices cannot register kinds. Bound agent sockets cannot name another
actor. Human replies still require the fixed recipient; free mode does not let an agent answer
as a person.

A manifest declares `kind`, `namespace`, positive `version`, `language: custom-projection.v1`,
exclusive `subject_prefix`, `creation_kind`, `claims`, `slots`, `fields` and optional `attention`.
The registry subject for `garden.review` version 1 is `custom/st3-kinds/garden/review/v1`.
`custom.st3-kinds.registered` records an exact immutable manifest document reference. Ordinary
raw writes cannot publish registry records. The internal `client` and `st3-kinds` namespaces
are reserved. Unregistered custom subjects retain their existing open-field behavior.

A kind/version is immutable. Identical registration is idempotent; changes require another
version and exclusive prefix. Registration cannot capture a prefix with existing untyped
facts. Offline competing registrations or overlapping prefixes disable the affected views.
Unsupported or unavailable manifests stay in raw history, with pinned sources pending until
supported manifest dependencies arrive.

Each claim schema declares an authority (`creator`, `owner`, or `recipient`) and fields using
existing schema types. Creation fixes the owner actor and addressed person. Only creation
claims have creator authority. Owner claims require that original actor; recipient claims
require the original person. Raw `fractal claim` writes receive the same validation as generic
replies. Fields support required/optional values, scalar types, enum strings, arrays with
`min_items`/`max_items`, subject reference families, immutable documents (`document: true`)
and immutable claim IDs (`claim: true`). `additional_fields` defaults to false. Document/claim
reference flags require string fields. String fields also support `pattern`: a regex matched
against the entire string. Patterns are limited to 512 bytes, nesting depth 16 and 256 KiB
compiled size/cache; invalid or over-complex patterns fail registration. `max_bytes` bounds
the JSON-encoded value, including string quotes and escapes. `_registration` and `_basis` are
reserved metadata.

Slots select `first` or `last` in canonical order for an exact declared claim kind. Expressions
copy a field (`op: field`, `slot`, `field`), an actor or claim ID (`actor`/`claim-id`, `slot`), or a
scalar constant (`constant`, `value`). Predicates are `exists`, `eq`, `all`, `any` and `not`.
Attention declares `when`, `recipient_field`, `title`, `detail`, `episode`, and a `reply` with a
claim kind, user-input fields, and expression bindings. The recipient is a required person
reference in the creation schema. Reply kinds must require recipient authority.

Limits include a 64 KiB manifest and claim field object, 32 slots/claim kinds, 64 output fields,
128 predicate nodes, depth 16, a 64 KiB projected row, and 256 basis dependencies. Invalid
facts or outputs disable only their source. Each source evaluates inside a savepoint; genuine
storage errors remain storage errors. No custom SQL, callbacks, loops, network requests,
retention rules, native actions, or contributed UI are accepted.

## Shared reference carriers

Register [image-file-reference.json](../../examples/fractal/image-file-reference.json) and
[onepassword-field-reference.json](../../examples/fractal/onepassword-field-reference.json)
to use the same `reference.image-file` and `reference.onepassword-field` v1 kinds across fleets:

```sh
fractal schema register examples/fractal/image-file-reference.json --as agent/garden/seed
fractal schema register examples/fractal/onepassword-field-reference.json --as agent/garden/seed
fractal claim custom/reference/image-file/v1/seed-picture custom.reference.image-file.v1.created \
  --actor agent/garden/seed --field 'file=file/alder:/srv/garden/seed.png' \
  --field content_hash=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --field media_type=image/png
fractal claim custom/reference/onepassword-field/v1/garden-token custom.reference.onepassword-field.v1.created \
  --actor agent/garden/seed --field locator=op://garden-vault/seed-service/token
```

The image carrier stores a validated `file/HOST:/ABSOLUTE_PATH` subject reference (native
subject limit 512 bytes, field limit 1024 JSON bytes) and a lowercase 64-digit SHA-256
content hash (66 JSON bytes). Neither `file.observed` nor `filesystem.file` declares a media
type, so the carrier permits an
optional `image/...` media type (≤128 JSON bytes, without parameters). The reference does not
read, embed, replicate or verify the image bytes; a consumer must check the file's hash.

The 1Password carrier accepts only an exact `op://vault/item/field` locator (≤1024 JSON
bytes). Each of its three components starts with an ASCII letter or digit and contains only
ASCII letters, digits, `_` or `-`. Use IDs or simple names; v1 deliberately excludes spaces,
section-qualified fields, query strings, fragments and encoded components. Fractal never resolves
the locator. The schema has no secret-value, document, content or free-text field, and rejects
extra fields. A locator's identifiers are graph data visible to graph readers; use invented
names in public examples and keep credentials in the [sekrets](sekrets.md) workflow.

Both carriers project their actual creator as `owner` and select its first creation fact.
Only that actor can append more creation facts; later ones do not replace the selected
reference. There is no update or reply kind. Use a new subject for a changed reference.
Subject suffixes are caller-chosen labels, not an identity binding: consumers must check
the projected owner against their expected writer. Custom facts do not satisfy native
approval gates.

## Reads, replication and derived state

`GET /v1/client/custom-subjects` pages the source index, with optional `kind`, `version`, `cursor` and
`limit`. `GET /v1/client/custom-subjects/{encoded-id}` reads a source. Both require
`read.projections`; the source is an extensible `custom-subject` resource, preserving old
client unknown-resource decoding. It includes the pinned registration, source revision,
selected fields, provenance and `state`: `ready`, `stale`, `pending-dependencies`, `conflict`,
or `invalid`. Custom attention adds optional metadata to the existing attention resource.
Clients send `x-st3-features: custom-subjects.v1` to receive namespaced attention and
`custom.reply`.

Manifest documents, registry claims and typed facts use the existing signed envelope and blob
replication. The three shared custom tables are rebuildable caches and participate in projection
digests. Rows use canonical claim identities, never local arrival indexes. Checkpoints keep
custom history and its referenced blobs. Client reads page indexes and never fold custom history.

An external domain tool can append a derived claim with `_basis`, an array of
`{"subject":"custom/...","kinds":["custom.domain.input"],"revision":"HASH"}`. Obtain the exact
input-set token with `fractal subject basis SUBJECT --kind custom.domain.input` (or the equivalent
`/v1/custom/basis` read). Another matching input claim immediately makes the derived view stale
and removes its card. The tool recomputes and appends a fresh derived claim. Filters exclude
the output kind, dependency cycles are invalid, and bounds are enforced.

`GET /v1/claims` lists claims in store-index order. `kind` keeps one kind, `subject` one exact
subject and `subject_prefix` every subject that starts with those characters, such as
`custom/acme/change/` (end the prefix with `/`, or `custom/acme/change` also matches
`custom/acme/changes/x`). `kind` combines with either subject filter; `subject` and
`subject_prefix` together are refused with `invalid-claims-query`, as is a prefix that is empty,
not ASCII, or holds `*`, `?` or `[`. Page with `after_index`, `before_index`, `order` (`asc` or
`desc`) and `limit` (1 to 500): the response is `{"claims": [...], "next_cursor": N}`, and
`next_cursor` is the next page's `after_index` ascending or `before_index` descending. Each filter
reads through a claims index. `fractal claims ls --kind KIND --subject-prefix PREFIX` prints one
line per claim and the flag that reads the next page; `--json` prints the page.

## Decision trees

[decision-tree.json](../../examples/fractal/decision-tree.json) registers `decision.tree` version 1
for the external decision-tree tool. Register it once per fleet:

```sh
fractal schema register examples/fractal/decision-tree.json --as agent/example/decisions
```

Each logical tree is one subject, `custom/decision/tree/v1/<seat>`, named after the durable seat
ID without its `agent/` family, so a tree has one revision to fence. The tool writes these kinds,
all `custom.decision.tree.v1.*`:

| Kind | Authority | Fields |
| --- | --- | --- |
| `opened` | creator | `seat` (agent reference), `recipient` (person reference), optional `legacy_trees` |
| `requested` | owner | `question`, `kind` (`blocker`/`refinement`), hash-pinned `body` document; optional `about`, `parent` (claim), `applies_when` (array of `{decision, option}`), `q`, `legacy_id` |
| `answered` | recipient | `request` (claim), `selection` (array, zero or more option keys); optional `text` reframe, `supersedes` (claim), `legacy_id` |
| `assumed` | owner | `request` (claim), `text`; optional `supersedes` (claim), `legacy_id` |
| `promoted` | owner | `request` (claim), `target`; optional `text`, `legacy_id` |
| `status` | owner | `state` (`pending`/`clear`), card `title` and `detail`; optional `request` (claim), `q`, `pending` |
| `damaged` | owner | `raw` (hash-pinned document of the unreadable bytes), `records`, `imported`, `malformed`; optional `source`, `detail` |

`opened` fixes the tree's owner (its actor) and its one addressed person. Only that owner asks,
assumes, promotes, records damage and derives status; only that person answers. Assumptions are
owner facts, never person responses.

Registration cannot tie `seat` to the writer or to the subject, so any agent can write the first
`opened` on a tree and name any seat and person. The source's `owner` output is that writer.
A card shows only when `owner` equals `seat`, so a forged tree raises no card. The tool must
reject a tree whose `owner` differs from `seat`, or whose subject is not
`custom/decision/tree/v1/` plus `seat` without `agent/`, and must read nothing from it.
Version 1 cannot reassign an owner: if another agent opened the seat's tree first, every owner
write by the real seat is refused. A retired or renamed seat likewise strands its tree, which
keeps its history but takes no new owner writes.

`status` is the tool's derived view. Write it with one `_basis` entry for the tree subject
covering every other kind, from `fractal subject basis custom/decision/tree/v1/<seat> --kind
custom.decision.tree.v1.opened --kind ...requested --kind ...answered --kind ...assumed
--kind ...promoted --kind ...damaged`. A `pending` status that names a `request` shows one card:
the attention predicate compares `status.request` with itself because `eq` is false when either
side is missing, which makes it a "field is present" test. The status claim ID is the episode,
so a revived question gets a fresh episode. The card's reply writes `answered` with that request
bound. Any new raw fact makes the status stale and removes the card until the tool writes a
fresh one.

Claim-reference fields (`request`, `parent`, `supersedes`) are checked only to name an existing
claim. The daemon does not check that it is a `requested` claim, an answer or assumption, or on
the same tree. The tool must check those, and must ignore any fact that fails them.

`q` and `legacy_id` are stored but not allocated or checked. Handle allocation, guards,
supersession forks and option grounding stay in the tool. A `damaged` receipt shows in the
source's `damage*` fields even when a tree has no readable requests, so a failed import does
not read as an empty tree. Writes need a running daemon; nothing is spooled offline. Custom
claims do not block native work or confer native approval; a tool that needs native blocking
separately uses authorized work asks.

### Version 2: captured history and deterministic reads

[decision-tree-v2.json](../../examples/fractal/decision-tree-v2.json) is a separate, immutable
`decision.tree` version 2 registration. Version 1 and its registered bytes remain unchanged.
Register v2 through the same schema mechanism:

```sh
fractal schema register examples/fractal/decision-tree-v2.json --as agent/example/decisions
```

V2 subjects use `custom/decision/tree/v2/<seat>` and claim kinds
`custom.decision.tree.v2.*`. They retain the seven v1 kinds and their authority and fields,
with one addition: `assumed.selection` is an optional array of up to 64 option strings.
An assumption without an explicit selection cannot satisfy an option guard. The v1 namespace
is not an alias for v2; no v1 claim is rewritten or silently migrated.

| Added kind | Authority | Required fields | Optional fields |
| --- | --- | --- | --- |
| `provenance` | owner | `target` (claim reference), `raw` (hash-pinned document), `source_host`, `source_layout`, `source_paths` (array), `legacy_id`, `source_scope`, `written_ms` (tagged object), `actors` (array) | none |
| `withdrawn` | owner/requester | `request` (claim reference), `reason` | `duplicate_of` (claim reference) |
| `recorded` | owner | `request` (claim reference), `recorder` (agent or person reference), `responder` (original string), `quote` (verbatim string) | `selection` (array), `supersedes` (claim reference) |
| `ask_linked` | owner | `request` (claim reference), `ask` (native ask's `step-run` subject reference), `activation` (string) | none |

The authority DSL has only `creator`, `owner`, and `recipient`; it has no general actor-union
or field-equality policy. Because `requested` is owner-only, its requester is necessarily the
owner, so owner authority implements requester-or-owner withdrawal without broadening admission.
The fold matches withdrawals to request claims on this tree. `reason` is required;
`duplicate_of` is an optional claim reference preserving the reported duplicate relationship.

`recorded` is an accountable capture, not an impersonated `answered` claim. `recorder` must
equal the actual writing actor; the DSL only admits the field's reference family, so a mismatch
is a fold defect rather than an admission-time equality check. `responder` preserves the
original spelling instead of resolving or inventing a person identity, and `quote` preserves
the actual response verbatim. Capture does not require that responder to be the addressed
person. Owner-written captures and assumptions never acquire person authority: only a genuine
recipient-authorized `answered` is a person answer. `selection` gives an explicitly captured
option selection; free-form quoted text is retained independently.

`provenance.target` names the typed fact whose original bytes are preserved by `raw`, an
immutable `doc/...@<64-hex-SHA-256>` reference. `source_host` and `source_layout` identify where and
how the source was found; `source_paths` preserves its path spellings. `legacy_id` identifies
the source record, `source_scope` scopes its numeric question handles, `written_ms` preserves
the source timestamp, and `actors` preserves the original actor spellings as strings. These
are evidence, not authority, native claim order, or permission to rewrite a source. The generic
schema bounds arrays but does not validate their members. As with v1 claim references,
existence checks alone do not establish the target kind or same-tree membership.

`written_ms` is exactly `{"known": <integer milliseconds>}` or
`{"unknown": "<nonempty reason>"}`. Unknown time is valid only when `target` names a
same-tree `damaged` claim, never a request or response. The manifest admits an object;
the fold rejects malformed tags and unknown times on non-damaged targets as defects.
Unknown time is never converted to zero or used for chronological ordering.
Every provenance fact's time is validated before indexing by target, so a second provenance
claim for the same target cannot hide an invalid or unauthorized unknown time.

`ask_linked` records a relationship to an existing native work ask and its activation. It does
not open, complete, cancel, approve, or delegate that ask. A decision answer, capture,
assumption, promotion, or withdrawal never bypasses native work authorization.

### Decision fold contract

```sh
fractal subject decisions custom/decision/tree/v2/<seat>
```

This is a read-only domain fold of the subject's effective history, not the latest-slot source
row and not an instruction to append `status`. The reader uses the existing canonical custom
ordering and repaired-record exclusion used by the basis fence; local store indexes are only
arrival cursors, never interpretation precedence. It does not write claims, documents, schema
registrations, or native asks. The generic source row retains its last-fact summaries and
status-driven attention contract; it is not a substitute for folding all requests.

`GET /v1/custom/decision-inputs?subject=<URL-encoded-tree>` returns
`{claims, basis}` in one reader transaction. `claims` contains full claim records, including
verbatim provenance `body.fields`, in canonical effective-history order; repaired originals
are excluded. `basis` is the existing subject/kinds/revision token for those inputs. No
client-side pagination or before/after token race is needed for this snapshot read.
Before materializing history, the same snapshot streams indexed subject rows, stopping as soon
as the effective count exceeds 10,000 claims or stored body bytes exceed 32 MiB (33,554,432
bytes). Repaired rows contribute neither count nor bytes, but a separate work budget refuses
after 20,000 scanned rows including repaired history, even if effective history is small.
Refusal is `decision-inputs-too-large`; history is never silently truncated. There is no
whole-history count/sum aggregate before refusal. The decision reader defines
`DECISION_INPUTS_MAX_CLAIMS`, `DECISION_INPUTS_MAX_BODY_BYTES`, and
`DECISION_INPUTS_MAX_SCANNED_CLAIMS` together. SQLite VM-step regressions cover count,
byte, and repaired-only oversized histories without work growing with their refused tail.
Within those bounds, work and response size scale with the selected tree's history, not a
constant latest-state page. The route-cost inventory records that explicit bounded contract.
The reader requires a ready matching registration, matching registration pins, a valid
creation and source, and exact addressed-recipient answer authority. Invalid, conflicting,
or unregistered histories are rejected rather than classified as approval evidence.

The fold reports duplicate scoped request keys, malformed or cyclic guards, recorder mismatches,
and dangling, cyclic or forked answer supersession as defects. Provenance supplies a request's
`source_scope`; requests
without provenance use the native tree subject as their scope. Source timestamps and numeric
`q` handles do not replace canonical claim ordering.

`requested.applies_when` is a conjunction of guards with this shape:

```json
[
  {"q": 1, "selection": ["ship"]},
  {"q": 2, "selection": ["small"], "source_scope": "legacy/seat-a"}
]
```

The original v1 guard shape `{ "decision": "<request-claim-id>", "option": "<option-key>" }`
is also supported, resolving the request directly rather than through a scoped `q`.

Each guard resolves `q` only within its `source_scope`, defaulting to the requesting question's
own scope only when `source_scope` is omitted. A supplied `source_scope` or direct `decision`
must be a string; malformed values never fall back to another identity. Every guard's structure
is validated before dependency resolution, including guards after an inactive predicate.
A guard is active when its option strings intersect the referenced question's effective
selection; it does not search every imported tree for a matching number. A missing effective
answer, an omitted optional response selection, or a withdrawn dependency leaves the guard
inactive without a defect. Malformed guard predicates remain defects rather than guessed
cross-tree links. Defects disqualify approval only for their affected requests, not unrelated
person-answered requests. A defective request cannot satisfy a guard: direct and transitive
dependents become inactive, with a dependency defect naming the cause. All guard dependencies
are inspected even after a non-matching predicate. Independent branches retain their approvals.
Inactive guarded requests are `moot`. A parent-linked reframe suppresses its
parent only while the child's guards are active; an inactive branch cannot erase a pending
parent. An active child still suppresses its parent when the child itself has been withdrawn.
Guard evaluation uses an explicit traversal stack rather than recursive calls, so deep
dependency chains cannot exhaust the Rust call stack. Supersession traversal indexes claim
IDs and memoizes completed predecessor paths instead of rescanning the chain for every answer.

The fold reports `pending`, `answered`, `withdrawn`, `superseded`, or `moot` for each request.
An assumption sets an independent `assumed` flag but leaves the question pending: an owner
assumption is not a response. Only an unimported `answered` written by a `person/` actor
counts as a person-authority approval gate, and only while the effective status is `answered`;
provenance-marked imported answers and recorded captures remain historical responses, not native approval.

Explicit supersession removes predecessors from effective-head eligibility before chronology
is compared. Imported response and assumption heads use the same
`(written_ms.known, legacy_id, source_scope)` order, with canonical claim order as the final
tie-break. Unknown damaged times do not participate. The imported head is then merged with
the newest native head using canonical claim order, giving a total order for mixed history.
Native-only history uses canonical claim order. Imported chronology is never inferred from
array order, local storage indexes, or `q`.

Supersession forks are reported as defects; a defective fold is not approval evidence.
A withdrawal removes that request from the active pending set without fabricating an answer;
its reason and duplicate relationship remain historical facts. `needs_ask` means the request
is open, has no `ask_linked` facts, and is not in a legacy-unproven tree.
Subjects under `custom/decision/tree/v2/legacy/<encoded spelling>` preserve unproven legacy
ownership: their `opened.seat` is the import actor, not proof of a native seat mapping.
Their requests can remain open, but `needs_ask` is always false; they never propose ask
creation. Normal trees require their subject suffix to match the creation actor's seat.
`ask_links` preserves link fields without assuming that a linked ask is still active or
authorized. The fold does not distinguish blockers from
refinements for this open-request accounting. Decision state is derived from effective
requests and guards, never trusted from a last-written `status` or an unrelated native ask.
