# Conversation timeline blocks

Fractal retains native harness records independently from its conversation presentation.
The terminal UI, web and phone share one normalized semantic display contract:
conversation is a projection, not a native event log. Access checks still apply;
only reasoning the harness exposes is available.

The **versioned semantic projection** below is the approved target contract.
The existing `conversation-blocks.v1` implementation remains the legacy baseline
until each consumer migrates. This specification replaces the former policy that
unknown native records receive visible JSON transcript rows; it does not claim
that the implementation has already completed that cutover.

## One display contract

Keep the timeline envelope (`id`, source `sequence`, `revision`, `timestamp`, `role`,
`type`, `final`, `body`) and use a negotiated `body.blocks` array. Each block has
`id`, `kind`, `source_type` and `payload`; optional `continuation` describes an
owner-fetched remainder. Kinds are text, reasoning, tool_call, tool_output, image,
job, subagent, ask, status, error, document, source_record, raw_text, image_link and unknown. For standard known bodies,
`payload: {body_ref: true}` refers to the containing entry's fallback body, avoiding
an identical second copy; a continuation still fetches the original full body. Preserve native identities, call/result
correlation, revisions, supplied timing and failure state. Job/subagent payloads
preserve supplied parent/activity links; an historical ask is never a live picker.
Errors also have a known `error` block with `{body_ref:true}` for native stop/exit
notices. Optional `block.metadata` is an open JSON
object for source-supplied timing (including `wallTimeMs` and `timeoutSeconds`),
preserving original names, units, values and future fields. Native tool-result details
and assistant text/reasoning metadata use that same shape.
Unknown retains `payload: {raw: <original JSON>}`, the native discriminator and
source attribution without inventing a speaker. Future fields and full structured
tool arguments survive. Raw JSON is available through explicit inspection, never
as synthesized conversation prose. A `source_record` block with the `internal`
visibility hint retains the entire parsed native record, including unused fields.
A `document` retains the original Claude document. Legacy `visibility` is an open
UI hint, not a read-scope restriction; the versioned projection derives default
disposition from semantic category. Unknown attribution does not authorize omission.

In the legacy implementation, Web ignores source records when classifying visibility,
hides all-internal bodies behind inspection, and uses a disclosure for mixed bodies.
Text blocks referencing the whole `body.text` cannot safely separate mixed prose
from administrative JSON. The versioned projection requires per-block text or typed
fields: it MUST NOT reconstruct visible prose from a mixed raw fallback body.
This requirement applies to all surfaces, not only Web.

Malformed JSON, unfinished tail lines, blank lines and unknown encodings become
`raw_text` blocks. Their payload has `encoding: "base64"`, `bytes` containing the
exact original bytes (including newline and controls), and a lossy `text` preview.
OpenCode message and part data are read as SQLite bytes, including BLOB/invalid
UTF-8 values, and preserved by the same rule. Replacing/completing a partial record
invalidates its old ref visibly; appending another record preserves previous refs.
A torn prefix can produce both a raw block and a recovered typed record.

### Native record visibility audit

The sole omission table is `omission_reason` in
`crates/fractal-store/src/external_sessions.rs`: one named match arm per permitted type with
an explicit reason. It omits only the four categories below. Every other source
record MUST be retained with an explicit semantic outcome, including Unknown when
unclassified; an empty reading-lane projection is not permission to drop the record.
The coverage test uses harness-specific native record fixtures, the redacted OMP
resume/tool captures (also exercising Pi’s shared format), and message/part objects
from the OpenCode 1.18.34 admission capture. It fails if an empty result has no
table justification; separate probes cover unknown types, malformed bytes and
direct owner reads. Fixture provenance is in `crates/fractal-daemon/fixtures/native-records/SOURCES.md`. When visibility is unproven, retain it.

