# Fractal for Mac

A native macOS 14+ SwiftUI menu-bar client for one person's Fractal: the
attention inbox, the person's availability and directive note, and the
objectives every Fractal client reads. It reads only Fractal, through
`FractalClient`, at launch and every 30 seconds; Fractal streams the attention
collection, but `FractalClient` has no typed attention or availability stream.

- **The panel** is a Control Center module: the existing Availability, Speech and
  Objectives tiles, a height-capped inbox, ten signal tiles in two columns, and
  the footer. Inbox rows show symbol, title, asker and age; drafts, writes in flight
  and refusals are marked on the row. Overflow opens a full scrollable inbox via
  “N more”; the home cap uses the screen's visible height minus a 12 pt margin
  and measured tile/footer chrome. At least three inbox rows stay on short screens,
  which then scroll; tall screens show more rows while keeping all signals in view.
  Selecting a card widens the panel to a 760 pt list | detail split: the 360 pt
  list remains mounted at its scroll position and the 400 pt detail column has
  a scrolling body and a sticky decision footer. Selecting another card replaces
  the detail; Close or Esc returns to the list. Other tiles still push pages
  with Back navigation. One native frame policy pins the top edge and, when
  widening, the leading edge, clamping to the visible screen; Reduce Motion
  makes the resize instant. Closing the panel returns it to the top level.
- **Inbox tabs** split the inbox into Asks, Updates and Notices, each with its
  count; `InboxKind` classifies on wire fields only, never titles. Asks are every
  open request (`Role.ask`); updates are `update.type` `update` sent by an agent
  (`requester_id` `agent/…`); every other update is a notice, such as the runtime's
  `daemon/runtime` word that an ask ended (`update.about` `step-run/…`). The home
  label's line carries the three as plain text tabs (no added height) and the capped
  rows follow the selected one; More opens the full inbox on that tab. The full
  inbox puts its Activity overview above a segmented All · Asks · Updates · Notices
  control; Asks group under `AskGroup` sub-headers (decisions, choices, feedback,
  questions, then reviews, prompts and forms answered in Fractal), and All lists
  each kind under its own label. Both surfaces open on Asks and return to it when
  the panel closes. Gallery scenarios `inbox-tabs-*` cover home, each page tab, an
  empty tab, and both with and without activity.
- **Inbox activity** counts arrival and resolution claims in the generic Fractal
  series response, grouped by ask/update type. Arrivals stack above the axis and
  resolutions below, with asks in the stronger tone, on one scale for every tab:
  the selected tab's series stays full and the other fades (notices count as
  updates; All keeps both full). The home label carries a
  fixed-height 14-day strip; the full inbox adds a 14-day daily / 24-hour hourly
  picker and per-bucket hover values. Missing activity data keeps the home label
  and full inbox Activity section visible with an explicit unavailable notice,
  never a fabricated zero chart. Failed reads retain the last chart with an offline
  notice. Live history requires Fractal's typed arrival and resolution events for
  every inbox row source, carrying ask/update type and person on both directions;
  the live adapter intentionally waits for that complete contract rather than
  substituting fleet-wide person-work counts. Gallery scenarios
  `inbox-activity-{busy,quiet,unavailable}-{home,page}` exercise both appearances at 360 pt.
- **Asks** are classified by the wire kind and offered actions. Person steps
  are answered in the detail footer: pick a named answer, then Send or ⌘Return.
  The recommendation is preselected; 1/2/3 pick while the detail has keyboard
  focus, not while typing. Picking never sends, and bare Return never sends.
  Declines retain their destructive tint; choices requiring words cannot send
  without them. Custom words and feedback use the same fenced send path.
  Untyped questions offer Yes/No; asks without named answers offer Nothing to Do.
  Custom forms, human reviews and harness prompts link to Fractal.
  A committed answer or control returns to the top level with a receipt row.
  Typed answers and the note draft live on the model across panel closure.
