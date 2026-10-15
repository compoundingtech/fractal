# Quick-note UX variants

Interview evidence for [CR01, DQ3 and DQ5](./spec.md). These are draft
prototypes, not a surface or audience decision.

| Source | Review scope |
|---|---|
| [Smalltalk #2867](https://github.com/compoundingtech/smalltalk/pull/2867) (draft) | Web kit stories in `packages/fractal-ui/src/notes/` |
| [dotfiles #5916](https://github.com/schickling/dotfiles/pull/5916) (draft) | Fractal TUI in `flakes/fractal/crates/fractal/src/` |

## Input count

Start with the intended target selected and the relevant surface focused.
`N` is nonblank text typed with N character inputs, within 1024 UTF-8 bytes.
Count a shortcut chord as one gesture, not separate physical key presses.
Navigation to the target, corrections and retries are not included. An
agent note here means a person writes with audience `agent`, not an agent author.
All four composers open with audience `self`.

| Variant | Entry | Text | Self toggle | Confirm | Self total | Agent toggle | Agent total |
|---|---|---|---|---|---|---|---|
| V1 Web inline | 1 click on **Add note**, or Enter on the focused button | N | 0 | 1 Enter | `1 + N + 0 + 1 = N + 2` | 1 Alt+A / Option+A | `1 + N + 1 + 1 = N + 3` |
| V2 Web popover | 1 unmodified lowercase `n` outside a text field | N | 0 | 1 Enter | `N + 2` | 1 Alt+A / Option+A | `N + 3` |
| T1 TUI inline row | 1 `n` from the selected agent or mission row | N | 0 | 1 Enter | `N + 2` | 1 Tab | `N + 3` |
| T2 TUI bottom bar | 1 `n` with the subject's drawer focused | N | 0 | 1 Enter | `N + 2` | 1 Tab | `N + 3` |

Web also permits one click on **Shared with agent** instead of the audience
shortcut. Clicking **Add note** confirms instead of Enter. Returning focus
to the text after a pointer toggle adds an extra click if Enter is used there.
T2's count excludes opening and focusing the drawer. V2's count excludes
selecting a different row; focus alone does not change its selected target.

## Web V1 — inline quick composer

Story: `fractal-explore-notes-inline-quick-composer--all-states`.
Width examples: `--light440`, `--light1440`, `--dark1440`.

| Aspect | Behavior |
|---|---|
| Entry | **Add note** below the detail pane's Notes list; button starts focused in the story. |
| Focus | Opening focuses the textarea. Successful add closes the composer and focuses the remounted **Add note** button. |
| Escape | From the textarea, an empty draft closes; a nonempty draft stays open with its text. |
| After add | Full text becomes the newest list item. The target header badge updates its count and newest one-line preview. |
| Audience | Composer: **Only me** / robot glyph + **Shared with agent**. List: the same labels, robot only for agent audience. Badge: robot only when the newest note is agent-audience; self has no visible audience label. |
| Weaknesses | Entry is below the list and may need scrolling. Open composer takes detail space. Empty-only Escape differs from V2 and the TUI. A self note can remove the badge's robot even if older agent notes remain. |

## Web V2 — keyboard `n` popover

Story: `fractal-explore-notes-keyboard-n-popover--all-states`.
Width examples: `--light440`, `--light1440`, `--dark1440`.

| Aspect | Behavior |
|---|---|
| Entry | Unmodified lowercase `n` opens a popover anchored to the selected row. A visible **Press N** hint and row `aria-keyshortcuts` expose the shortcut. Uppercase Shift+N does not open it. |
| Focus | Opening focuses the textarea. Successful add closes and returns focus to the selected row. Typing `n` in an input, textarea or editable field does not open it. |
| Escape | Cancels the popover even with a draft; focus returns to the row. A refused create keeps the popover, text and audience, focuses the input and announces an error until dismissal. |
| After add | The selected row badge updates count and newest text preview. This story does not show a full note list or a badge action to open one. |
| Audience | Same composer labels and robot glyph as V1. Badge robot describes only the newest note; the accessible name also names that audience. Full newest text is available in a focus/hover tooltip. |
| Weaknesses | No pointer-only entry control. Selection can differ from focus; the shortcut acts on selection. The popover obscures nearby content. New text is only a truncated preview until its tooltip is read. Escape can discard text. |

The planner target deliberately refuses creation in V2. This is an error
fixture, not a rule about which agents accept notes. Both web variants use
`NoteComposer`: Enter adds, Shift+Enter inserts a newline, Alt+A changes
audience, and a successful submission resets audience to self.

## TUI T1 — inline row composer

| Aspect | Behavior |
|---|---|
| Entry | `n` from an agent or mission sidebar row. A non-Smalltalk seat cannot accept a note. |
| Focus | The one-line editor owns input and the terminal caret. Target is fixed when opened. Submit or cancel closes it and leaves the underlying browse focus in place. |
| Escape | Always cancels and discards the draft, including nonempty text. |
| After add | Row count updates immediately, including pending sends. Drawer Notes shows `sending` while pending, then the newest note text (up to three wrapped lines) and older one-line entries. If the drawer is closed, the row has only a count and delivery arrow, not a text preview. |
| Audience | `[me]` or `[agent→]` in the composer and drawer. Row `✎4→` means four notes, at least one agent-audience note; no arrow for self-only notes. |
| Weaknesses | Narrow sidebar scrolls the draft horizontally and drops key hints. The composer clears/overlays a row slot below, above, or on the selected row rather than reserving new list space. If no row slot is available it falls back to the bottom bar. |

Unicode snapshot, `ui/snapshots/fractal__ui__st_notes__tests__snapshot_st_note_composer_row_unicode.snap`
(relative to the Fractal `src/` directory):

```text
╭ 1 agents (2)·2 ptys·3 missions ────┬ 1 Session · 2 Definition ─── no session selected ───────────╮
│/   fuzzy · repo: state: tool:  ⏎ ke│                                                             │
│▼ [example-host]                    │  Select a PTY to show its session.                          │
│┊   ⋯ agent/example/pending unread  │                                                             │
│┊   ▲ example ✎4→ unread            │                                                             │
│ ✎ [agent→] up from the review step │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
├ 1 Quota·2 System·3 Agents·4 GitHub ┤                                                             │
│set --omp-usage-endpoint, FRACTAL_OM│                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
╰────────────────────────────────────┴─────────────────────────────────────────────────────────────╯
 browse no selection  / filter  d drawer  ? help  q quit                        0.1.0+test · unknown
```

The fixture has already toggled to agent audience. Its draft is
`Pick the rollout up from the review step`; only the tail fits in the row.
The generic browse footer remains visible, not the composer's key hints.

## TUI T2 — bottom-bar composer

| Aspect | Behavior |
|---|---|
| Entry | `n` from the focused drawer opens a composer in the bottom status bar for that drawer's subject. |
| Focus | Same input ownership as T1. Submit or cancel returns input to the existing drawer focus. The bar names its fixed target. |
| Escape | Always cancels and discards the draft. |
| After add | Same pending row count and drawer Notes update as T1. The open drawer keeps the newest text close to the add flow. |
| Audience | `[me]` / `[agent→]`, Tab changes audience. Same aggregate arrow on sidebar markers as T1. |
| Weaknesses | Needs a focused drawer before the one-key entry. Composer is away from its target row and replaces the normal footer. Hints disappear when the bar lacks space. Long target labels are shortened. |

Unicode snapshot, `ui/snapshots/fractal__ui__st_notes__tests__snapshot_st_note_composer_bottom_bar_unicode.snap`:

```text
╭ agents (2)·ptys (1)·missions (1) ──┬ 1 Session · 2 Definition ─── no session selected ───────────╮
│/   fuzzy · repo: state: tool:  ⏎ ke│                                                             │
│▼ [example-host]                    │  Select a PTY to show its session.                          │
│┊   ⋯ agent/example/pending unread  │                                                             │
│┊   ▲ example ✎4→ unread            │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    ├Session·Resources·Conversations·Missions──────[e] events  [x]┤
│                                    │── notes (4)  [n] note [j/k] select [x] archive              ▐
│                                    │› [agent→] · you · 10-14 08:42Z                              ▐
│                                    │    Retire this session after the handoff lands; start the   ▐
│                                    │    next one from the rollout notes.                         ▐
│                                    │  [agent→] Handoff written… · agent/example · 10-14 08:30Z   ▐
│                                    │  [me] Check the quota before Friday. · you · 10-14 08:21Z   ▐
│                                    │  [note] Pairing with the review step… · you · 10-14 08:12Z  ▐
│                                    │                                                             ▐
│                                    │── pty ──                                                    ▐
│                                    │  session              example-a1                            ▐
│                                    │  geometry             100x30                                ▐
│                                    │  clients              total=1 attached=1 readOnly=0 neutral=▐
│                                    │── st harness ──                                             ▐
├ Quota · System · Agents · GitHub ──┤  observation          not reported                          ▐
│set --omp-usage-endpoint, FRACTAL_OM│  activity             needs-you                             ▐
│                                    │  open attention       0 item(s), not live asks              ▐
│                                    │                                                             ▐
│                                    │Decisions                                                    ▐
╰────────────────────────────────────┴─────────────────────────────────────────────────────────────╯
 note on example [me] Ask about the quota on Friday              Tab me/agent  Enter add  Esc cancel
```

### Shared TUI row marker

Unicode snapshot, `ui/snapshots/fractal__ui__st_notes__tests__snapshot_st_note_row_marker_unicode.snap`:

```text
╭ 1 agents (2)·2 ptys·3 missions ────┬ 1 Session · 2 Definition ─── no session selected ───────────╮
│/   fuzzy · repo: state: tool:  ⏎ ke│                                                             │
│▼ [example-host]                    │  Select a PTY to show its session.                          │
│┊   ⋯ agent/example/pending unread  │                                                             │
│┊   ▲ example ✎4→ unread            │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
├ 1 Quota·2 System·3 Agents·4 GitHub ┤                                                             │
│set --omp-usage-endpoint, FRACTAL_OM│                                                             │
│                                    │                                                             │
│                                    │                                                             │
│                                    │                                                             │
╰────────────────────────────────────┴─────────────────────────────────────────────────────────────╯
 browse no selection  / filter  d drawer  ? help  q quit                        0.1.0+test · unknown
```

These are synthetic fixtures: `example-host` is an invented grouping label,
not a machine address. The corresponding story snapshots are
`stories/snapshots/fractal__stories__tests__st-notes__{composer-row,composer-bottom-bar,row-marker}.snap`.
They show 120×40 and 80×24 terminal frames; the embedded UI frames are 100×30.
At 80×24, the T1 story hides the sidebar and paints the composer in the
bottom bar instead. This is fallback behavior, not evidence of an inline
composer working at a 440 px browser width.

## Comparison

Counts are measured from the entry context above. Other judgments follow
source, story assertions and stored artifacts; they are not new runtime tests.

| Variant | Input overhead | Discoverability | Visibility after add | Risk of wrong audience | Keyboard-only use | Works at 440 px |
|---|---|---|---|---|---|---|
| V1 | Self N+2; agent N+3; may need scroll to entry | Explicit **Add note** | Full list + newest badge | Self default is quiet; explicit labels; badge only describes newest | Yes, from focused button; no global `n` | Stored 440 px view shows list and entry; composer footer wraps by source |
| V2 | Self N+2; agent N+3; row selection is extra | Visible N hint; no pointer entry | Count + truncated newest preview/tooltip | Same default; hidden composer until entry; newest-only badge | Yes; lowercase `n` only outside fields | Stored 440 px view shows rows; popover width is viewport-bounded by source; open popover not shown in artifact |
| T1 | Self N+2; agent N+3 | Help/binding; inline hints may not fit | Count/any-agent arrow; text in drawer | Self default; short chips; aggregate arrow can refer to an older note | Yes; Tab is audience, not focus navigation | Not established: terminal uses columns, not CSS pixels; narrow row clips draft and hides hints |
| T2 | Self N+2; agent N+3; drawer setup is extra | Drawer `[n] note`; bottom-bar hints if space | Count/any-agent arrow + open drawer text | Same self default and chips; reopened failed draft resets audience | Yes; same bindings as T1 | Not established in pixels; wider input line than T1, but target and hints consume columns |

## Differences to discuss

| Area | Web stories | TUI behavior |
|---|---|---|
| Audience shortcut | Alt+A / Option+A; Tab navigates controls | Tab toggles audience and stays in editor |
| Escape | V1 protects a nonempty textarea draft; V2 cancels it | Both T1 and T2 discard it |
| Add completion | Waits for create resolution; failure keeps open editor, text and audience | Closes on Enter before worker result; pending marker/list entry appears; failed text is saved and `n` reopens it with default self audience |
| Delivery marker | Robot refers to newest badge note | Arrow means any counted note reaches the agent |
| Text visibility | V1 full list; V2 newest preview/tooltip | Row has no preview; drawer newest text is capped at three lines |
| Text editing | Shift+Enter adds a newline | One-line editor; no equivalent multiline affordance |
| Target model in examples | Agent detail or selected agent row | Agent and mission rows; mission summary also includes run/step notes |

## Local web review artifacts

Files below are under `tmp/st-notes-web/screenshots/`, are gitignored, and
are not committed. Each basename expands over `{light,dark}` and `{440,1440}`:

| Surface | Screenshot basenames |
|---|---|
| V1 | `fractal-explore-notes-inline-quick-composer-{light,dark}-{440,1440}.png` |
| V2 | `fractal-explore-notes-keyboard-n-popover-{light,dark}-{440,1440}.png` |
| Badge | `fractal-plugins-notes-note-badge-{light,dark}-{440,1440}.png` |
| List | `fractal-plugins-notes-note-list-{light,dark}-{440,1440}.png` |
| Composer | `fractal-plugins-notes-note-composer-{light,dark}-{440,1440}.png` |

These are final story-play states, not a recording of every interaction.
In particular, the V1 artifact shows a successful add and the V2 artifact
shows the popover closed after the error/cancel path.
Supporting story IDs: `fractal-plugins-notes-note-badge--all-states`,
`fractal-plugins-notes-note-list--all-states`, and
`fractal-plugins-notes-note-composer--all-states`.

## Questions for the interview

| Question | Options | Evidence to compare, without a decision |
|---|---|---|
| **DQ3: default audience?** | Self; agent | All prototypes default to self: N+2 for a private reminder, N+3 for agent context. Agent default reverses that cost but can send an intended private reminder to the agent. The spec's retirement example can be a private reminder. Ask which intent is more frequent and which mistake is worse. |
| **DQ5: tags?** | Plain text only; optional tags such as `retire`, `watch`, `context` | All four prototypes require only text. No prototype supplies a tag control or measures its cost. Ask whether filtering by tag is needed, and whether an optional tag must be reachable without adding a required step to CR01. |
| **Web surface choice?** | V1 inline detail; V2 selected-row popover | Same base count. V1 has visible entry and full post-add text; V2 avoids opening detail but needs shortcut knowledge and shows only a badge. Compare frequent list work, pointer use and 440 px entry. |
| **TUI surface choice?** | T1 row; T2 bottom bar; retain both by focus context | Same base count. T1 stays near the target but clips text and hides hints; T2 has more width and an open notes list but needs drawer focus. These are two entry contexts in the same implementation, not a selectable preference. |

Ask for one self reminder and one agent-context note on each surface.
Observe entry discovery, target selection, audience checking and whether
the added note can be found without another explanation. Do not infer a
preferred variant from equal keystroke totals alone.
