# Attention tray

A native macOS 14+ SwiftUI menu-bar client for one person's Fractal: the
attention inbox, the person's availability and directive note, and the
objectives every Fractal client reads. It reads only Fractal, through
`FractalClient`, at launch and every 30 seconds; Fractal streams the attention
collection, but `FractalClient` has no typed attention or availability stream.

- **Asks** (all attention pages, classified as Fractal web's
  `data/attention.ts` does) are answered in the panel: a decision's or choice's
  named answers (recommended first; request-changes takes words), a choice's
  custom words or feedback text, Yes/No for an untyped question, or Nothing to
  Do when there are no named answers. Custom forms and harness prompts link to
  Fractal. Every control is always in place, for the keyboard and VoiceOver as
  much as the pointer. The first answerable ask's recommended answer (never a
  decline) is the one prominent button while nothing is being typed; no answer
  takes Return, since answers are irreversible.
  Typed answers and the note draft live on the model, so closing the panel keeps
  them.
- **Snooze and Dismiss** are the person's own attention controls
  (`attention.snooze`, `attention.dismiss`), offered when the session is
  granted them, on every open ask (and Snooze on updates), never as an answer.
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
- **Speech** (with `ATTENTION_SPEECH_SOCKET`): the tray decides what the local
  speech service (`flakes/speech-service`) says; the service only decides how.
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
    unless speech is muted (all, or this requester), the Mac is in a call the
    person has not allowed speech for, or the requester spoke within 10
    minutes. Read aloud posts no banner and shows in the panel why it stayed
    quiet.
  - **Queue and controls.** The tray sends one item at a time and owns the
    queue, so it knows what is being spoken. **Stop** (panel, notification, or
    `⌃⌥⌘.` from any app via Carbon `RegisterEventHotKey`, no permission) ends
    the current item and the queue goes on; **Skip** drops a queued item;
    **Mute this agent** (1 hour or today) and **Mute speech** (1 hour, 4 hours,
    until tomorrow 9:00, or until unmuted) drop what they hold and stop it if it
    is speaking. Mutes are speech only, kept in the app's defaults across
    restarts, and end on their own.
  - **Calls.** A microphone in use by another process (Core Audio process
    objects, `kAudioProcessPropertyIsRunningInput`, the menu bar indicator's
    signal) or any camera running (`kCMIODevicePropertyDeviceIsRunningSomewhere`)
    is a call: speech stops and items are banners only. Both are listeners, not
    polls, and need no permission. **Allow speech during this call** in the
    panel lifts that for the current call only; the call ends, and the override
    resets, after 15 seconds without any microphone or camera in use.
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
    queued`, `speaking`, then `spoken`, `skipped` or the service's failure),
    plus mutes and call starts and ends; spoken text never enters the log. The
    tray waits 30 seconds for `queued`, then 60 seconds plus 30 seconds of
    margin for the result.
- Asks, availability and objectives refresh independently: one failing keeps its
  last data with its own stale note and never blanks the others.
- Times are relative with the absolute time on hover. The footer shows the build
  identity, Settings… and Quit; launch and each change of refresh outcome are
  logged to stderr.
- Settings holds Show in Menu Bar (`MenuBarExtra(isInserted:)`). With the item
  hidden the tray keeps running; opening AttentionTray again brings it back.

## Packaging and configuration

`nix build ./flakes/attention-tray#attention-tray` produces an unsigned
`Applications/AttentionTray.app` for `aarch64-darwin`, using Apple's host Swift
SDK. SwiftPM consumes the exact locked Fractal Swift client via
`FRACTAL_CLIENT_PATH`; builds use two Swift jobs. The standalone flake pins the
Fractal revision carrying the availability API (`availabilityList`,
`availabilitySet`, `presenceReport`, `notesSet`); the root tray input follows the
root Fractal source, so the deployed tray uses the client of the daemon it talks
to, and builds only once that source carries the same API.