| Harness records | Decision and reason |
| --- | --- |
| Codex `session_meta` | Omit the session identity/cwd setup header; it is not a terminal turn. |
| Codex `event_msg`, `turn_context`, `token_usage_record`, `world_state`, response `ghost_snapshot` | Retain raw; terminal visibility varies by native subtype/release. No generic exclusion of event mirrors. |
| Codex message roles `system`/`developer` | Omit provider bootstrap instructions, not terminal chat turns; unknown/future roles are retained raw. |
| Claude `progress`, `file-history-snapshot`, `file-history-delta`, `queue-operation`, `permission-mode`, `mode`, `atis-latch`, `last-prompt`, `ai-title`, `custom-title`, `cost-state`, `agent-name`, `tag`, `pr-link`, `bridge-session`, `fork-context-ref` | Retain raw; no demonstrated generic harness-hidden rule. |
| Claude attachments other than queued commands and system records without text | Retain raw; queued prompts and textual notices keep their known display shape. |
| Pi/OMP `session` | Omit the native session setup header. |
| OMP `reset_boundary`, `credential_pin`, `title` | Retain raw plus typed status views; reset/title are quiet events, credential pins are internal bookkeeping hidden by default. |
| Pi/OMP `custom_message` with explicit `display: false` | Omit because the native extension explicitly marks it hidden from its terminal. (The administrative `model_usage`/`title_change`/`session_init`/`ttsr_injection` records are not in this row: they are retained, and `display: false` marks them `hidden-by-harness`.) |
| Pi/OMP `custom`, `label`, `session_info`, model/thinking changes and summary records without text | Retain raw; summaries with text remain readable native notes. |
| OpenCode `snapshot` | Retain raw; visibility is not established across releases. |
| Claude documents | Retain the complete document payload, with a readable document fallback. |
| Malformed/partial/unknown-encoding input | Retain exact bytes as a `raw_text` block; never silently skip a tail line. |
| Any unknown record/block/role | Retain original JSON and source attribution; count as Unknown and expose ordered inspection, not a raw transcript row. |

These decisions avoid silent per-type exclusions. Bounded native input windows and
read failures still produce visible size/read notices; they are transport
limitations. Unparseable records within the input bounds remain accessible as raw bytes.
OpenCode items whose native rows fit the input bound receive bounded display stubs
and full owner continuations. Rows exceeding that bound produce explicit
not-fetchable notices, rather than disappearing or acquiring unusable refs.

The normalizer retains exposed reasoning and image references. Native transcript
formats remain authoritative; JSON previews and excerpt limits apply only to
inspection. The retention audit above does not determine reading-lane visibility.

## Versioned semantic projection

### Categories and default disposition

Adapters MUST classify each retained native record or independently identified part.
Known native discriminators use explicit typed mappings; input and wire decoders
remain open to future discriminators and fields. Category is not inferred from
`[omp ...]`, XML wrappers or any other conversation-text prefix.

| Category | Default presentation | Examples |
| --- | --- | --- |
| Conversation | Reading lane, with existing reasoning/image/document controls | Human/model text, visible extension messages, supplied summaries |
| Tool activity | Existing folded invocation/result controls | Tool calls, correlated outputs, jobs/subagents and historical asks |
| Actionable notice | Quiet typed row or live authorized question control | Rule applied, model/context change, session failure, approval required |
| Usage | Compact usage indicator; no per-record reading-lane row | OMP `model_usage`, request token/cost facts |
| Internal activity | Counted, ordered inspector; no prose row | Session initialization, title/header bookkeeping, credential pin |
| Unknown | Distinct counted disclosure and ordered inspector; no prose row | Unclassified type/subtype/role or unsupported semantic category |
| Source diagnostic | Visible bounded diagnostic, with authorized source inspection | Malformed bytes, unavailable/truncated input, invalid required native fields |

Known administrative data MUST NOT become Conversation merely because its view
is unsupported. OMP `session_init` is Internal activity; `title_change` updates
header state; `ttsr_injection` remains a quiet Rule applied notice, never rule
text dumped into prose. Errors and unanswered permissions MUST NOT disappear
under a blanket telemetry/internal filter. A source `display:false` hint remains
available and cannot promote hidden native content into the reading lane.
Source records and inspection references are not extra conversation rows.
Pi/OMP `custom_message` has explicit native display intent (`CustomMessageEntry.display`:
OMP `packages/coding-agent/src/session/session-entries.ts:323-332`). An otherwise
unknown, valid `customType` is Conversation only with `display: true`; false or absent
display leaves it counted, inspectable Unknown. Known tool/notice subtypes retain their
explicit categories. Missing, null or non-string required type/subtype/role discriminators
are Source diagnostic, not the future-string fallback.

One source record may yield several semantically distinct parts, or no reading-lane
rows. Each part MUST have stable identity, source linkage and its own display
text/typed fields. Unknown attribution remains unknown; embedded Claude request
roles MUST NOT turn bookkeeping into the person's words.

### Counts, inspection and source identity

Unknown is retained data whose meaning is not classified, not a synonym for safe
bookkeeping. Its visible disclosure MUST distinguish it from Internal activity.
Counts identify scope: `loaded window` unless the owner supplies a complete
session total. Partial source coverage, unavailable history or transport limits
MUST be shown; a zero count for an incomplete window is not a whole-session claim.

