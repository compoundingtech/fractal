# Attention tray

A native macOS 14+ SwiftUI menu-bar client for one person's Fractal: the
attention inbox, the person's availability and directive note, and the
objectives every Fractal client reads. It reads only Fractal, through
`FractalClient`, at launch and every 30 seconds; Fractal streams the attention
collection, but `FractalClient` has no typed attention or availability stream.

- **The panel** is a Control Center module: tiles for Availability, Speech and
  Objectives (the circle toggles, the rest opens the module's page), the inbox
  as one-line rows (symbol, title, asker, age; a draft, a write in flight or a
  refusal is marked on the row), and the footer. A row or tile opens its page
  within the panel, with a back chevron and Esc; with Reduce Motion pages
  cross-fade instead of sliding, and VoiceOver focus moves to the page title
  and back to the row or tile that opened it. Closing the panel returns it to
  the top level.
- **Asks** (all attention pages, classified as Fractal web's
  `data/attention.ts` does) are answered on their page: a decision's or
  choice's named answers (recommended first; request-changes takes words), a
  choice's custom words or feedback text, Yes/No for an untyped question, or
  Nothing to Do when there are no named answers. Custom forms and harness
  prompts link to Fractal. Every control is always in place, for the keyboard
  and VoiceOver as much as the pointer. Each page has at most one prominent
  button: an ask's recommended answer (never a decline, never one that needs
  words) while nothing is being typed, or an update's Mark Read; no answer
  takes Return, since answers are irreversible. A committed answer or control
  returns the panel to the top level with a receipt row. Typed answers and the
  note draft live on the model, so closing the panel keeps them.