The bundle carries `Contents/Resources/nix-build-stamp.json` (`type`, `version`,
`rev`, `dirty`, `fractalClientRev`). Its `rev` is `src.<hash>` of the tray's own
Swift package (`Package.swift`, `Sources/`, `Tests/`) plus the FractalClient
revision, so unrelated repository commits, docs and design renders neither rebuild nor
restart the tray; the footer shows `machineVersion` (`0.2.0+src.<hash>`) with the
FractalClient revision on hover.

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
`ATTENTION_TRAY_SNAPSHOT=<png>` refreshes once, renders the open panel to that
file and exits.
Snapshot mode starts no presence reporter, so its speech gate stays quiet.

On review-host, `services.attentionTray.enable = true` with
`services.attentionTray.person = "person/ada"` imports the Home Manager
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
unanswered and logged as `speech service unreachable`. On review-host it follows
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
permission and no entitlements (call detection reads only device and process
state; the Stop hotkey is a Carbon hot key). Notifications need the bundle
signature to bind `Info.plist`: Home Manager's ad-hoc bundle signing of
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

The panel's views live in the `TrayViews` target and read only `AttentionCore`.
`TrayGallery` drives the real `AttentionModel` through fixture scenarios (busy,
answering, snooze and dismiss, quiet, read-only, degraded, unconfigured;
speech: speaking, in a call, muted) for the design Storybook in `design/`, from
the Nix-built gallery (`nix run ./flakes/attention-tray#gallery -- <dir> …`).
Every manifest records the source rev. It needs a Mac with a login session and
has these outputs:

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
  `ATTENTION_TRAY_CAPTURE_HOST` (an SSH host, default `review-host`, or `local`),
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

## review-host rollout: root agent and Ada only

Preparation is not permission to deploy. The root agent obtains Ada's
approval and uses the intended reviewed revision. Person-valued commands run
only in Ada's idle `as-schickling` PTYs, never in an agent's bound shell:
use `pty root --json` (`.nativeDefault.path`), `pty --root ROOT list --json`,
peek first and send only to an idle shell.

### 1. Ada's browser session

The root agent sends `fractal web` through the idle review-host `as-schickling` PTY.
It opens an authenticated local browser session and writes the protected
`web-launch-url` rendezvous. Do not print, paste or cache that file. If it
reports missing assets/person configuration, repair the existing daemon
installation; do not start a second daemon or invent an origin.

### 2. Root agent deploys after Ada approves

From the intended dotfiles checkout on review-host:

```sh
nix run .#review-host-substrate-deploy -- -m "Attention tray over the local Fractal member socket"
```

This is the AGENTS.md entrypoint, not `home-manager switch` or
`darwin-rebuild`. Do not sit at an interactive sudo prompt. The member socket
and the web rendezvous are runtime prerequisites, not activation prerequisites;
the agent stays idle while the member is down.

### 3. Root agent verifies; Ada checks the UI

On review-host in the active GUI session:

```sh
nix run .#nix.activations.aarch64-darwin.review-host.attention-tray.verify
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

Ada opens the menu-bar icon and checks that configuration is valid,
freshness is current (not stale), current pending asks appear and objectives
match `fractal slo` on review-host: member-measured objectives describe this
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
`~/.config/attention-tray/fractal/fractal-devices.json` on review-host. Revoke it
on dev5, the member that paired it, by its nonsecret device ID (recorded in
`CREDENTIALS.md`), through the idle dev5 `as-schickling` PTY:

```sh
fractal devices --as person/ada revoke device/DEVICE_ID --reason "Attention tray moved to the local member socket"
```

Then, through the idle review-host `as-schickling` PTY:

```sh
rm -r "$HOME/.config/attention-tray/fractal"
rmdir "$HOME/.config/attention-tray" 2>/dev/null || true
```

Never print the profile. Report only the revoke result and that the directory
is gone.