Count distinct stable source-record/part identities once per category; retransmitted
frames, continuation expansion and page overlap MUST NOT inflate counts. Multiple
Unknown parts of one record require explicit part identities, not row-position IDs.
Inspection preserves source order, discriminator, harness, attribution, native
timestamp, source identity and available raw payload/bytes. Existing access checks,
bounded previews and owner continuation validation apply to every inspection.
Denied, expired or unavailable source is explicit, never an empty successful view.

Pagination remains source-based. A page containing only non-reading-lane events
MUST still advance its cursor; filtering MUST NOT skip source identities or require
rescanning the entire transcript per frame. Mixed blocks retain real prose without
exposing neighboring raw payloads. Show-everything is an explicit reversible debug
view, not the normal reading lane and not an authorization bypass.

### Usage indicator

The indicator exposes only source-supplied normalized facts: input/output/cache
tokens, total tokens and monetary cost with currency when supplied. Each metric
carries its units, coverage scope and completeness; unavailable is not zero.
Native explicit zero remains zero. Estimates and model/provider attribution remain
identified; unsupported metrics stay absent rather than being fabricated.

Adapters MUST distinguish per-request deltas from cumulative snapshots and supply
stable request/counter identity. Request retries, fallback/auxiliary model calls,
assistant metadata and standalone usage records MUST NOT double count one fact.
Cumulative snapshots replace their counter state; independent deltas accumulate
once. If correlation or semantics are unknown, expose unaggregated inspectable
facts and unavailable aggregate rather than summing arbitrary JSON numbers.
Loading older pages and reconnecting MUST NOT inflate totals. Visible values label
loaded-window versus complete-session scope and retain source links for inspection.
This contract supersedes blind summation of legacy header cost metadata.

### Schema evolution and shared ownership

One schema authority owns semantic category, disposition, source identity,
counter semantics and projection revision. Generate client definitions from it;
Rust, Web and iOS MUST use the same language-neutral conformance vectors.
Layout remains surface-specific. Independent harness-type allowlists or raw-text
fallback parsers in each renderer are not alternative semantic authorities.

CI MUST inventory observed fixture discriminators by harness, type and relevant
subtype/role, and fail on an unclassified discriminator or unjustified empty
adapter outcome. Synthetic future-type vectors explicitly expect Unknown.
Additive fields MUST survive without failing classification. Fixture provenance
MUST distinguish real redacted captures from constructed contract examples.
Runtime MUST retain Unknown and continue the session rather than reject the
whole transcript on a new native type. Bounded local compatibility counters may
record harness/version/type/adapter revision, never prompts or raw payloads.
Remote telemetry requires a separate privacy decision.

### Projection revision, rebuild and compatibility boundary

The semantic projection capability MUST be distinct from `conversation-blocks.v1`.
Responses identify the selected projection revision; clients request only revisions
they implement. Native bytes/IDs are authoritative; normalized items are derived.
Projection revision MUST participate in fold/cache identity and page-cursor
validation. Changing it deterministically rebuilds projections from retained native
records, invalidates obsolete cursors with explicit resync, and preserves source
identity/order and authorization. It MUST NOT rewrite native transcript history.
Process-local caches clear on restart; persistent or hot-reconfigured projections
MUST NOT reuse stale items merely because source bytes are unchanged.

All supported consumers MUST migrate before removing their old-capability path.
Supported legacy readers receive safe typed-derived readable fallback: actual
conversation text and bounded semantic notices, never serialized administrative or
unknown payloads as prose. Unsupported readers receive an explicit upgrade refusal
before incompatible decoding; closed legacy top-level enums MUST NOT receive new
unnegotiated kinds. Exact minimum reader versions belong to the generated client
compatibility matrix when this capability ships, not an invented version here.
No indefinite raw-as-prose bridge remains after that boundary. Raw inspection and
authorized source continuations remain available independently of reader layout.

### Required conformance vectors

Every surface MUST exercise identical expected categories, default reading items,
usage facts, scoped counts and source-inspection identities for these vectors:

| Vector | Required assertion |
| --- | --- |
| Real redacted OMP admin capture | Usage outside prose; init internal; title updates header; quiet rule notice |
| Claude/Codex native-shape contract fixtures | Explicit bookkeeping/mirror classification, no false speaker or duplicated prose |
| Literal `[omp model_usage]` in actual message text | Preserved as conversation, never prefix-filtered |
| Synthetic future type/category/role and additive fields | Counted Unknown; runtime continues; raw retained; extra fields survive |
| Missing required fields and malformed/torn bytes | Explicit diagnostic and exact bytes; valid recovered record not lost |
| Mixed text/internal/unknown blocks | Real prose retained independently; correct distinct counts and inspection |
| Usage delta/snapshot/retry/duplicate/absent/zero | Correct deduplication, scope and units; unavailable never fabricated zero |
| Page overlap, invisible-only page, reconnect and session switch | Stable source cursor/counts; no duplication or cross-session state |
| Revision change with unchanged native input | Rebuild, explicit cursor expiry/resync, native identities preserved |
| Continuation denied/expired/oversized and old/new readers | Honest bounds/access outcomes and negotiated compatibility |

Parser retention tests alone are insufficient. Producer vectors, Rust/TypeScript
projection tests and fixture-driven TUI/Web/iOS captures MUST prove the normal lane
contains no synthesized native JSON. Explicit debug/export paths MUST still recover
the authorized original record. Migration is complete only when obsolete raw
fallback synthesis and per-surface native-envelope inference are removed.


## Typed views and conversation header

Legacy typed views are additive and optional on `conversation-blocks.v1`; old clients
ignore unfamiliar fields. Versioned semantic readers instead use category disposition
and the compatibility boundary above, never a raw-as-prose fallback. A block MAY
carry `view: {type: <string>, ...}`; `type` is an open discriminator, and the client-v0
schema encodes one closed definition per known `view.type` (`TimelineView*` in
`docs/st3/client-v0/schemas/client-v0.schema.json`) plus a `TimelineView` fallback
branch that accepts any other type string. An unfamiliar view uses its known semantic
category or Unknown, not serialized native prose. `view` holds parsed fields only;
the full native arguments or output stay in `payload`, without changing the block contract. Raw JSON
stays available for every record, `view` is computed per record and deterministic,
and the fold caches it with the entry. Generated Rust, Swift and TypeScript clients
carry `view` and the header below as loose JSON values (`Option<Value>`, `JSONValue?`,
`unknown`), so no client decoder gains closed cases.

`conversation_blocks::prepare_one` bounds both `view` and `metadata` after enriching
child-session links, for full reads and native keyset pages alike. Their outer object
and keys (including `view.type`) survive; long strings carry the visible size-limit
marker, and oversized nested objects/arrays may become marked JSON-preview strings.
Generated clients retain these as loose JSON, and both conversation renderers check
string/array/object shapes rather than assuming a closed view. A single clipped open
subtree uses a continuation to `/body/blocks/{index}/view` or `/metadata`; if another
remainder already needs that block's continuation, it instead points to the whole
original `/body` so owner fetch recovers every exact subtree. Clients inspect the
returned JSON accordingly. The final entry-size guard still runs after all display
bounding and can replace an over-budget entry with a fetchable error notice. `omp_event`
view fields are bounded before the view is built so it stays parseable: kind and attribution
have 256-byte windows, content and details 8 KiB windows, each plus a visible marker. The
complete raw record remains available through the block's owner continuation. Page windows target 768 KiB
but retain one complete larger entry up to the hard response ceiling (1 MiB minus 128,000
reserved bytes); paging refuses an entry beyond that hard ceiling rather than skipping it.

On `tool_call` blocks, parsed from the native arguments (OMP field `i` becomes
`intent`; every tool_call view also has `tool`, the native tool name):

| view.type | fields | OMP source |
| --- | --- | --- |
| `bash` | command, cwd?, timeout_s?, env_keys?: string[], background: bool | tool `bash` {command,cwd,timeout,env,async} |
| `edit` | path?, ops: number, input_bytes | tool `edit` {input} (hashline patch; path = first `[PATH#TAG]` header) |
| `write` | path, bytes, content?, line_count? | tool `write` {path,content}; line count counts native written lines |
| `read` | path, range?: string | tool `read` {path} (suffix after `:` = range) |
| `search` | engine: "grep"\|"glob"\|"web", pattern?, path?, query?, case?, hidden?, gitignore?, limit?, skip? | tools `grep`, `glob`, `web_search`; flags preserve explicit false/zero values |
| `todo` | op, items?: [{content, status}], phase?, task? | tool `todo` |
| `ask` | questions: [{id, question, options: [{label, description?}], multi: bool, recommended?: number}] | tool `ask`; every alternative survives the selected answer |
| `task` | context?, tasks: [{name?, agent?, task}] | tool `task`; context carries the parent context/contract |
| `hub` | op, name?, target?, timeout_s?, message? | tool `hub`; send body remains beside its receipt |
| `eval` | language, title?, code_bytes, code?, timeout_s?, reset? | tool `eval`; preserve explicit reset=false and timeout=0 |
| `generic` | name | any other tool |