- **Pages** show what the row cannot: the card read again with its whole
  request (`attentionGet`: question, why the person, summary, reasons,
  recommendation, subjects, attachments), its work step (`workGet` of the
  step run: title, state, last progress), each GitHub pull request's and
  issue's state through Fractal's GitHub plugin (with "New commits" when a
  pull request's head moved since the ask), image attachments as blobs read by
  the step run that carries them, and the asker's newest timeline page reduced
  to its last 10 prose turns, each capped at six lines. These are projection
  reads under the session's `read.projections` scope, like the inbox itself. A
  page is read when it opens, again on every refresh while the panel is on
  screen, and when the panel comes back on screen; a closed panel polls no
  page. Each part keeps its last value when a read fails and says why; a
  member that does not grant a read, or predates its route (404), leaves only
  that part unread. Raw image data leaves the model with its card; decoded images
  use a shared, evictable 32 MiB cache (charged by decoded pixels).
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
  (`ATTENTION_TRAY_HISTORY_PREVIEW=1`) draws a sample week there instead, badged
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
- **Speech** (with `ATTENTION_SPEECH_SOCKET`): the tray decides what it asks the
  local speech service (`flakes/speech-service`, wire v2) to say; the service
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
  the item hidden the tray keeps running; opening AttentionTray again brings it
  back.

## Packaging and configuration

`nix build ./flakes/attention-tray#attention-tray` produces an unsigned
`Applications/AttentionTray.app` for `aarch64-darwin`, using Apple's host Swift
SDK. SwiftPM consumes the exact locked Fractal Swift client via
`FRACTAL_CLIENT_PATH`; builds use two Swift jobs. The standalone flake pins the
Fractal revision carrying the availability API (`availabilityList`,
`availabilitySet`, `presenceReport`, `notesSet`); the root tray input follows the
root Fractal source, so the deployed tray uses the client of the daemon it talks
to, and builds only once that source carries the same API.

The tray carries the canonical build identity (effect-utils
`context/builds/01-identity`) through the shared Swift reader
`flakes/swift-shared/build-identity`. The Nix build compiles exactly the
`flakes/attention-tray` closure in `nix/buck2-products/source-paths.json` (its
Swift package and the shared reader) and embeds that closure's NixStamp: `rev`
and `commitTs` name the commit that produced it
(`nix/buck2-products/product-revs/flakes/attention-tray.json`), so unrelated
repository commits, docs and design renders neither rebuild nor restart the
tray. `baseVersion` in `Sources/TrayViews/Build.swift` is the one version
literal (also `CFBundleShortVersionString`). Fractal sees the tray as
`attention-tray <machineVersion>`.

The footer follows the identity spec's UI modality, refreshed each minute:

- A Nix build abbreviates to the base version and the shared short form
  (revision, `*` iff dirty, compact commit age): `0.2.0 · 2c3b5d7 · 2h`.
- A source run or dev build keeps the spec's local prose, so it never passes
  for a release:
  `0.2.0 — running from local source (d7a78aea1b, committed 2 hours ago)`.
- Its hover lists `displayVersion`, `machineVersion` (`0.2.0+2c3b5d77c917`,
  `0.2.0+local.d7a78aea1b`), the absolute commit time (and a dev build's start
  time) and the FractalClient revision (`FractalClientRevision` in
  `Info.plist`).

A build that does not run `flake.nix` (the dev channel, `swift run`) leaves the
`__CLI_BUILD_STAMP__` placeholder in `Build.swift` and must pass a LocalStamp in
the tray process's `CLI_BUILD_STAMP` environment variable (effect-utils
`cliBuildStamp`'s shape, timestamps in Unix seconds):
`{"type":"local","rev":"<git rev-parse --short HEAD>","ts":<start>,"commitTs":<git log -1 --format=%ct HEAD>,"dirty":<false|true>}`.
Without it the footer shows only `0.2.0`. `Contents/Resources/nix-build-stamp.json`
is no longer read.

Run tests with `nix develop ./flakes/attention-tray -c /usr/bin/swift test`.
The compiler-free shell avoids nixpkgs' xcrun stub; explicit `/usr/bin/swift`
uses Apple's host toolchain, consistent with the package build.

Standalone launches require three explicit environment variables:
`ATTENTION_FRACTAL_SOCKET` (absolute path of this Mac's Fractal member socket),
`ATTENTION_FRACTAL_PERSON` (`person/<id>`) and `ATTENTION_FRACTAL_URL`. There
are no defaults and no Boss fallbacks. The tray talks to the member over its
owner-only Unix socket as that person (local person trust, as `fractal note set`
does): no endpoint, bearer, pairing or gateway, and so no App Transport Security
exception. The application loads no keychain, 1Password item or device profile.
`ATTENTION_SPEECH_SOCKET` (optional, absolute) turns speech on.
`ATTENTION_TRAY_HISTORY_PREVIEW=1` (review builds only) draws the labelled sample
history where the member has no history read.
`ATTENTION_TRAY_SNAPSHOT=<png>` refreshes once, renders the open panel to that
file and exits.
Snapshot mode starts no presence reporter, so its speech gate stays quiet.

On mbp2025, `services.attentionTray.enable = true` with
`services.attentionTray.person = "person/schickling"` imports the Home Manager
module. Its activation bundle installs `~/Applications/AttentionTray.app`, signs
it ad-hoc, registers it with Launch Services and converges the GUI launch agent
`com.schickling.attention.tray`. `KeepAlive.PathState` on the member socket
`<services.smalltalk.stateDir>/run/fractal.sock` runs it while the member runs;
`RunAtLoad` is absent, so the agent is inert while the member is down, including
across unrelated Home Manager activations. Missing web prerequisites fail with a
clear wrapper log under `ThrottleInterval = 60`, not a tight loop. There is no
separate Login Items or SMAppService enrollment. Changed payloads restart once
while the member runs; unchanged activation does not restart the process.

`services.attentionTray.speechSocket` (default null) makes the wrapper export
`ATTENTION_SPEECH_SOCKET`. Point it at the speech service's socket only where that
service admits `~/Applications/AttentionTray.app/Contents/MacOS/AttentionTray` as
its one caller; any other executable, including a development build, is closed
unanswered and logged as `speech service unreachable`. On mbp2025 it follows
`services.speechService.socketPath` whenever `services.speechService.enable` is
set, whose `authorizedCaller` is that installed executable.

Activation verification checks only installed build/configuration facts
(signature and registered launchd job). The member socket and web availability
are checked by both the launcher and the separate `attention-tray-readiness`
command, never by the Home Manager activation. The module asserts that the
rendered XML has `PathState` and no `RunAtLoad`.
This matches [launchd.plist(5)](https://www.manpagez.com/man/5/launchd.plist/osx-10.11.6.php):
`PathState = true` keeps the job alive while the path exists; unlike
`SuccessfulExit`, it does not imply `RunAtLoad`. The shared activation helper
installs the rendered plist directly; neither nix-darwin nor HM adds a launch
condition. Filesystem notifications can be lossy: if the member starting does
not start the tray, inspect the resolved domain and kickstart only this job.

The wrapper requires the member socket to be a socket owned by the user and not
a symlink; the member creates it owner-only, which is the whole trust boundary.
The tray acts as the configured person and holds no credential, so there is
nothing to pair, rotate or revoke. It reads `v1/client/capabilities` and shows a
read-only note instead of answer controls unless `work.done` is granted.

The Fractal URL has one source of truth: the configured local Fractal daemon's
`<services.smalltalk.stateDir>/web-launch-url`. This is the same private
rendezvous consumed by `fractal web`; the wrapper retains only the HTTP origin,
not its one-use login token, and requires the file to be owned, not a symlink
and owner-only (`0400` or `0600`). Missing runtime prerequisites do not block
activation: an explicit launcher/readiness diagnostic identifies them.

The tray asks for one permission, Notifications (`UNUserNotificationCenter`,
alerts only), on its first launch with a speech service; it needs no Screen
Recording, Accessibility, Microphone, Camera, AppleEvents or Developer Tools
permission and no entitlements (call detection and the Stop hotkey are the
speech service's). Notifications need the bundle signature to bind
`Info.plist`: Home Manager's ad-hoc bundle signing of
`~/Applications/AttentionTray.app` does, the store bundle's linker-only
signature does not (`UNErrorDomain` 1, no prompt). The permission is keyed by
bundle identifier and stays granted when a rebuild changes the cdhash. No
keychain unlock or op-proxy panel is needed for this tray installation.

`ATTENTION_TRAY_NOTIFICATION_CHECK=1` verifies notifications: the tray asks for
the permission, logs the result, shows one fixture notification through the
real path (nothing is sent to Fractal or the speech service, clicked actions
are logged) and exits after a minute. Focus suppresses banners; the
notification still lands in Notification Center.

## Design gallery

The panel's views live in the `TrayViews` target and read only `AttentionCore`
and the build identity.
`TrayGallery` drives the real `AttentionModel` through fixture scenarios (busy,
answering, snooze and dismiss, quiet, read-only, degraded, unconfigured;
speech: speaking, in a call, muted) for the design Storybook in `design/`, from
the Nix-built gallery (`nix run ./flakes/attention-tray#gallery -- <dir> …`).
Every manifest's `trayRev` records the gallery build's `machineVersion`, and
the footers show its build identity at render time. It needs a Mac with a login
session and has these outputs:

- `devenv tasks run attention-tray:gallery`: every scenario, light and dark,
  as `@2x` PNGs, and the status item in every state, plus `manifest.json`,
  via the same `NSHostingView` + `cacheDisplay` path as snapshot mode.
  Window-server materials (the menu-bar window's Liquid Glass) are not in
  these renders; the Storybook labels them "no materials". The notification
  scenarios render its content model (`SpeechNoticeView`: text fields and
  actions), not macOS notification chrome, which only Notification Center
  draws. Committed.
- `devenv tasks run attention-tray:gallery-capture`: real captures. The gallery
  runs the actual `MenuBarExtra`, opens it under its status item and captures
  the region with ScreenCaptureKit over a fixed backdrop, other apps' menu-bar
  items masked: every scenario, and the status item in every state, into
  `renders/capture/` plus `capture.json`, committed when a run produces them
  (until then the Storybook shows the fallbacks). Captures show windows on the
  real screen, so they run only with the person's consent
  (`ATTENTION_TRAY_CAPTURE_CONSENT=1`, `--consent`), wait until the Mac has had
  no input for ten minutes (`--away <seconds>`), abort and write nothing on any
  input, and need Screen Recording for the terminal they run from (System
  Settings › Privacy & Security › Screen & System Audio Recording).
  `--only <text>` redoes the captures whose file name contains it.
- `devenv tasks run attention-tray:gallery-capture-vm`: the same captures with
  nobody's screen involved. `scripts/capture-vm.sh` builds the gallery here and
  runs `--capture` inside a headless macOS VM (tart, Virtualization.framework,
  paravirtualized GPU: Liquid Glass and materials render as on a Mac) on
  `ATTENTION_TRAY_CAPTURE_HOST` (an SSH host, default `mbp2021`, or `local`),
  then writes `renders/capture/` and `capture.json`; extra arguments go to
  `--capture` (`--only <text>`). The guest never draws a host window:
  tart runs with `--no-graphics` and `CI=1` (without them it opens Screen
  Sharing on the host's display). `capture-vm.sh provision` prepares a host
  once (a few minutes, plus a first pull of ~30 GB): it clones the macOS 27.0
  guest `ghcr.io/cirruslabs/macos-golden-gate-vanilla:27.0`, which needs a host
  on macOS 26.6.2 or later (`ATTENTION_TRAY_CAPTURE_IMAGE` names another tart
  image), refuses to cross the host's reclaim-disk floor (a guest takes about
  35 GB), installs an SSH key and grants Screen Recording to
  `sshd-keygen-wrapper` through the guest's System Settings over the VM's
  loopback VNC, since neither SIP nor macOS 27 lets anything grant it
  programmatically. The guest's macOS is the captures' macOS: `capture.json`
  records it.

## mbp2025 rollout: root agent and Johannes only

Preparation is not permission to deploy. The root agent obtains Johannes's
approval and uses the intended reviewed revision. Person-valued commands run
only in Johannes's idle `as-schickling` PTYs, never in an agent's bound shell:
use `pty root --json` (`.nativeDefault.path`), `pty --root ROOT list --json`,
peek first and send only to an idle shell.

### 1. Johannes's browser session

The root agent sends `fractal web` through the idle mbp2025 `as-schickling` PTY.
It opens an authenticated local browser session and writes the protected
`web-launch-url` rendezvous. Do not print, paste or cache that file. If it
reports missing assets/person configuration, repair the existing daemon
installation; do not start a second daemon or invent an origin.

### 2. Root agent deploys after Johannes approves

From the intended dotfiles checkout on mbp2025:

```sh
nix run .#mbp2025-substrate-deploy -- -m "Attention tray over the local Fractal member socket"
```

This is the AGENTS.md entrypoint, not `home-manager switch` or
`darwin-rebuild`. Do not sit at an interactive sudo prompt. The member socket
and the web rendezvous are runtime prerequisites, not activation prerequisites;
the agent stays idle while the member is down.

### 3. Root agent verifies; Johannes checks the UI

On mbp2025 in the active GUI session:

```sh
nix run .#nix.activations.aarch64-darwin.mbp2025.attention-tray.verify
codesign --verify --deep --strict "$HOME/Applications/AttentionTray.app"
launchctl print "gui/$(id -u)/com.schickling.attention.tray"
attention-tray-readiness
```

The shared domain resolver selects `gui/<uid>` when Aqua exists, else
`user/<uid>`; use that same resolved domain for inspection outside a GUI
session. Readiness validates the member socket, the web rendezvous and a live
launchd PID; expect no repeated nonzero exits. Logs are
`~/.local/state/attention-tray/{stdout,stderr}.log`. The stderr log has one
line per change of refresh outcome, for example
`attention ok (3 asks, 1 updates); slo ok (28 rows, 7 need attention)`; a
refusal appears there verbatim.

Johannes opens the menu-bar icon and checks that configuration is valid,
freshness is current (not stale), current pending asks appear and objectives
match `fractal slo` on mbp2025: member-measured objectives describe this
member. Compare with Fractal's authenticated attention view, respecting the
tray's supported-answer filter and full pagination. Wait at least one 30-second
interval and observe refresh. Answering is proven on an ask raised for the
purpose (for example `fractal work ask` from an agent seat), never on a real
one: the card shows Sending, then committed, and leaves on the next read; the
asking agent sees the answer. Open in Fractal must open `/w/<agent>` for its
conversation/requester and the authenticated page must load.

### 4. Root agent retires the gateway transport's paired device

Only after step 3 passed: the tray's earlier dev5-gateway transport used a
paired device whose bearer is in
`~/.config/attention-tray/fractal/fractal-devices.json` on mbp2025. Revoke it
on dev5, the member that paired it, by its nonsecret device ID (recorded in
`CREDENTIALS.md`), through the idle dev5 `as-schickling` PTY:

```sh
fractal devices --as person/schickling revoke device/DEVICE_ID --reason "Attention tray moved to the local member socket"
```

Then, through the idle mbp2025 `as-schickling` PTY:

```sh
rm -r "$HOME/.config/attention-tray/fractal"
rmdir "$HOME/.config/attention-tray" 2>/dev/null || true
```

Never print the profile. Report only the revoke result and that the directory
is gone.

## Scroll and refresh performance Mac gate

`PanelScroll` and `PanelForm` preserve their compact ideal height but accept a shorter
window proposal (bounded by the scroll cap). Markdown blocks/inline attributed strings
and attachment images use bounded content-keyed caches. Ages share one minute-aligned
clock. Identical page polls and pruning do not rewrite observable sections; separately
kept successful-read timestamps preserve accurate subsequent stale errors. Closing the
panel removes its page observation subscriptions, preserving only a lightweight sized
placeholder and visibility reporter.

Run from the repository root on an authorized Mac, with the compatible FractalClient
package supplied by the development shell (record its exact SHA):

```sh
nix develop ./flakes/attention-tray -c bash flakes/attention-tray/scripts/validate-scroll-mac.sh check
# Attach to an already-running, muted tray; do not launch another copy:
bash flakes/attention-tray/scripts/validate-scroll-mac.sh trace "$TRAY_PID"
bash flakes/attention-tray/scripts/validate-scroll-mac.sh memory "$TRAY_PID"
```

The `check` mode builds both products, runs Swift tests and invokes the offscreen
`TrayGallery <out-dir> --scroll-probe`. The probe fails unless the bottom is reachable
at 300/400 pt with inline/pinned answers, and 20 identical full refreshes preserve the
scroll view/offset without any context, excerpt or attachment writes. It never orders
a window front and does **not** measure actual screen presentation or 120 Hz behavior.

`trace` records Animation Hitches and Time Profiler and exports their data tables.
The script lists the interaction procedure and strict pass thresholds: every drill-in
and back first frame <100 ms; at 120 Hz, 8.333 ms/frame with zero drops/hitches.
`NavigationFirstFrame` signposts measure first AppKit draw submission; the reviewer must
also correlate actual presentation in Instruments. Missing data is not a passing result.
`memory` captures `footprint -p "$TRAY_PID"` and `vmmap -summary "$TRAY_PID"` at roughly
one-second cadence for three minutes to investigate the previously observed transient
~289 MB spikes every 30–60 seconds. This remains an unproven perf suspect, not a claimed
memory fix.

Build, tests, probe, UI/TCC, frame-time and memory validation for this change:
**NOT RUN (needs Mac)**. Linux cannot compile or exercise AppKit/SwiftUI. The V4 base's
flake client may also fail on `AvailabilityRule.word`; use and record the compatible
client revision rather than treating a mismatched dependency build as a tray result.