- **Details open synchronously** from the complete request snapshot already
  returned by `attention.list`: question, why you, summary, reasons,
  recommendation, subjects and attachments. Background reads enrich the work
  step, GitHub subject status and conversation independently; they never gate
  the first frame or show an initial loading state. One coalesced per-revision
  stale-while-revalidate cache retains up to 12 entries with 15-second freshness.
  While the panel is visible, serial prefetch warms the top five asks/updates.
  Enrichment does not reread `attentionGet`; the authoritative submission fence
  still does. Failed reads retain their last value and expose their refusal.
  These remain existing projection reads, with no schema or composer changes.
  The readable thread attributes each turn, collapses delivery envelopes into
  “Prompt from X” chips, hides reasoning behind a toggle, and shortens GitHub
  links to `repo#N`. Adjacent delivery copies are deduplicated; each prose turn
  has a four-line cap with Show More, and Earlier turns in Fractal opens the
  full conversation. The question and work-step title are not repeated.
  Image bytes remain lazy; raw image data leaves the model with its card and
  decoded images share an evictable 32 MiB cache charged by decoded pixels.
- **Snooze and Dismiss** are the person's own attention controls
  (`attention.snooze`, `attention.dismiss`), offered when the session is
  granted them, on every ask's page (and Snooze on an update's), never as an
  answer.
  Snooze (1 hour, 4 hours, until tomorrow 9:00) hides the card on all the
  person's clients until then; its deadline still runs. Fractal still lists
  snoozed rows: the tray leaves them out of the list, the count and speech (a
  snoozed item being spoken stops; a queued one is dropped), says how many are
  snoozed and when the first returns, and re-renders when the earliest snooze
  ends. Dismiss sets a person-step ask aside with Fractal's own `dismissed`
  outcome, which its asker reads; Fractal refuses it for anything else
  (`attention-dismiss-unsupported`) and the tray shows that refusal verbatim. A
  card whose row is in the panel while the panel's window is on screen (its
  occlusion state, not the row appearing: the closed window-style panel may keep
  its rows) is marked seen (`attention.seen`) once per revision, unless Fractal
  already has it seen. Controls are fenced and keyed exactly as answers are
  (below).
- Answers mirror Fractal web `alert-action-port.ts`: re-read the card, require it
  open, offering `work.done` and at the revision the person read, then submit
  one `work.done` fenced on that read. The action ID and idempotency key are
  derived from the card revision and the whole answer, so a repeated send of the
  same answer replays Fractal's first receipt. A card shows "Sending…", then
  "committed" only after Fractal's result, or Fractal's refusal verbatim; it is
  never removed optimistically.
- **Updates** are listed apart and not counted; Mark Read sends the server's
  prefilled `work.done` (`Read`, answer `read`), which is also what dismissing
  an update does, so updates offer Mark Read and Snooze.
- **Objectives** (`GET v1/client/slo`): rows by Fractal's served tone, missed
  first, then stale, then met; unmeasured objectives collapse; budgets follow.
  They stay in the panel: the status item shows asks only (one template glyph
  and the count; offline swaps in the sync-problem glyph, keeping the count).
- **Signals** are a fixed glance board over the same Fractal SLO rows. One table in
  `SignalBoard.swift` maps served IDs to the ten tiles; each row joins only the first
  tile that names it:
  - PR throughput: `factory/merged-prs`.
  - Merge latency: every `…/merge-latency/…/p50` and `…/p95` row, for example
    `fractal/merge-latency/p95` and `landing/merge-latency/example/p50`.
  - CI critical path: `…/ci-critical-path/p95` and `build/admission/HOST/CLASS`.
  - Flakes: `flakes` and `…/flakes`. Main green: any `main-green` segment.
  - Fleet: `fleet/…`, including `fleet/roll-completion`. Agents: `agents/…`,
    including `agents/mail-delivery/HOST`. Disk: `disk/…`.
  - Fractal: the daemon's native rows (`person-read*`, `projection-*`, `write-ack`,
    `terminal-attach`, `sql-statement`, `transaction/…`, `writer-hold`, `cpu`,
    `cpu/idle`, `database`, `database-archive`), `ios-*` phone rows and
    `fractal-api-readiness[-early]/MEMBER`. An explicit `fractal` objective joins
    these rows as one more member; it never hides them or masks a worse child.
  - Spend: every budget row plus the `CAM-R01/…` and `CAM-R02/…` credit objectives.

  The worst served tone determines the tile's tone and subtitle; labelled parts
  (`…/p95{member=x}`) stay out so they are not counted twice. Multi-member signals
  show missed/measured counts; Spend shows the worst budget's used percentage, or
  counts when a credit objective is worst. Missing objectives show “— / no objective”;
  objectives without measurements show “— / no data”. Each tile opens its member
  objectives with the existing 24-hour histories; the Objectives tile still opens all
  rows, including any a tile does not name. These histories are budget remaining, not
  PR throughput: a per-day throughput histogram remains a Fractal data-side gap.