On `tool_output` blocks, parsed from the OMP toolResult `details`; all of these also
have `tool`, `call_id` and `is_error`:

| view.type | fields |
| --- | --- |
| `bash` | exit_code?, wall_ms?, timeout_s?, timed_out?: bool |
| `search` | engine, match_count?, file_count?, truncated?, file_limit_reached?, per_file_limit_reached?, warning?; counts come from native matchCount/fileCount, never inferred from bounded rows |
| `edit` | path?, first_changed_line?, diff?: string (unified diff as given) |
| `todo` | phases: [{name, items: [{content, status}]}] |
| `ask` | answers: [{question, selected: string[], custom?: string, note?: string}] |
| `task` | async: bool, total_ms?, agents: [SubagentSummary] |
| `hub` | op, timed_out?: bool, jobs?: [JobSummary], state? |
| `generic` | is_error: bool, wall_ms? |

`SubagentSummary` = `{id, name?, agent?, status, task?, duration_ms?, tokens?, cost_usd?,
requests?, tool_count?, conversation?: {session_id}}`. `name` preserves the OMP name
(native `name`, otherwise `id`); `agent` is its type. `task` preserves the full native
`assignment`, otherwise `task`. Both clients retain invocation rows when the result
arrives and draw all assignment lines only on the invocation's expansion. Subagent
result cards show status, usage and the child-open link, without repeating the
assignment. The child-open row names the agent while routing to its actual session
id. The owner fills
`conversation.session_id` only when the child transcript exists at
`<parent transcript without .jsonl>/<id>.jsonl` and is readable; that session id opens
through the normal conversation routes (same fold, paging and refs), which is how a
subagent card links to the child transcript as its own conversation.
`JobSummary` = `{id, name?, type?, state, exit_code?, started_at?, ended_at?,
duration_ms?, output_bytes?}`.

Extension and bookkeeping records become blocks as follows; anything not listed keeps
its block shape:

| OMP record | block kind | view |
| --- | --- | --- |
| custom_message `irc:incoming` | `irc` | `{type:"irc", from, message, reply_to?, message_id}` |
| custom_message `launch-completion` | `job` | `{type:"job", jobs: [JobSummary]}` (from details.daemons) |
| custom_message `async-result` | `job` | `{type:"job", jobs: [JobSummary]}` (from details.jobs) |
| custom_message `skill-prompt` | `status` | `{type:"skill", name, path?, args?}` |
| `compaction` / `branch_summary` | `status` | `{type:"compaction", method?, tokens_before?, tokens_after?, short_summary?, summary?}`; first lines collapsed, full summary on expansion |
| `model_change` | `status` | `{type:"model_change", model, role?, fallback?: bool}` |
| `thinking_level_change` | `status` | `{type:"thinking_level", level, configured?}` |
| other visible custom_message | unchanged (`text`) | `{type:"omp_event", kind, attribution?, content?, details?}`: `kind` is the native customType, the rest are the native values present on the record (attribution is never inferred); content and details are bounded separately to 8 KiB (marked text when over) and the full record stays in the raw payload |
| `model_usage` / `session_init` / `ttsr_injection` | `status` | `{type:"omp_event", kind: <record type>, attribution?, details: <record>}`; explicit `display: false` is retained with visibility `hidden-by-harness` |
| `title_change` / `title` | `status` | `{type:"title", title, previous?, source?}` (`previous` only on title changes) |
| `reset_boundary` | `status` | `{type:"reset_boundary"}`; quiet event `session reset` |
| `credential_pin` | `status`, visibility `internal` | `{type:"credential_pin", provider}`; hidden by default; hash remains only in raw payload/source record |
| custom `session_exit` | `status` | `{type:"session_exit", kind, reason}` |
| custom `tool_execution_start` | `status`, visibility `internal` | `{type:"tool_start", call_id, tool, started_at}`; renderers attach it to the call row and do not draw it alone |
| assistant message metadata | on its `text`/`reasoning` blocks | `metadata.model`, `metadata.provider`, `metadata.usage` {input, output, cache_read, cache_write, total, cost_usd}, `metadata.context_tokens`, `metadata.stop_reason`, `metadata.ttft_ms`, `metadata.duration_ms`; no view |
| assistant error message | `error` | `{type:"assistant_error", status, presentation, is_error, label, message, stop_reason?, error_id?, api?, provider?, model?, retry?}` |

