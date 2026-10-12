# Fractal UI

A small, independent UI kit for the Fractal web client, built on React Aria Components and StyleX, with Storybook as its workshop.

It has two layers:

- `.` (`kit.tsx`, `tokens.css`): token-driven component families. The workshop renders them in three switchable visual directions (**Folio**, **Relay**, **Orbit**), each with light and dark palettes and two densities.
- `./assistant-ui` and its subpaths: the dark-first application surfaces the web app composes, built on assistant-ui 0.15.25. They cover the transcript, composer, tool calls and previews, the sidebar agent row and status glyph, the thread header, resizable splits, resource cards, the work log, the sync line and the workbench layout model, plus rich markdown with highlighted code, the thinking disclosure and the dismissible failure layer. Every surface is controlled: the host supplies data, clock and transport, and unknown values render as unknown.

## Run

```sh
nix develop .#web
pnpm install --frozen-lockfile  # repository root
pnpm --filter @smalltalk/fractal-ui storybook        # http://127.0.0.1:53705
pnpm --filter @smalltalk/fractal-ui typecheck
pnpm --filter @smalltalk/fractal-ui build-storybook  # packages/fractal-ui/storybook-static/
```

The package is an explicit member of the root pnpm workspace and uses its pinned toolchain, frozen lockfile and lifecycle-script suppression. `bash scripts/ci-fractal-web` runs its typecheck and the static Storybook build after the shared install.
The kit keeps TypeScript 6.0.3 and declares the Node types used by its Storybook configuration explicitly.

The static kit book disables Vite CSS splitting only in its Storybook config.
`@stylexjs/unplugin` 0.19.0 collects the full book's atomic rules, then appends them
to one emitted CSS asset; its `cssInjectionTarget` option chooses an existing
asset, not a preview-head injection. With split CSS, the default choice can be a
lazy story's stylesheet, leaving direct visits to other stories unstyled.
With `cssCodeSplit: false`, the shared stylesheet loads from `iframe.html` before
any story import. Development's runtime CSS injection is unchanged.

After the static build, the web gate runs
`node packages/fractal-ui/scripts/check-storybook-stylex.mjs`. It requires every
emitted stylesheet containing StyleX-generated class selectors to be linked as
a stylesheet in the preview head. This is a load-order-independent check for all
stories, rather than a visit that could inherit CSS from an earlier story. The
split-CSS configuration fails because its StyleX-bearing story asset has no such
head link. A negative control removes those links in memory and must fail the
same assertion; no built files are modified. This checks asset availability,
not browser rendering or execution of story plays.

The dev server binds to localhost on a fixed port (53705). To view it from another machine, forward the port, for example `ssh -L 53705:localhost:53705 <host>`, and open http://localhost:53705.

Stories:

- **Fractal UI / Visual language**: one working context rendered in the chosen direction. It contains a thread list, a conversation with a tool call and a diff, a terminal pane, a command palette and component specimens. `Explore` exposes direction, scheme and density as canvas controls and Storybook args. The `FolioLight` … `OrbitDark` and `Compact` stories are fixed comparison entries.
- **Fractal UI / Component families**: every family on one canvas (`AllFamilies`, `FamiliesDark`), under the same three switches.
- **Fractal UI / Markdown**: CommonMark + GFM coverage with streamed fences and inline markup; language fences assert highlighted syntax and stable wrap/copy state.
- **Fractal UI / Thinking Entry**: the muted reasoning disclosure, settled and streaming.
- **Fractal UI / Hierarchy r1**: a fourteen-row scrolling fold with Thinking, a rich answer with a loose list and a three-block prose run at 440/1440 in both schemes; plays measure the fold indent and rail, the shared child scroller, pinned disclosure and duration columns, visible non-success words, Thinking row parity, the header-only message, the run Copy text and Markdown rhythm.
- **Fractal UI / Sync Line**: every observation across both schemes; `TransitionSequence` asserts fixed-height slots and CLS 0 across all transitions.
- **Fractal UI / Transcript**: the locked U2·F3·Y3 transcript in dark and light: settled/expanded work, streamed answers, failed/interrupted/unknown work, loading, retained-history synchronization, answer metadata, unavailable conversations, the older-history boundary with and without a load action, pending and failed sends (including Pending→Sent identity) and the read-only empty state.
- **Fractal UI / Sidebar/Agent Row**: compact hover-card facts, omitted unreported fields, the known-vs-none time cohort and the quick Open hover action.
- **Fractal UI / Questions**: harness-neutral read-only and answerable questions, all settled outcomes, attention surfaces, and keyboard/negative-control stories at 440 and 1440 pixels in both schemes. Drafts and callbacks are host-owned; the terminal hand-off is explicit.
- **Fractal UI / Agent Messages**: promoted incoming messages and folded outgoing sends, with via chips, parent/child identifiers, per-recipient receipts, bounded long-message disclosure, and source-preview transitions. State-specific plays include the quoted-message and file-path negative controls.

### Harness questions

`QuestionForm` uses controlled `draft`/`onDraftChange` props and React Aria radio/checkbox groups. `autoFocus`, question-step transitions, Other and note editing transfer focus; Escape returns to the selected/recommended choice without answering or clearing the draft. Multi-select keeps custom text when another option changes. `NeedsYouBar` receives transcript visibility from the host and hides at 60%; its pinned form and the inbox must receive the same host-owned draft.

Hosts composing repeated forms, settled outcomes or docks may pass `ariaLabel` to name each landmark by its surface/context without changing visible question copy. The full state gallery and shared-draft composition use distinct labels; this keeps identical request views distinguishable to landmark navigation.

`composerRouting` distinguishes a message held until the question is answered from a note about the question; typing copy follows the host-reported route. Forms cap desktop width at the shared reading-lane token, inbox entries start as two-line disclosures, and pinned forms omit duplicate card chrome. The question stories compose the production composer, share drafts keyed by request id, and drive attention visibility with a real IntersectionObserver.