- **Objective history** (`GET v1/client/slo/history`): for every listed row,
  what was left over the last 24 hours in 24 hourly steps (Fractal's default
  period; a row's `window` is display text and is not parsed). Each step holds
  its lowest reading; an unmeasured step stays empty, never spent. A row's
  history is read after its rows, again only when the row changes or an hour has
  passed, and is its own section: a failed history read keeps the last one and
  marks only that section stale.
  Due history requests run serially, avoiding a per-refresh burst of transport workers.
- **Availability** (`GET v1/client/availability`, `GET v1/client/notes`): status
  (available or away, by hand or from device presence; "no device reporting"
  when Fractal reads it as `unreported`), interruptions (all, urgent only, none)
  and the deadline clock (run, hold routine asks, hold all asks), each changed
  with an until preset (1 hour, 4 hours, tomorrow 9:00, until changed);
  returning an axis to its default (follow presence, all, run) takes no until.
  A word a newer Fractal adds is shown verbatim. The directive note agents read
  is written, given an expiry or cleared here. A change shows only once Fractal
  returns the folded result (`PUT v1/client/availability`, `PUT v1/client/notes`),
  or Fractal's refusal.
- **Availability history** (`GET v1/client/availability/history`): the last four
  weeks in hourly buckets from local midnight, read with the refresh at most
  every ten minutes. The availability page draws it as one of three options
  (design question H, its segmented picker remembered per Mac): H1 the last seven
  days as 24-hour strips (available fills the strip, away its lower part, no
  device reporting is an outline, nothing recorded the bare track), H2 each hour
  of the day's available share over the four weeks with today as a line, H3 the
  last 24 hours as minutes available above and away below the baseline with a
  lane of limited interruptions, and the same 24 hours as a strip on the
  availability tile. H1 places clock labels separately for each local day,
  including 23- and 25-hour daylight-saving days. H3 includes the whole first
  hour overlapping its 24-hour window in the chart, header total and tile:
  hourly aggregates cannot locate changes within that partial hour.
  Hover names an hour; VoiceOver reads each day or hour in
  words. A member without the read (an empty 404, which the client reports as
  `unsupported-capability`) shows "History unavailable on this Fractal version"
  instead of a failure, and is asked again hourly. A design-review build
  (`FRACTAL_MAC_HISTORY_PREVIEW=1`) draws a sample week there instead, badged
  "Preview data" above the chart, in the header and on the tile, until the member
  serves the person's own history.
- **Presence** (`POST v1/client/presence`): this Mac reports its raw state
  (`active`; `inactive` while another login session is in front; `locked`;
  `sleeping`) with its last keyboard, mouse or trackpad input, and Fractal
  derives presence the same way for every device; the tray applies no idle
  threshold and sends no heartbeat. Lock, unlock, session and sleep/wake changes
  are system notifications and are reported at once. While active, one
  non-repeating timer fires at Fractal's `report_again_at` (when it would read
  the Mac as idle without newer input) and reports the current last input from
  the HID idle counter (no permission needed). Once Fractal reads the Mac as
  idle, a global mouse monitor (no Accessibility grant) reports the return at
  once; keyboard-only returns show within a minute. A failed report is retried
  after a minute, never while asleep. Reports and failures are logged once per
  change.