Assistant errors come from the assistant session message, not a synthesized daemon
failure. Persisted `retry` markers carry `{kind,status,attempt,recovery,note,
recovered_at?,superseded_by?}`. Status is `failed`, `recovered`, or `superseded`;
presentation is `full`, `compact-recovered`, or `none`, matching OMP's persisted
outcome. Original error text remains available on expansion after recovery.
Superseded errors are retained in source data but not drawn. No in-progress retry
state is invented when the session record does not contain it.

Written content, assignment/context, eval code and compaction summaries use the
existing 8 KiB display bound and authenticated continuation. Typed view-only refs
are reattached to their original blocks and passed through the same adapter on
expansion; metadata and unknown subtrees keep a raw JSON fallback. The written
line count describes the complete source, not the bounded preview.
Simplified rendering and history auto-fill share the same bundling predicate:
write, compaction and assistant-error preview cards remain separate rather than
being counted as one ordinary-tool run.


Timeline pages and delta responses (when `conversation-blocks.v1` is negotiated) MAY
carry a `header` object. Each field is `{value, source: register|transcript, as_of}`:
`model` (model id), `context` ({tokens, window}), `cost` ({usd}, with `window: true`
when the fold window is truncated), `todos`, `jobs` ([JobSummary]), `subagents`
([SubagentSummary]), `ask` (the last ask call without a matching result, or null) and
`working` (bool). `source` can be `register` or `transcript`; a register field takes
precedence when supplied. The current `conversation_header::register_value` adapter
returns `None`, so headers contain transcript-derived fields and omit `working`. A
field is absent when its source supplies no value. Transcript derivation uses the
normalized block metadata: model = last `metadata.model`; context = last
`metadata.context_tokens`; legacy cost = sum of `metadata.usage.cost_usd` in the window
(versioned readers use the usage-indicator deduplication/scope contract instead);
todos = last todo tool_output view, else the last todo call; jobs = latest state per
job id from job blocks and hub outputs, keeping non-terminal ones; subagents = task
outputs whose status is not terminal; ask = the last ask call without a matching
result. `docs/st3/client-v0/fixtures/timeline-views.json` fixes the wire shape with a
synthetic page that carries views, an `irc` block and a full header.

The Rust and TypeScript renderers show every populated header field in the same fixed
order. The most common source appears once at the end (`· transcript · 2d ago`);
ties use the first source in field order. Only fields from another source get their own
marker (`cost $0.02 [register · 30m ago]`). The shared age uses the oldest field from
that source, so compaction cannot hide stale data. Phone headers wrap without a line
limit. A subagent's open-child link is an independent accessible control, outside the
card's expand/collapse control; opening it pushes a conversation so Back restores
the parent. Todo statuses remain distinct: `[x]` completed, `[ ]` pending, `[~]`
in progress, `[!]` blocked and `[/]` abandoned. Only `completed` counts as done.

## UI filters and show-everything mode

Rust `conversation_with_filters` and TypeScript `conversationEntries` accept an
explicit named filter set. `DEFAULT_FILTERS` selects harness-markup, context-block,
control-character, internal-block and excerpt filters. This keeps terminal UI and the
phone's familiar delivery/task/command display shapes, folded tools/mail and
heartbeat/usage suppression, while new reasoning, image/link, document,
job/subagent/ask/status blocks have readable typed controls. Legacy filters
hide XML reasoning/internal/system-reminder/tool wrappers, setup context wrappers
(permissions, environment, collaboration, skills, app/plugin and command metadata),
control characters and `internal`/`hidden-by-harness` payloads. Versioned readers
apply semantic disposition before layout: Unknown has a counted inspector rather
than an excerpted prose row. Bounds remain presentation-only choices.

`SHOW_EVERYTHING` is the empty filter set: every entry, including message metadata,
usage/status, original JSON blocks and raw byte payloads, is rendered as reversible
JSON without folds, excerpting or markup removal. JSON escapes controls so a raw
terminal view can expose them without interpreting them. Decoding that JSON yields
the unchanged normalized entry. Transport size caps and owner continuations still
apply; raw mode cannot conjure an unrequested chunk.

The terminal UI and the phone fetch image bytes and clipped remainders through the
chunk route. Existing graph-message image handling is separate. Generated Rust,
Swift and TypeScript clients expose the chunk API.

## Owner reads, images and limits