Question surfaces use shared Fractal composition palette, type, spacing, radius and geometry tokens directly, inheriting the nearest ancestor's StyleX theme. Roots establish the shared UI type scale and medium-weight headers instead of inheriting page typography. Selected option descriptions use the soft foreground token. They require no component-specific palette decorator; harness weight and selected-option keyboard plays include axe color-contrast checks in dark and light.

Recorded `answered` outcomes collapse to one medium-weight row: the selected choice for a single question, or the question count for a batch. A trailing disclosure opens per-question choices and provenance, with original questions behind a further disclosure. The kit derives compact age from the recorded timestamp and the host's numeric `now` clock prop, never a host-supplied display string. Its absolute tooltip uses the viewer's locale and time zone through Intl defaults, never a raw ISO label. Optional `locale`/`timeZone` props make story fixtures deterministic. Invalid or future timestamps omit the age. Successful calls without recorded answer facts keep their original questions/options visible and do not adopt the answered summary.

Question ages floor elapsed time into `now`, seconds, minutes, hours or days, then show a short English month/day after seven days (in the viewer's time zone). The absolute tooltip retains the viewer's locale. Age has a content-sized, non-shrinking track and never wraps; the choice takes the remaining space with ellipsis. Longest-age and long-choice stories keep the collapsed row within its height bound. The local formatter will fold into #2348's shared formatter at grammar adoption; this kit does not depend on that unmerged PR.

Read-only pending questions retain radio/checkbox choice affordances; archived original options use non-interactive choice rows with the same circle/box vocabulary. Settled footer timestamps use compact ages rather than raw source strings. `NeedsYouBar` owns its 12px, medium-weight, 24px-high toggle; the title truncates before the action label. Production plays exercise these looks and reject bold-title, missing-affordance, raw-date and oversized-toggle controls at both story widths and schemes.

The ratified question Card uses an attention hairline, 8/12 padding and a 24-high title row (E37/C24/C25, C1). Choice indicators and disclosures come from the shared SVG glyph family (C22); tags have no fill. The dock and inbox own their frames, while pinned forms omit theirs (C21). `CompositionFrames` mounts the production surfaces together and rejects nested frames and missing glyphs. Markdown uses the shared 8-unit sibling rhythm with no outer margins (C16), and pending input prose requests foreground ink (C27).

The generic `EmbraceToolCall`/`EmbraceToolItem` seam is explicitly read-only: its `ReadOnlyToolQuestion` type cannot carry a web-answer capability. Singular `{question, options}` and batched `{questions}` arguments keep their options; structured result `details.results`/flat `selectedOptions` facts render `QuestionSummary`. Calls that closed without recorded answer facts use `unrecorded`, not an invented answer. Live adapters supply richer typed requests and the terminal callback.

The ACH model is retained with small display extensions: `autoSelected` records per-question timeout picks; `answered.where` labels terminal/web provenance; `chat-redirected` represents the catalog outcome; `unrecorded` keeps archived missing-result facts honest. Optional `variant` defaults to transcript; `composerTyping` labels held composer text; `onAttach` hands real selected files to the host instead of implementing upload/storage in the kit.

### Agent messages

`AgentMessage` has a public tagged `variant`: `Incoming` (default), `Outgoing`, or `Human`, with pure sender, recipient, optional channel and tagged body props. Input headers are 16-high at 12/16, with a medium-weight 14/20 subject four below and body four below the subject, or two below a header without a subject. There is no footer. Incoming uses a raised Card with padding 8/12; Human defaults to an accent rail, and its optional tagged `frame: { _tag: 'Card' }` selects the recommended Card. One shared SVG glyph occupies the leading 16-unit slot (C22). Input body ink is foreground (C27). `headerMeta` replaces native copy/time, `headerGlyph` selects a kit icon or explicitly hides it, and `frameStyles` lets compositions adapt the native frame without wrapping a Card in another Card (C21).

The host owns fetching, copying and reply navigation. `onCopy` copies the message; if absent, the existing id-copy callback supplies the single header action instead. Short body regions add no synthetic Tab stop. Long complete messages render only a bounded excerpt until disclosed; a fetched long body opens immediately while retaining focus on its load action. Large expanded regions retain labelled, keyboard-focusable scrolling. Markdown parsing/highlighting is capped at 16,000 characters; larger incoming and outgoing bodies render as literal plain text, never an unbounded Markdown tree. Missing full-body access has an explicit keyboard-reachable reason.

Large plain-text bodies mount one 16,000-character page at a time. Previous/Next controls remain focusable at boundaries with an accessible unavailable reason, so paging never transfers keyboard focus or mounts the full multi-megabyte body.

`OutgoingMessageRow` is one folded 24-high WorkRow per send (E27/C9), including broadcasts. Its header reads `To` plus the known recipient/count, a bounded excerpt and receipt-count summary. Broadcast count uses the reported total, otherwise listed recipient facts; with neither it says only “Broadcast”. Expansion mounts the shared OpenedBody shape, two below the row at its text column, with hairline/radius 6/padding 8/12. Recipient chips, reported failures and the optional subject live there; repeated `Re:` prefixes normalize to one. Missing receipts are omitted. No strong rail or clock row is added; the reported timestamp remains a title. Failed chips stay keyboard-reachable, and expand/collapse retains trigger focus.

These components replace the incoming `HarnessNotice` Irc/Delivery presentations and outgoing peer-message/file-change presentations in application adapters. The kit does not parse message envelopes or resolve recipient receipts.

`TranscriptTurn.outgoingMessages` maps host-normalized sends to their underlying work-call ids. The production work log replaces each mapped shell row with its quiet outgoing block and counts it as a message, not a command. Parsing transport envelopes stays outside the kit. The transcript story exercises the folded shell-send replacement; display-hidden reminder routing through “Show system events” belongs to HarnessMarker, not these message surfaces. All variants inherit the nearest Fractal composition theme. The transcript's existing `UserMessage` adopts the exported `Human` variant during grammar adoption after the stacks merge, not in this message-kit change.

The hierarchy and composition stories cover Incoming, Human rail/Card, folded outgoing sends and nested Markdown at 440/1440 in both schemes. Plays reject letter avatars, missing SVG glyphs, nested Cards, old footer/gaps, extra Tab stops, old Markdown margins/indents/rails and soft input ink. Markdown follows C16: sibling rhythm 8, no first/last outer margins, outer list indent 22 and nested indent 20 with a hairline/open-circle marker. Only block quotes use the strong neutral rail. The q106 comparison keeps B1/B2 and pending promotion decisions unchanged.

Message cards and story roots use only composition palette, type, spacing, radius and geometry tokens. Headers establish medium-weight shared UI typography; metadata, tags and plain-text output use the shared meta/dense scales rather than inheriting page defaults.

The file-tool negative control is an existing legacy component, so its own subtree receives the existing legacy dark theme without changing the composition-themed message root. Its production Markdown contrast is checked against 4.5:1, with the prior white-on-white pair as a failing control and a scoped axe check.

Command subtraction uses the projected `countedAsCommand` fact, not merely the run kind: orphan results never contributed to the command count and must not remove an unrelated observed command. Custom work-log owners supply that fact only when the call contributed to their count.

## Families

`kit.tsx` exports these families. Each is labeled, keyboard-reachable and token-driven:

| Family | Notes |
|---|---|
| Button | `primary`/`secondary`/`quiet`/`danger`; `sm`/`md`; square icon-only; disabled |
| Badge, Pill | Labeled semantic status tags; tone is never the only signal |
| StatusDot | `ready`/`building`/`queued`/`error`/`canceled`, always with text |
| Spinner | Labeled `role=status` |
| Progress | `ProgressBar` with label, value, `maxValue`, tone and size |
| CodeBlock | Filename, language chip, copy action |
| CommandMenu | Grouped items with description, keywords and shortcut; `Autocomplete` virtual focus keeps typing in the field while arrows and Enter drive the list; empty state |
| ContextCard | Preview on hover **and** keyboard focus after a 350 ms intent delay |
| Description, Entity | Metadata pairs; named actor chip |
| EmptyState, Note | Honest empty state with an action; severity notes, filled or outlined |
| Input | Controlled; slash hint; Escape clears; clear button |
| Kbd | Shortcut keys |
| Toggle, Checkbox | `Switch`; checkbox with checked, indeterminate and disabled states |
| Menu | `MenuTrigger` with popover, shortcut hints, separators |
| Modal | `Modal`: focus trap, Escape and scrim dismissal, explicit close |
| Table | Row headers, single replace-selection, compact, optional sticky header |
| Tabs | Controlled; `shouldForceMount` keeps hidden panels' state |
| Tooltip | Terse labels, configurable delay and placement, no close delay |
| Icons | `CheckIcon`, `CopyIcon`, `XCircleIcon`: original inline strokes, `currentColor` |

## Tokens

`tokens.css` defines semantic tokens per direction × scheme:

- Color: canvas, panel, recess, ink, muted, line, accent, on-accent, selection, good, warning, danger, added, removed.
- Type: sans, display, mono.
- Radius: control, panel.
- Motion: duration, curve.
- Density: spacing, control height, chrome size. Each direction applies its own pixel bias.

Every foreground/background pair used for text meets WCAG AA (≥ 4.5:1) in all six palettes. The minimum, 4.59:1, is in Relay dark. Under `prefers-reduced-motion`, durations are zero and spinner and pulse animations stop. Fonts come from system stacks only; no font or image assets are distributed.

## Assistant-ui surfaces

### Gated transcript and work log

`Transcript` is the locked U2·F3·Y3 presentation: left-aligned prompts with an accent rule, four-line highlighted tool previews with a host-owned Open action, and a thin synchronization/run progress rail above the header status. Render it under `EmbraceRuntimeProvider` with the same source references used by the runtime. The host supplies `TranscriptTurn` boundaries, work/lifecycle facts, sender captions, retry actions, output navigation and the observation clock. The transcript never creates a transport or turn state machine.

Turns render once the runtime contains their prompt id; adopting replacement snapshot objects with the same ids preserves rows, open disclosures and scroll anchors. Settled work folds; running, failed, interrupted and explicitly incomplete work stays expanded. Notices, events, unknown events, system text, status and usage summaries render as quiet, muted meta-size inline rows in item order, reusing the runtime converter's text summaries without avatars, sender headers or borders. Harness and subagent messages use the U2 Markdown answer row with a muted sender caption: the host's `senderCaptions` entry takes precedence over the item's sender label. No item kind requires the S2 sender presentation inside `Transcript`. Shared failed-send and empty-state content lives in `composition/TranscriptFeedback.tsx`; `Transcript` and `EmbraceThread` do not import each other. Tool rows identify state and observed duration, use non-interactive rows for absent output, highlight commands, and retain an expanded-work divider. Recognized output media types select syntax highlighting; read paths provide an extension fallback for plain or unrecognized media types. Counts appear only when the host reports them or marks the helper's source history complete. A settled answer keeps its copy action and known completion time below the prose; invalid timestamps are omitted and streamed answers have no settled footer. The one response-in-progress status stays at the turn's live edge.

The runtime adopts new messages in a passive effect after React commits the new snapshot, and its store publishes them a task later. Until then, `Transcript` renders each turn's adopted prefix: a turn whose prompt is already published stays mounted with the items up to the first pending id, and its work calls are filtered to those items. Appending a tool call, reasoning or answer therefore never unmounts the existing turn, prompt, open disclosures or scroll position, and never flashes a fallback. `EmbraceRuntimeProvider` reads the runtime's message ids right after each adoption; an id that is absent from the runtime once its snapshot has been through a commit (an adapter that filters it, a duplicate id) is stranded, not pending: it renders in place as a compact muted row with the item's own text, or **Couldn't display this entry** when it has none, later items keep their order, and development builds `console.warn` the ids. There is no timeout; the signal is the commit epoch plus the runtime's adopted ids. Both are consulted only for ids the store has not published: the provider exposes its adopted ids as a store, and the epoch is recorded only when an unpublished id is not held, so a fully published transcript mounts in one commit. Stranded tool calls and reasoning stay in the work log, which reads the turn rather than the runtime. The dark/light `AppendedItems` stories assert node identity and no fallback flash with a `MutationObserver` across each append; the dark/light `StrandedItem` stories withhold ids from the adapter and assert the fallback rows, their order and the warning.
The adoption subscription uses the same snapshot getter for server and client rendering. `test:transcript` checks `renderToStaticMarkup` for both empty history and an unpublished prompt using the real runtime provider and the kit's StyleX transform.

Sender captions are paragraphs (`message-sender`), not headings, so embedding a transcript inside a Details surface never introduces a skipped heading level. `SemanticItems` checks the Host and Review delegate captions in both schemes.
Transcript examples use uniquely labeled state regions so `AllStates` owns the only page main landmark. Its dark/light plays assert one main and nine distinctly named conversation regions. `Expanded` asserts exactly two populated tool-detail previews (read and run), including highlighted read source, rather than assuming that a work log has only one preview. Opening the read detail checks the exact source bytes in its `<pre>`, preserving whitespace.
Expanded tool details show the observed human summary and status first. **Show raw input/output** is a second disclosure; diagnostic command/output bytes and the four-line highlighted preview mount only after that action. The host's **Open** action remains inside the raw panel.
Failed tools and the work-log failure banner show one compact readable reason, skipping path dumps and traceback frames and abbreviating paths. Full command/output diagnostics stay behind **Show raw input/output**, including beside proposed diffs or Markdown. Path-only output uses an honest generic failure reason.

The host also states what it knows about the conversation itself. `availability` (`Available` or `Unavailable` with host-supplied `reason`/`detail`) swaps the thread for a calm inline state. `history` (`Complete` or `HasOlder`) adds an "Earlier messages not loaded" boundary; "Load earlier messages" appears only when the host passes `onLoadEarlier`. A user `TextItem` may carry `sendState` (`Sent`, `Pending`, or `Failed` with `reason`/`detail`): pending prompts render muted, failed prompts show the reason as a danger line with detail on disclosure, and the item keeps its id from Pending to Sent so the server echo replaces the row in place. `emptyState` (a node or `{ title, body }`) replaces the neutral "No messages yet" once the conversation is live; `EmbraceThread` accepts the same prop. The kit never invents reason copy.
`Unavailable` is the whole status: it shows the title, one `reason` line and no sync line. `detail` may carry diagnostics, so it stays behind **Show details**. The optional `action?: { label: string; onPress: () => void }` renders the host's single recovery button (for example **Try again**); without it no button appears. The state fills only the Transcript's own box, so a host composer rendered as its sibling stays in place; `UnavailableWithComposer` covers that layout.
Both `Transcript` and `EmbraceThread` accept `onRetrySend?: (itemId: string) => void`. Failed-send Retry appears only when this callback is supplied; the host owns the transition to Pending and Sent. Keep the same item id throughout Failed→Pending→Sent to retain the row and its DOM identity. The dark/light `FailedRetry` stories exercise the callback and both transitions.
`TranscriptTurn.prompt?: TextItem & { readonly role: 'user' }` is optional: truncated history or standing-seat output can begin mid-turn. An omitted prompt renders no prompt bubble; the host does not need to invent a message. `MidTurnHistory` covers a first promptless turn below the `HasOlder` boundary.
Omit `onOpenTool?: (call: WorkLogCall) => void` for a read-only transcript: tool rows have no host navigation action; the raw disclosure still reveals recorded details in full. A turn with neither tool calls nor reasoning has no work summary. Initial loading shows **Loading conversation…**; synchronization copy uses user-facing loading and update wording rather than transport phase names.
Failed sends carry `reason: { _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }`. Unknown host causes map to `Failed`, shown as **Couldn't send**. Reason tags stay in a data attribute; optional diagnostic `detail` stays behind the disclosure. Retry requires a callback and a `Failed`, `StaleFence`, or `SnapshotUnavailable` reason; rejection, permission and invalid-message failures never expose Retry.
Agent-authored Markdown never loads images automatically. Each image defaults to an alt-text/host placeholder with a per-image **Load image** action; links use `target="_blank"` and `rel="noopener noreferrer"` without prefetch, and raw HTML is skipped. `Markdown`, `Transcript`, and `EmbraceThread` accept `resolveImage?: (src: string) => { _tag: 'Load'; src: string } | { _tag: 'Defer' }`. A host can explicitly allow its attachment URLs; no URL shape or origin is implicitly trusted. Loaded images suppress referrers, and `srcset`, raw HTML/CSS URLs and content-driven previews are not rendered. `Content network safety` stories exercise both transcript boundaries under request interception.
Hosts whose CSP blocks remote images inline (for example `img-src 'self' data: blob:`) pass `onLoadImage?: (src: string) => void` alongside `resolveImage` on `Markdown`, `Transcript`, `EmbraceThread` and `EmbraceMarkdownPreview`. The placeholder action then reads **Open image · host** and calls the host with the source instead of loading it inline; the host opens it in a new tab with `noopener,noreferrer`. Nothing is fetched before the click, and with `onLoadImage` set nothing is fetched after it either (`*HostOpens*` stories). Without it, the inline load after consent is unchanged.
`EmbraceMarkdownPreview` uses that same boundary and accepts the same resolver and opener, including when rendered inside an Embrace tool call. There is no separate permissive tool-output Markdown renderer.
Syntax grammars are a fixed, eagerly imported package set; an authored code fence cannot cause a lazy grammar/module network request.
The syntax palette is neutral: strings, characters, attribute values and inserted diff lines use the muted foreground token (as the review route's string colour does), keywords and links the running foreground, literals the attention token, deleted lines the danger foreground. No transcript-reachable token resolves to green; the dark/light `NoGreenAllStates` stories check the computed colours of every element across all transcript states, including quoted shell strings and inserted diff tokens.

Answers put sender, time and Copy on one 12/16 header line, with the body 2px below and no footer row. Copy keeps its place in the header and its single Tab stop, and fades in on pointer hover or keyboard focus. Consecutive prose blocks from one sender in a turn form a run that shows the header once; its Copy takes the whole run, blocks joined by a blank line. An open work fold indents its children by one 22px text column beside a 1px rail on the header chevron's centre, with no header border or closing divider. Tool rows and Thinking share one capped scroller, so a scrollbar narrows every child row alike. Each row reads glyph slot · title · status word · disclosure · duration. The disclosure sits in a fixed 16px slot at half ink until hover or focus; the 80px tabular duration column ends at the lane edge; successful and running rows show only their duration (`Running` until a running row ends). Failed and interrupted rows keep their word visible just left of the disclosure, so their title ellipsizes earlier. Thinking is an ordinary child row: a glyph, a 14/20 label, and a framed 13px Markdown body in the scheme's muted ink (AA on both canvases) that opens on the text column. Live work reads `Working · 18m 40s` with the shared duration formatter. Markdown separates blocks with one 8px gap and no outer margins; blocks inside a loose list item keep the same 8px gap. Lists use a 22px marker column, 2px item spacing and muted markers; nested lists add a 1px rail and open markers; quotes take a 2px rail and muted ink; code blocks are 12/18 mono with a 6px radius. Tool and Thinking rows fade hover paint over 150ms. Empty disabled composers, plain and token variants alike, collapse to one compact row after their adaptive layout with host guidance in the placeholder; a retained draft that becomes read-only keeps its geometry and shows the reason as visible help text. The reason remains the field's accessible description in both cases. All these transitions become instantaneous under reduced motion. `Live critique r2` and `Hierarchy r1` mount these production components at 440/1440 in both schemes. Their selectable old-behavior controls invalidate the matching geometry, copy, contrast, disclosure, overflow, description or timing assertion after a re-render.
Disclosure tracks keep their full slot geometry when open: only the inner chevron glyph rotates, for both tool rows and Thinking. Reduced motion removes the glyph transition without changing the slot.

The visual-language cutover retains the current seat header until the turn-language pick (C8/E14): `Transcript.showSeatHeader` defaults to `true`; `false` removes the seat prose sender/time/Copy header without affecting incoming message headers or prose-run grouping. `Hierarchy r1/Headerless…` compares that path at 440/1440 in both schemes, with `seat-header` restoring the opposite path after a re-render. C3 uses the single 22px text-column nesting step plus its 1px rail; the obsolete 28px `toolIndent` token is retired. C16/E14 keeps Markdown siblings at 8px with no outer margins; `proseGap` remains retired. C18 v1.1 retires `fgFaint` entirely: DiffPanel disabled/unavailable labels, line numbers and hunk labels, Shell disabled controls and drawer hints/range labels, Sidebar idle dots, and Workbench placeholder hints/chips all use `fgMuted`. The three inks are foreground, soft and muted; secondary labels step down by size rather than a fourth grey. C6 preserves slot · text · bounded status word · pinned chevron · tabular 80px duration, with no success word. C22 uses the SVG glyph family; synthetic tool output uses words instead of Unicode checkmarks. Seat Prose stays soft (C27/E14); input Card bodies are foreground. `railStrong` is used only by block quotes, and nested lists keep the 20px additional indent.
The legacy transcript prose export is named `SeatProse` (E14), distinct from the input `AgentMessage` Card; its callers use the same name, with no compatibility alias.

Added diff rows use the shared blue `statusVars.diffAddTint` token (`#7295ed` dark, `#4269df` light): the gutter marker uses its ink, and the wash mixes that same ink at 9% with transparency. Diff, resource and Embrace preview counts are neutral foreground text. Embrace's `darkTheme` aliases its legacy `added` token to `diffAddTint` and its `good` to the soft foreground on the themed element. The light Embrace defaults carry the light values directly: a default resolves on `:root`, where an alias would read the dark composition defaults. The composition `done` and `diffAdded` aliases are declared inside each scheme's theme, so they resolve against that scheme. The kit's syntax strings already use muted foreground directly, so no green `syntaxStringFg` default is introduced. `DiffTint` carries the route's assertion for a non-green wash, a shared marker/wash token and addition/deletion luminance parity. It also asserts that addition counts paint like deletion counts, and that Embrace `good`/`added` match this scheme's soft ink and tint in light and under `darkTheme`; each check has an opposite-scheme control.

`EmbraceScrollViewport` follows the latest messages until the reader scrolls, focuses or navigates within history. It preserves the visible row anchor through content growth and width changes without invalidating the message subtree. New content while detached reveals a docked **New messages ↓** button; activating it resumes following. `EmbraceThread` uses the same viewport for its non-virtual E3 lane.
While a row action is pressed, the viewport pins that row's screen-relative top through insertions above it, rather than relying on the browser's first-visible-row anchor. The jump dock waits for all pointer releases. Wheel, touch, navigation keys, or an actual reader scroll give scrolling back to the reader and stop press compensation. `Transcript press/InsertAbovePressedRow` covers a reply arriving above a failed send between pointerdown and coordinate-based pointerup; the host still owns turn order.
Press compensation also rebases the ordinary history anchor to the adjusted viewport, so releasing the press and revealing the dock never undo the scroll. The insertion play waits for the pointerdown history capture before inserting and checks row position again after release and after dock layout settles.
Hosts must bound the lane so it owns scrolling; following and ordinary history anchoring remain lane-local. As defense in depth, held row actions compensate through the nearest scrolling ancestor or the document scrolling element when the lane is unbounded. Development builds warn once per mounted lane on the first affected press. `HostInsertRetainedRows`, `HostInsertFreshObjects` and `HostInsertShiftedIndex` retain stable host keys while inserting above the last row; the explicit `HostInsertDocumentScroll` and `HostInsertAncestorScroll` controls cover misconfigured scroll ownership and the warning.

Work-log detail uses the same `toolOutput` extraction as tool previews: strings and native content arrays preserve their text parts in order; empty and non-text results have no detail. `Transcript/NativeToolOutput` covers the expanded row, and `pnpm test:work-log` covers strings, arrays, empty output and ignored non-text parts.
The shared Markdown boundary keeps exactly one streaming caret on the final paragraph line, or immediately after another terminal block. Tool output uses the exported `HighlightedSource` boundary. Failure banners optionally expose a host-owned **Open output** action without clearing or reflowing history.
Failed-command reason lines select the final top-level named exception from a traceback (including names such as `subprocess.TimeoutExpired`), then a recorded exit code, then the readable-output fallback. Traceback frames and indented Python source are never presented as reasons; full diagnostics remain in tool output. `test:work-log` covers timeout and chained JSON errors, truncated traces, exit codes and unchanged path redaction.

### Thread header

`ThreadHeader` shows **Toggle terminal drawer** only when `terminalAvailable` is set. A host that knows why the terminal is unavailable passes `terminalDisabledReason?: string`: the toggle then stays visible but disabled, keeps its exact name, and carries the reason as its accessible description and hover title. Without a reason the toggle stays hidden. `@smalltalk/fractal-ui/assistant-ui/shell` also exports `TerminalDrawer`. The `Thread header` stories cover both cases.


### Portable sync seam

`src/assistant-ui/st3-views/sync-status.ts` defines the decoded observation contract with no Effect import: `SyncStatus`, `SyncStage`, `StaleReason` and `SyncFailureCause`; timestamps are epoch milliseconds supplied by the host. `sync-line.ts` exports `syncLine(input): SyncLineValue | undefined` — the shared status vocabulary — and `observeSyncStatus(previous, status, now)`, the client transition clock. `SyncLine.tsx` exports `<SyncLine>`, requiring explicit `now`/`observedAt` numbers and never reading a clock. Hosts decode their own transport into the plain union.

`Stale.reason` is explicit: an observation without a reason binds `{ _tag: 'Unknown' }`, shown immediately as **Waiting for an update** with its observed age, or **Last updated** with a known last-live age, never an invented cause. Only known `Resync`/`Reconnecting` reasons receive their 400 ms/2 s delay. `Failed.cause` is `{ _tag: 'Server'; code; message } | { _tag: 'Local'; kind; detail?: { cap?: number; message?: string } } | { _tag: 'Unknown' }`. Local and server subscription-limit failures share one plain vocabulary; a missing cap is never invented and a reported cap of zero is displayed. Usage surfaces are HTTP reads and never present subscription-slot failures. Retry appears only for retryable causes; Details shows the decoded cause facts.

Hosts must not invent `Progress` or `Quiet` observations to fill gaps in the wire contract. `observeSyncStatus` retains its timestamp only in memory for the same status/stage/reason; do not persist or hydrate it.

### Sidebar agent row

Rows render reported facts only; unreported fields are omitted — never shown as placeholders — and remain in the accessible details. Line-one metric and trailing-signal tracks share the widest intrinsic content width in their row cohort, so rows without a reported since-time keep the same title start as their cohort; tree nesting supplies hierarchy without a second indentation. Hovering a row swaps the time slot for a quick Open action. The hover card stays compact — status, host, current work, spend, duration, Last turn/Last activity, model, PR, branch and subagents — and never shows raw timestamps.

The status glyph column holds the glyph only; elapsed time in status stays in the accessible details. Each row is the containing block for its visually hidden details trigger, so a long roster overflows only its scroll pane, never the document. `ThreadHeader` omits the breadcrumb folder when the host reports none.

### Markdown and the thinking disclosure

The Markdown boundary renders CommonMark and GFM prose — emphasis, nested lists, tables, linked headings — while preserving resource-chip and inline-reference seams. Fenced code uses the pinned `refractor@5.0.0` dependency with fixed, eagerly imported grammars for the explicit set (TypeScript, TSX, JavaScript, JSON, Bash, diff, Rust, Nix, Python, YAML, Markdown, CSS); authored fences cannot trigger grammar requests, and unknown language labels stay visible above plain source. Each code `<pre>` is keyboard-focusable (`tabIndex={0}`), named **Code, language** (or **Code, text**), and shows an inset semantic-token focus ring. `SemanticItems` exercises an overflowing code block, Tab from Copy code, its accessible name and the visible ring in dark and light. Wrap and copy are local to the stable code-block identity, so streaming text updates do not reset them. Unfinished streaming link tails complete through a linear backward scan of the current line, including escape-run handling, before label brackets are matched. Settled reasoning uses a muted `Thinking` disclosure whose expanded content renders through the same Markdown seam.

`Markdown` takes `tone: 'default' | 'muted' | 'foreground'`. Paragraphs, list items and tables inherit their ink from the Markdown root (soft by default, muted for `muted`, foreground for input Card bodies per C27/E04–E12) or from a quote, so a tone is one themed `color` declaration. `EmbraceMarkdownPreview` forwards the same tone. `Markdown/InputForeground…` covers that forwarding and an in-play soft-ink control at 440/1440 in both schemes. It never re-points theme variables, which would freeze them at the dark default. Quotes and the `muted` tone keep strong text muted; Thinking uses `muted`. A run's Copy follows the Copy primitive: a missing clipboard, a thrown or rejected write is swallowed, and only a completed write sets `data-copied` for 3s. A write that completes after the Copy unmounts or changes message scope is ignored.

### Floating failure layer

`ErrorOverlayHost` defaults to one dismissible floating layer per surface; failures portal into the nearest host so deep banners do not clip or reflow history. Its optional `lane` mode renders notices in place instead and attaches no portal layer. `Transcript` uses lane mode so run failures remain beside their work log without covering history; synchronization failures remain within that host. Escape inside the host dismisses the newest inline or floating banner without moving focus, and a new failure id reappears after a dismissal. Without a host, the work log retains its inline banner.

### Gated workbench and composer

The focused Workbench stories use W3/H1/P3/D2; Composer stories use C2/R3/M2/K1 while exercising the gated alternate states. They do not import the route's sidebar, details fixtures, exploration controls or runtime provider. Workbench thread resources provide the kit's `TranscriptProps` and `ConversationRuntimeOptions` directly: the host owns turns, run state, synchronization facts, observation times and tool actions. This is deliberately different from the route's combined `ConversationState` provider.

Workbench pane identities survive layout changes through persistent pane slots. `workspaceId` scopes stored ratios, tabs and composer drafts, so two workspaces displaying the same pane do not share draft text. `ViewportStore` and `ViewportStoreContext` retain per-conversation scroll/follow/unread state when a thread pane is parked or moved; `Transcript.viewportKey` identifies the owning conversation. The minimal terminal boundary is host-supplied read-only lines/session facts for pane rendering and drag targets, not a terminal transport or emulator.

`DiffPanel` accepts numeric or string widths, optional `onClose` and `landmarkContext`, current-turn and branch file lists, and an explicit `{ path, sequence }` reveal request. Missing file facts remain missing. The host interprets `onOpenTool` and drives controlled diff reveals; the kit never guesses a path from a tool's input. Its scope picker, line-wrap and whitespace controls remain keyboard accessible, with semantic-token React Aria portals owned by `ThemePortal`.

`DiffPanel` is exported from `@smalltalk/fractal-ui/assistant-ui`, not from `@smalltalk/fractal-ui/assistant-ui/shell`. Its source highlighting loads the Markdown grammar graph, and the shell entry stays runtime-free. `pnpm check:shell-imports` walks the shell entry's import graph and fails if Markdown, refractor or the assistant-ui runtime becomes reachable.

The composer preserves kit send, cancellation, byte-limit and token-history contracts while adding adaptive pill/slab geometry, grouped mention/command popovers, effort selection, and the recipient/model toolbar. `ComposerSession` owns only device-local draft persistence and composer interaction policy; transports and runtime facts remain host-owned. Its tooltips retain the gated 150 ms delay and viewport-clamped, wrapping contents; Workbench/DiffPanel share the 350 ms Controls tooltip instead.

The Workbench thread dock uses the reviewed route's composition composer policy, distinct from `ComposerSession`'s R3 choice: idle Enter sends immediately; during a run Enter queues and modified Enter explicitly steers. `IdleSample` seeds the route's exact `designItems('idle', 'sample')` messages with explicit idle host facts. `RunningQueueAndSteer` keeps the running case separate and asserts both actions without changing the queue. Neither fixture infers a runtime state from a header label.

State and draft codecs use the workspace's single `effect@4.0.0-rc.118` runtime and the matching `@effect/atom-react@4.0.0-rc.118` binding. `react-aria@3.52.1` is declared explicitly for the separator and portal APIs already used by the kit. Performance counters and `RenderProfiler` stay internal rather than becoming package-root exports.

The codecs use native `Schema.optional`, preserving both absent fields and explicitly present `undefined` fields from the original Effect 3 contract. `OptionalSnapshotFields` exercises the real pane-key parser and snapshot encode/read path, including form-only, view-only and undefined-ratio inputs.

The `Geometry` story is a non-interacting fixture for the reusable composer and its layout/running/target/mention matrices. It lets the source geometry assertions measure the same native composer before story play actions mutate its state. Its plain 768px host lane preserves the composer/body width and footer/popover containment assertions without shipping the route's Explore controls, conversation or application chrome. The single lane fills the viewport height and bottom-anchors the composer like the reviewed single preview, so grouped mention popovers have the same room above the composer. `KitLabelResize` returns to its short label after its own play, so external gates measure the real label growth.

The exported legacy Folio, Relay and Orbit palettes also avoid green: `good` is the direction's 80% ink foreground; `added` is a non-green wash whose luminance difference from the panel is within 20% of the direction's removed wash. Run `pnpm --filter @smalltalk/fractal-ui test:palettes` to check all six direction/scheme combinations without a browser. The public names and palette-selection API are unchanged.

The shared diff tint assertion identifies a token through its causal effect on both the row wash and gutter, letting the browser resolve CSS layers, specificity and conditional rules. It restores exact inline styles and verifies computed paint restoration after each probe. Every dark/light tint story also rejects the original neutral pre-tint paint and an equal-blue wash backed by an unrelated variable, then verifies the restored positive result.

### Transcript turn exploration

`fractal-ui-explore-transcript-turns--exploration` compares three turn structures, three input-kind cues and four time/copy placements. Every dimension has the shared controlled `VariantPicker` contract and a Compare toggle; selections are Storybook args, including dark/light and 1440/900 host lanes. Link to `/iframe.html?id=fractal-ui-explore-transcript-turns--exploration&args=turn:T1;kind:K1;meta:M2;scheme:light;width:900`.

The public pairing stories have no play and preserve deep-linked args without automated interactions or failing-control specimens. `ExplorationProbe` and the explicitly named `*Probe` stories mount their own state and failing controls for the control-driven assertions.

`fractal-ui-explore-transcript-turns--recommended` is the pending-ratification T1/K1/M1 preset, using compact K3 only for system notices. Relays and agent mail remain full native input cards, and seat prose has no sender line. The Recommended button applies the same deep-linkable args; choosing an alternative clears the preset without deleting any T/K/M option. The preset has no play; `RecommendedProbe` exercises its own state.

M1 puts input time and Copy in the native input header. At the responsive metadata token's 48rem breakpoint, seat time and Copy share a zero-height-reservation toolbar floating 24px above the Prose run's top-right corner, with a canvas fill and hairline border. A pending design choice lets it overhang right by the shared 32px panel spacing in spare canvas margin. Below that breakpoint, it becomes a right-aligned, in-flow line immediately above the run, explicitly reserving 24px rather than risking text overlap or off-canvas placement. This narrow fallback is a deliberate exception to the zero-meta-height desktop alternatives. Probes use the real 8px within-turn gap and reject intersection with any preceding text line at 440 and 1440; removing the overhang from a collision fixture fails the same predicate. Toolbar bounds and document width reject clipping or added horizontal scroll. Native Card framing/header is rendered once rather than nesting a Card in a projected Card. All kind, disclosure and Copy glyphs reuse the production outline SVG family. Input bodies use foreground ink; the explorer uses only foreground, soft and muted text roles. Probe controls deliberately reproduce nested framing, a covering toolbar and a non-SVG glyph to reject each regression.

The explore-only projection reuses the incoming/human `AgentMessage` family, the production `Transcript.SeatProse` row and its Markdown renderer, and the `WorkLogV1` fold. It does not promote a variant into production. Input cards, chat alignment and sender-gutter language are alternatives; header, gutter, gap-divider and inline-trailing metadata all omit a footer. Hover, focus-within and touch tap reveal the same Copy/time affordances without another metadata tab stop. Compact input bodies mount only while disclosed, so collapsed panels do not retain a second hidden Copy/time projection. Repeated body landmarks include the block and variant identity in their accessible names. The fold retains a 22px nesting step, a rail and fixed disclosure/status columns.

The production answer row exposes `showMeta?: boolean` (default true). The explorer passes false so each metadata choice owns its time and Copy without mounting a second production header or footer; a control restores production metadata. The original K3 relay/system alternative keeps a full-foreground title, w500 sender and SVG kind glyph in one non-wrapping line, with the title truncating and a minimum 24px Row height. The Recommended system-only alternative is a 20px Marker in muted 12px ink. Plays bound each to one line box and require one text-run client rect; a stacked control fails both checks. Quiet/undersized and heavy-sender controls fail the input-cue assertions. The production hierarchy fold is reused directly, without a second rail or status projection.

Pointer reveal uses React Aria's `useHover` state alongside focus-within and touch tap. This lets native pointer movement and Storybook's synthetic pointer events exercise the same affordance state instead of comparing synthetic events with the browser's independent CSS `:hover` flag.

The fixture retains the screenshot's four relay/reply cycles verbatim except for equal-length replacements of scanner-flagged person names, host names and repository namespaces. The approved clearer reference supplies the fourth reply at 9:49; signed person, titled agent mail, system notice and tool-fold additions remain synthetic. The `MetaM1Probe` through `MetaM4Probe` plays measure block-height deltas and time ownership against the next block's rectangle. All plays reject reserved-footer, next-input timestamp, unstyled-token, hover-only and deliberately remounting controls. Stable message component identities keep focus and trailing timestamp portals attached across state changes; M4 asserts one time per block (11 total), with a missing-time control. Plays operate Recommended, every picker, metadata Compare, scheme, width and compact-relay disclosure, wait for changed lane attributes/counts or expanded paragraph timestamps, then repeat geometry and affordance assertions. `AllStatesProbe` starts with the full dimension matrix before exercising the controls.

## Clean-room note

The original kit's behavior references are listed below. The gated workbench/composer additions reuse our independently authored internal review route at `98a270d4e58a44e4c4625cf1956ad09dbabfc1a7`, with the reviewed tint cutover from `35b6922a1559b4b3bcd78b672cb6b707c5d1e108`; their host/runtime boundaries are adapted to this kit as described above. This provenance is not a claim that the later additions were authored without consulting our own source.

The original workflow requirements cited public product documentation of coding-agent workflows; those behavioral references are separate from the internal-source port described above:

- [Coding-agent app workflow introduction](https://openai.com/index/introducing-the-codex-app/): parallel threads, isolated worktrees, review before landing.
- [Zed agent panel](https://zed.dev/docs/ai/agent-panel): keyboard-first palette, per-change review, explicit execution state.

Per direction, the metaphor and the behavior it emphasizes are:

- **Folio**, an annotated working notebook: warm paper, plum annotations, serif headings, deliberate 180 ms easing. It emphasizes progressive disclosure of tool input and output, and an explicit review checkpoint.
- **Relay**, a dispatch desk: mineral surfaces, teal signals, squared 2 px geometry, compact rows, 90 ms linear motion. It emphasizes keyboard-reachable actions, discoverable commands and visible execution outcomes.
- **Orbit**, a navigation instrument: indigo layers, amber bearings, 14/20 px curves, roomier spacing, 240 ms settling motion. It emphasizes keeping context while moving between threads, review and execution, and text-labeled attention.

The original kit fixtures use newly written synthetic data. The transcript turn explorer additionally uses the screened excerpt and substitutions described above. The terminal is a read-only fixture, not an emulator. Nothing contacts a filesystem, network service, model or provider.

This note records provenance practice. It is not legal clearance or an independent originality certification.