- **Speech** (with `FRACTAL_SPEECH_SOCKET`): the tray decides what it asks the
  local speech service (dotfiles' `flakes/speech-service`, wire v2) to say; the service
  decides how, and holds the person's controls (mute, calls, Stop) for every
  caller, this tray and the person's `speak` alike.
  - **Delivery.** An item is delivered only if Fractal marks the ask `urgent`
    (its one urgency rule; the tray never derives urgency from a deadline), it
    is an update the person asked to be told (it carries `update.speech`), or
    the person pressed Read aloud, **and** the person is available and this Mac
    is present as Fractal derives it from the Mac's reports, **and**
    interruptions are `all`, or `urgent` for an urgent ask or asked-for update;
    never with `none` or a word the tray does not know. Delivery is decided
    after each refresh, so an item is delivered up to 30 seconds after Fractal
    shows it; only freshly read rows and availability count. Each revision is
    delivered once, kept across restarts; Read aloud may repeat it.
  - **Banner and speech.** A delivery is a native notification, and the
    update's own `speech` words, else the title (nothing else), are spoken too
    unless this requester is muted or spoke within 10 minutes, or the service
    holds it. The spoken banner shows once the service queued the item (or
    could not be reached); one the service answers `held` (`muted` or
    `in_call`) gets the held banner instead. Read aloud posts no banner and
    shows in the panel why it stayed quiet.
  - **Queue and agent gate.** The tray sends one item at a time and owns its
    queue, so it knows which of its items is being spoken. **Skip** drops a
    queued item; **Mute this agent** (1 hour or today) drops what it holds and
    stops it if it is speaking (`skip{id}`). Agent mutes are speech only, kept
    in the app's defaults across restarts, and end on their own.
  - **Person controls (speech service).** The tray follows the service's
    `watch` (its controls and queue, at every change; reconnecting with
    backoff from 1 to 30 seconds while the service is absent or restarting)
    and sends the person's controls to it: **Stop** in the panel or the
    notification sends `stop`, which ends the service's current request (the
    service's own `⌃⌥⌘.` hotkey does the same) and the queue goes on; **Mute
    speech** (1 hour, 4 hours, until tomorrow 9:00, or until unmuted) sends
    `mute`/`unmute`; **Allow speech during this call**, shown while the
    service reports a call, sends `allow_call` and resets when the call ends.
    The panel shows the service's mute, call and Stop hotkey as it reports
    them; a service that reports none (absent, or older than wire v2) leaves
    them out, and speech goes on without them.
  - **Notification.** While an item is spoken (or held to a banner) the tray
    shows a notification for it (`UNUserNotificationCenter`, one per item,
    replaced as it changes) with Stop, its answers as buttons, a text reply for
    feedback, custom answers and request changes, Mute this agent and Open in
    Fractal. Answers go through the same `work.done` as the panel: the
    notification says answered only once Fractal committed it and shows
    Fractal's reason on refusal. Notifications of items closed elsewhere are
    removed; an answered one stays as its receipt.
  - **Log.** Each verdict and outcome is logged once per item revision
    (`speech attention/…#rev from agent: urgent, queued` or `asked to be told,
queued`, `speaking`, then `spoken`, `skipped`, `held (…)` or the service's
    failure), plus agent mutes and the service's mute, call starts and ends;
    spoken text never enters the log. The tray waits 30 seconds for `queued`,
    then 60 seconds plus 30 seconds of margin for the result.
- Asks, availability and objectives refresh independently: one failing keeps its
  last data with its own stale note and never blanks the others.
- Times are relative with the absolute time on hover. The footer shows the build
  identity, refresh and Settings…; launch and each change of refresh outcome are
  logged to stderr.
- Settings holds Show in Menu Bar (`MenuBarExtra(isInserted:)`) and Quit. With
  the item hidden the tray keeps running; opening Fractal again brings it
  back.

## Packaging and configuration

From the repository root, `nix build .#fractal-mac` produces the unsigned
`Applications/Fractal.app` for `aarch64-darwin`, using Apple's host Swift SDK
and two Swift jobs. Its bundle identifier is `tech.compounding.fractal.mac`;
the executable is `Fractal.app/Contents/MacOS/Fractal`.

The SwiftPM dependency is the repository-local
`clients/swift/FractalClient` package. App and generated client therefore
change atomically; there is no external client pin or `FRACTAL_CLIENT_PATH`.
Run native checks on a Mac with:

```sh
cd apps/macos/Fractal
/usr/bin/swift build
/usr/bin/swift test
```

The package has five modules: `FractalMacCore` (model and projections),
`FractalMacUI` (views and template resources), `FractalMac` (live adapters and
app lifecycle), `FractalMacGallery` (fixture renderer), and
`FractalMacCoreTests`. The app product is named `Fractal`.

The canonical build identity is decoded by the shared Swift reader in
`apps/macos/shared/build-identity`. Nix embeds the stamp from
`nix/build-stamp.nix`; `rev`, `fullRev` and `commitTs` identify the same
repository revision as `FractalClientRevision` in `Info.plist`.
`baseVersion` in `Sources/FractalMacUI/Build.swift` owns the version literal,
also used for `CFBundleShortVersionString`. The member sees
`fractal-mac <machineVersion>` as the client name.

The footer refreshes each minute. It shows the base version, seven revision
characters, `*` if dirty, and compact commit age:
`0.2.0 · 883b657 · 2h`. Source builds add `local`; hover reveals the full
machine version, absolute commit time and client revision.

For a source run, set `CLI_BUILD_STAMP` to the canonical LocalStamp:
`{"type":"local","rev":"<git short HEAD>","ts":<start>,"commitTs":<HEAD commit time>,"dirty":false}`.
Use the exact built revision and its real commit timestamp, never the install
time. An unstamped source build shows `n/a` rather than fabricated provenance.

Standalone launches require `FRACTAL_SOCKET` (an absolute member socket path),
`FRACTAL_PERSON` (`person/<id>`) and `FRACTAL_URL` (an HTTP(S) origin).
No defaults or legacy environment aliases are accepted. The app uses the
owner-only local person socket without a bearer, pairing or device profile.
`FRACTAL_SPEECH_SOCKET` optionally enables the local speech client.
`FRACTAL_MAC_HISTORY_PREVIEW=1` enables labelled sample history only where the
member does not support history. `FRACTAL_MAC_SNAPSHOT=<png>` renders one
refreshed panel and exits without starting the presence reporter.

Home Manager installation, launchd, signing, Permissions Host and speech
service configuration belong to dotfiles, not this package. Its
`services.fractalMac` module consumes a pinned Fractal package, installs
`~/Applications/Fractal.app` and owns `tech.compounding.fractal.mac`.
The speech host must authorize the installed executable path in the same
activation as an app identity change. Rebuilding this unsigned package does
not itself install, sign, authorize or launch it.

The app requests Notifications on its first speech-enabled launch, with no
Screen Recording, Accessibility, Microphone, Camera, AppleEvents or Developer
Tools permission. Notification authorization is bundle-id keyed; the rename
from AttentionTray requires one new consent. The installer signs the bundle
so notifications can bind its `Info.plist`.
`FRACTAL_MAC_NOTIFICATION_CHECK=1` exercises a fixture notification locally,
logs the result and exits after a minute without sending data to the member
or speech service. Focus may suppress banners while Notification Center still
receives the notification.


## Design gallery

The panel's views live in `FractalMacUI` and read only `FractalMacCore`
and the build identity.
`FractalMacGallery` drives the real `AttentionModel` through fixture scenarios (busy,
answering, snooze and dismiss, quiet, read-only, degraded, unconfigured;
speech: speaking, in a call, muted) for the design Storybook in `design/`, from
the Nix-built gallery (`nix run .#fractal-mac-gallery -- <dir> …`).
Every manifest's `trayRev` records the gallery build's `machineVersion`, and
the footers show its build identity at render time. It needs a Mac with a login
session and has these outputs:

The `glance-quiet` (three items) and `glance-busy` (17 items) scenarios render at
a deterministic 520 pt scroll budget, not an uncapped page. Live panels instead
use their own screen's available height. Their synthetic Fractal fixtures cover
all tones, unmeasured producer objectives, daemon-health misses and a 37%-used
weekly budget. Render just these on a Mac with:
`swift run FractalMacGallery <out-dir> glance-quiet glance-busy`.

- `swift run FractalMacGallery design/renders`: every scenario, light and dark,
  as `@2x` PNGs, and the status item in every state, plus `manifest.json`,
  via the same `NSHostingView` + `cacheDisplay` path as snapshot mode.
  Window-server materials (the menu-bar window's Liquid Glass) are not in
  these renders; the Storybook labels them "no materials". The notification
  scenarios render its content model (`SpeechNoticeView`: text fields and
  actions), not macOS notification chrome, which only Notification Center
  draws. Committed.
- `swift run FractalMacGallery design/renders --capture --consent`: real captures. The gallery
  runs the actual `MenuBarExtra`, opens it under its status item and captures
  the region with ScreenCaptureKit over a fixed backdrop, other apps' menu-bar
  items masked: every scenario, and the status item in every state, into
  `renders/capture/` plus `capture.json`, committed when a run produces them
  (until then the Storybook shows the fallbacks). Captures show windows on the
  real screen, so they run only with the person's consent
  (`--consent`), wait until the Mac has had
  no input for ten minutes (`--away <seconds>`), abort and write nothing on any
  input, and need Screen Recording for the terminal they run from (System
  Settings › Privacy & Security › Screen & System Audio Recording).
  `--only <text>` redoes the captures whose file name contains it.
- `bash scripts/capture-vm.sh capture`: the same captures with
  nobody's screen involved. `scripts/capture-vm.sh` builds the gallery here and
  runs `--capture` inside a headless macOS VM (tart, Virtualization.framework,
  paravirtualized GPU: Liquid Glass and materials render as on a Mac) on
  `FRACTAL_MAC_CAPTURE_HOST` (an SSH host or `local`),
  then writes `renders/capture/` and `capture.json`; extra arguments go to
  `--capture` (`--only <text>`). The guest never draws a host window:
  tart runs with `--no-graphics` and `CI=1` (without them it opens Screen
  Sharing on the host's display). `capture-vm.sh provision` prepares a host
  once (a few minutes, plus a first pull of ~30 GB): it clones the macOS 27.0
  guest `ghcr.io/cirruslabs/macos-golden-gate-vanilla:27.0`, which needs a host
  on macOS 26.6.2 or later (`FRACTAL_MAC_CAPTURE_IMAGE` names another tart
  image), refuses to cross the host's reclaim-disk floor (a guest takes about
  35 GB), installs an SSH key and grants Screen Recording to
  `sshd-keygen-wrapper` through the guest's System Settings over the VM's
  loopback VNC, since neither SIP nor macOS 27 lets anything grant it
  programmatically. The guest's macOS is the captures' macOS: `capture.json`
  records it.


## Panel surface

The window-style `MenuBarExtra` supplies the native glass. One
`windowBackgroundColor` underlay at the `TrayPanel` root covers both columns,
adapts to light and dark appearance, and leaves the system glass edge intact.
`PanelMetrics.surfaceTintOpacity` controls its weight (0.45 retains 55% of
the backdrop contrast). Reduce Transparency omits this tint and leaves the
system's opaque accessibility surface alone. Groups and tiles do not add
another glass layer.

## Scroll and refresh performance Mac gate

`PanelScroll` and `PanelForm` preserve their compact ideal height, bounded by the
screen budget after measured chrome. The live `PanelWindowFrame` sizes the hosting window
from the measured page on both growth and shrinkage, anchored two points below the current
status-item window. macOS 27's `MenuBarExtra` can otherwise retain a larger hosting frame
and center the smaller painted page inside it: checking only the window's top misses this
detachment. No anchor is learned from previous panel frames. Short content stays compact,
and content beyond the budget scrolls internally. Unbounded snapshots still size naturally.
Markdown blocks/inline attributed strings and attachment images use bounded content-keyed
caches. Ages share one minute-aligned clock. Identical page polls and pruning do not rewrite
observable sections; separately kept successful-read timestamps preserve accurate subsequent
stale errors. Closing the panel removes page observation subscriptions, preserving only its
lightweight sized placeholder, visibility reporter and window-frame owner.

The menu-bar template has an equality boundary keyed by ask count, urgency, connection
health and display scale. Poll freshness alone does not rerun its `ImageRenderer`.
Rendering that small image on every refresh caused transient RenderBox/Metal allocations:
the observed ~288 MB footprint was dominated by ~258 MB of owned unmapped graphics,
not JSON decoding or resident malloc growth. Changes to the displayed state still redraw
the same template image, including its accessibility label.

Run from `apps/macos/Fractal` on an authorized Mac; the Swift client is
the package in the same checkout:

```sh
bash scripts/validate-scroll-mac.sh check
# Attach only to an already-running, muted app with authorization:
bash scripts/validate-scroll-mac.sh trace "$FRACTAL_PID"
bash scripts/validate-scroll-mac.sh memory "$FRACTAL_PID"
```

The `check` mode builds both products, runs Swift tests and invokes the offscreen
`FractalMacGallery <out-dir> --scroll-probe` and `--navigation-probe`. The scroll probe fails
unless the bottom is reachable at 300/400 pt with pinned answers, and 20 identical full
refreshes preserve the scroll view/offset without any context, excerpt or attachment writes.
Neither probe orders a window front or measures actual screen presentation or 120 Hz behavior.

For navigation, build first (`/usr/bin/swift build -c release`), then run
`.build/release/FractalMacGallery <out-dir> --navigation-probe`.
It also never orders a window front, and records `navigation-probe.json` (phase,
window height/top, content, viewport, offset) at 300/400/1578 pt. It checks layout through
first-open loading, separately released SLO/availability reads, row arrival and removal,
every signal, speech, ask/update route and refresh. Back restores native scroll state; a
fresh push starts at the top. Page heights match unbounded content capped to the screen
budget, and a quiet panel stays compact. This synthetic window is not a native attachment
test. Native outward wheel events and refreshes preserve top/bottom offsets, including
fitting content. A 19-row inbox fits without scrolling on the tall screen; short screens
apply `InboxCap`.

### Native panel attachment regression

Build the DEBUG app and evolving Unix-socket member fixture on a developer Mac:

```sh
bash scripts/probe-panel-mac.sh build "$HOME/.local/state/fractal-panel-probe"
```

Transfer that directory into an isolated macOS GUI VM (the capture VM route above can
provide one). In the guest, with the directory at the same short absolute path, run:

```sh
stage="$HOME/.local/state/fractal-panel-probe"
bash "$stage/run.sh" run "$stage"
```

This opens the actual app's status-item scene, keeps it open across 19→14→40→3 rows and
an availability read failure, navigates compact pages, and closes/reopens it twice.
It needs an authorized GUI session, not Accessibility consent. On macOS 27 the status
button has no target/action, so the DEBUG-only hook invokes AppKit's expanded-interface
session delegate; release builds contain neither this SPI nor the probe.
`frames.jsonl` records window moves/resizes/setFrame stacks, status/screen frames and the
painted page rectangle. Its oracle checks the painted top, not just the hosting window's
top; persistent centering within a stale large window fails. Missing samples, transitions
or fixture data also fail. `source-revision` identifies the build source.
Do not run the UI probe on an unapproved desktop. It is not wired into the headless
macOS CI lane: it needs a logged-in WindowServer session. The offscreen layout checks
remain the headless gate.

`trace` records Animation Hitches and Time Profiler and exports their data tables.
The script lists the interaction procedure and strict pass thresholds: every drill-in
and back first frame <100 ms; at 120 Hz, 8.333 ms/frame with zero drops/hitches.
`NavigationFirstFrame` signposts measure first AppKit draw submission; the reviewer must
also correlate actual presentation in Instruments. Missing data is not a passing result.
`memory` captures `footprint -p "$FRACTAL_PID"` and `vmmap -summary "$FRACTAL_PID"` at roughly
one-second cadence for three minutes. Inspect category totals, not RSS alone: owned
unmapped graphics can dominate a transient footprint spike. For a periodic-refresh
regression check, also sample at ten-second cadence for at least six minutes and record
the process peak so short spikes between samples are not missed.

## Brand assets

Regenerate committed icons and template PDFs on macOS with
`swift scripts/generate-icons.swift`. The generator uses AppKit and `iconutil`;
the SVG sources are the authority, not the generated binary assets.

`Resources/AppIcon/fractal-app-macos.svg` is Ada's selected final Fractal
mark (I2 Ink Light · D1 · Closest, 2026-11-16) on Apple's macOS icon grid: an
824 px squircle at offset 100 on the 1024 px canvas, with a drop shadow (10 px
down, σ 10, 30 % black). It renders the 64 px and larger slots of
`Resources/AppIcon.icns`. The small-size masters `fractal-app-macos-32.svg`
(16 pt @2x and 32 pt @1x) and `fractal-app-macos-16.svg` (16 pt @1x) change only
the mark stroke: 0.13372 and 0.18299 mark units, against the large master's
0.07038 (1.9× and 2.6×). NSImage's SVG decoder ignores filters, so the generator
draws the master's declared `feDropShadow` with Core Graphics.

`Resources/MenuBar/*.svg` supplies the template PDFs in
`Sources/FractalMacUI/Resources/MenuBar`. The owner's picks are I1 (idle:
1.25 pt outline with the count), Z1 (zero: the idle glyph with no count, so no
separate asset), U3 (urgent: filled badge dot at the top right), O2 (offline:
hollow ring at the top right, last count kept), and N2 (unconfigured: `!` badge,
a filled disc with a transparent `!` cut-out, no count). All states share the
18.67×16 pt artwork, which reserves the badge area so the count never shifts.
It is centered in a 19×16 pt PDF page because NSPDFImageRep rounds fractional
page sizes up. Masks become even-odd clips, so every cut-out is transparent.
The label code draws the unread dot: 4 pt across, 2 pt after the count (3 pt
after the glyph when the count is hidden), vertically centered in the 22 pt
item, and never with unconfigured. Menu-bar rendering keeps the template-image
and equatable boundary; assets decode only once and freshness-only polls do not
trigger another image render.