Bind an exact seat/session/incarnation and native file identity on its owner machine.
HTTP pages, follow, search and image/body fetches use that binding and conversation
access control through existing owner forwarding. No newest-file guesses. The
conversation reports **owner unavailable** when unreachable; a reachable owner with
a missing transcript reports **transcript unavailable**, rather than an empty session.
Neither state substitutes stored transcript operations or an old prepared page.

Use `GET /v1/client/conversations/{session}/content/{ref}/chunk?offset=N` for
image bytes and oversized block bodies. The opaque ref is scoped to the authorized
session, entry/revision and owner source identity, not an arbitrary path or URL.
Return bounded bytes with media type, total size and next offset; check authorization
and binding on every fetch. Inline/base64 pixels and files in the bound Pi/OMP
content-addressed provider blob store can be read on demand. Blob digests are
verified on every fetch, so changed pixels cannot be joined across chunks. A transcript `file://` URI grants no
access outside that store; symlink escapes and non-regular files are rejected.
The daemon never requests transcript HTTP(S) URLs, including their redirects;
`image_link` retains the original URL and JSON for explicit opening by the client.
Detected PNG/JPEG/GIF/WebP signatures determine image MIME; native MIME labels are
ignored. SVG/HTML and unrecognized bytes remain fetchable as opaque octets and
must never be rendered as active image/HTML content. Reuse transport/chunk
machinery, not message-attachment authority: a native image need not have a graph
message ID. Native absence or fetch failure is availability, not content scrubbing.

`read.projections` is the raw-transcript read scope. It authorizes full native
reasoning, arguments/output, unknown JSON and image bytes, including secrets the
agent saw. This includes anonymous local read-only Unix sessions, identified local
actors, and projection-only paired display/phone keys. There is no separate secret
filter or raw-content scope. Pairing scope changes and revocation apply to every
chunk request, including owner-forwarded requests.

The 8 KiB/value and 768 KiB timeline-page bounds protect memory and transport only.
The initial chunk contract returns up to 256 KiB decoded bytes; image reads have
a visible 32 MiB decoded limit. Four expensive owner timeline/chunk reads can run
concurrently; excess reads, including managed timeline reads, return HTTP 429
`rate-limited`, without a queued backlog. Clients must back off and retry.
Input bounds are distinct from response clipping. A JSONL read captures at most
32 MiB of source bytes and retains at most 4,096 complete lines from that window.
The partial first line and older prefix are marked **not fetchable through this
owner read**; there is no prefix continuation. A line exceeding the
window can therefore have no complete record in this response, with the same
explicit marker. Within the window, clipped values have fetchable continuations.

Open `block.metadata` retains its outer object and keys: strings over 8 KiB use
the existing `[st truncated this native timeline value: size limit; N bytes]`
suffix, and nested collections still over 16 KiB after leaf clipping become
clipped JSON-text previews. The block's existing `continuation` shape fetches
the original `/body/blocks/N/metadata` when no payload continuation is present;
an existing payload continuation takes precedence (a `/body` ref also includes
the original metadata). Error/status `message`, `details` and `detail` fallback
fields are bounded in the same preparation pass; strings remain strings and
negotiated body-ref blocks fetch the complete original `/body`. Legacy clients
receive the same visible clipping without blocks or continuation refs. A final
entry-size guard bounds remaining large body fields; an entry whose retained
keys still cannot fit becomes a visible `native-entry-too-large` error with the
same identity and ordering, without discarding its page's other entries.
Negotiated clients fetch that entry's exact original `/body`; legacy clients
receive the error message without its blocks.

A single 32 MiB text-heavy JSONL line is held several times during parsing,
source-record attachment, normalization and response preparation: budget roughly
150–250 MiB transient per read, or 600 MiB–1 GiB for four concurrent reads, in
addition to the daemon's other work. This is an estimate, not a strict RSS limit:
dense JSON, many projected parts, allocator overhead and encoded/base64 expansion
can exceed it. The source-byte limit and four-read admission limit are enforced;
these do not impose a hard heap/AST budget or guarantee streaming JSON normalization.

OpenCode streams selected messages and their parts instead of copying up to
4,097 message rows and every part into Rust vectors. Before any Rust byte copy,
the total ID/time/data cells of a message row, or ID/data cells of a part row,
must fit 32 MiB; `sqlite_bytes` also checks each cell. At most one accepted
message row and one part row are being normalized at a time, alongside the
32 MiB encoded/4,096-entry timeline suffix. Parsing and projection incur the
same transient-copy/AST overhead as JSONL. SQLite can materialize cells or sort
rows inside its engine before the borrowed-cell guard, so this is a Rust payload
copy bound, not a SQLite-engine or process RSS guarantee. Over-limit rows emit
`native-record-size-limit`, original source size and `fetchable:false`, visibly
**not fetchable**; a rejected message's parts also lack usable display context.
Direct chunk reads apply the same row bound; enlargement invalidates an old ref.
The native source is still authoritative and is not modified by these limits.

The OpenCode message-window query-plan regression uses the harness's
`message_session_time_created_id_idx(session_id, time_created, id)` from
[OpenCode v1.18.34](https://github.com/anomalyco/opencode/blob/aec0b9a6d8898f68f923aaf08b7306d931fd9d76/packages/core/src/session/sql.ts#L64-L76).
`EXPLAIN QUERY PLAN` shows an indexed session lookup in the inner query, with
only the already-limited outer window sorted chronologically (at most 4,097
rows). A read-only check of the installed native schema produces the same plan.
This guarantee assumes that composite index exists; older schemas without it
may sort session history before applying the limit. The regression exercises
the production SQL, including timestamp ties and session isolation.

An authenticated ref contains a JSONL offset/length/digest or SQLite part/message
identity/digest (message role/time, independent of streaming usage updates),
entry/revision, session and stable native file identity. Its encrypted source
descriptor carries the owner-located driver, native ID and transcript path; the
client cannot see or choose that path. No conversation bytes are carried in the
ref. Native chunks use this descriptor directly, without inventorying other
sessions. Managed chunks retain current owner binding checks. Each chunk reads and
normalizes its identified record once, then rechecks source identity after fetching
bytes. It returns the validated record snapshot; content-addressed blob hashes bind
external pixels to that snapshot. A concurrent edit after capture is detected on
the next fetch. No whole timeline rebuild or second record decode occurs.
Appends, including SQLite WAL growth, preserve existing refs;
record edits, file replacement, managed binding changes or an owner process restart return
`conversation-content-invalidated` with `full_resync: true`. The ephemeral owner
authenticated-encryption key is neither stored nor replicated. Clients must reload
the timeline after a daemon restart to obtain new refs. Timeline reads capture a finite native
high-water mark; concurrent appends belong to the following read.
Count encoded response bytes, preserve valid JSON/UTF-8, and mark every clipped
value with its reason, original size when known and continuation. Fetching the
remainder recovers full arguments/output/unknown JSON; do not cut JSON into an
apparently complete object. Oversized entries must make progress through chunks.
File/line scan bounds and missing native history also need explicit partial notices.
Paging cursors fence source revisions; invalidation requests resync and preserves
explicit notices for unavailable content.

## Client compatibility

Existing terminal UI/iOS clients retain their negotiated capabilities. Swift decodes a closed
`TimelineType` enum, so sending new top-level kinds to old clients is breaking.
Advertise/negotiate blocks, chunk fetch and semantic projection revision. Supported
old clients retain their known text/tool/status shapes through safe typed-derived
fallbacks, not native JSON. New clients apply the category/count/usage contract;
they inspect unknown JSON only explicitly. Unsupported readers are refused before
an incompatible wire response. Removing legacy paths requires the supported-reader
matrix and all-surface conformance vectors above.

## Collection timeline frames

The terminal UI (`fractal-feed`) and phone (`apps/ios/feed.ts`) subscribe to a conversation on
one existing collections socket. The first `replace:true` frame contains the newest 200
entries, subject to the existing 1 MiB frame limit, with `has_more` when older content exists.
Older content remains available through the existing 200-entry timeline pages and owner
content chunks. Subsequent `replace:false` frames carry new or revised entries by ID.

Native assistant stop outcomes use the shared `assistant_error` block and harness
presentation rules: superseded attempts, silent aborts and user interrupts render
nothing; recovered attempts show a compact note and terminal failures render in full.
The view retains the exact native `stopReason`, `errorMessage`, `retryRecovery`,
`errorStatus` and `errorId` under `native`, alongside the presentation projection.
Native source records remain available through owner reads.

Session exits preserve the complete checkpoint, including `pendingToolCalls`,
arguments/intent and unknown fields. Only transport size and paging limits apply,
with visible truncation markers and owner continuations for exact full values.
Outcomes are read from the owner machine, not stored in the database.
Recognized exits use the existing `cancelled` status value — the process stopped
before the turn finished; the recorded kind, reason and exit code stay in `detail` —
so a planned stop is neither a completed turn nor an error, with no new
client-visible vocabulary. Fatal and unknown exits remain diagnostic errors.
