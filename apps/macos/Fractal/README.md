# Attention tray

A native macOS 14+ SwiftUI menu-bar client. It reads all pages of open supported
person asks at launch and every 30 seconds, retaining the last successful data
with an explicit stale indicator on failure. Ask links open Fractal's
`/w/<agent>` route. Notices are labelled synthetic previews; notice controls do
not persist or send unsupported actions. Ask answering happens in Fractal.

## Packaging and configuration

`nix build ./flakes/attention-tray#attention-tray` produces an unsigned
`Applications/AttentionTray.app` for `aarch64-darwin`, using Apple's host Swift
SDK. SwiftPM consumes the exact locked smalltalk Swift client via
`ST3_CLIENT_PATH`; builds use two Swift jobs.

Standalone launches require three explicit environment variables:
`ATTENTION_ST_ENDPOINT`, `ATTENTION_ST_CREDENTIAL`, `ATTENTION_FRACTAL_URL`.
There are no endpoint, localhost, or Boss fallbacks. The application itself
loads no keychain, 1Password item or device-profile file.

On mbp2025, `services.attentionTray.enable = true` imports the Home Manager
module. Its activation bundle installs `~/Applications/AttentionTray.app`, signs
it ad-hoc, registers it with Launch Services and converges the GUI launch agent
`com.schickling.attention.tray`. The job starts at login and is kept alive;
there is no separate Login Items or SMAppService enrollment. Activation changes
restart it once, and unchanged activation does not restart it.

The stable wrapper reads one dedicated paired-device profile at
`~/.config/attention-tray/fractal/stui-devices.json` (regular, owned, `0600`;
parent `0700`). It accepts exactly one `person/schickling` device and exports
its endpoint and bearer only to the app process. Never put this bearer into
Nix options, launchd environment configuration, shell arguments or logs.

The Fractal URL has one source of truth: the configured local Fractal daemon's
`<services.smalltalk.stateDir>/web-launch-url`. This is the same private
rendezvous consumed by `fractal web`; the wrapper retains only the HTTP origin,
not its one-use login token. Missing profile or rendezvous fails explicitly.
The web listener is not the paired API carrier: do not send the device bearer
to the browser listener or forward the privileged daemon socket.

The current tray requests no Notifications, Screen Recording, Accessibility,
AppleEvents or Developer Tools permission and needs no entitlements. A local
ad-hoc signature is sufficient. If UserNotifications or another TCC API is
added, migrate to the governed stable-app signing identity before rollout.
No keychain unlock or op-proxy panel is needed for this tray installation.

## mbp2025 rollout: root agent and Johannes only

Preparation is not permission to deploy or pair. The root agent obtains
Johannes's approval, keeps the PR draft until the normal merge process allows
it, and uses the intended reviewed revision for rollout. No agent invokes
person-valued `--as` or runs the pairing steps in its bound session.

### 1. Johannes provisions a dedicated read-only device

Use Johannes's human `as-schickling` PTY on the trusted Fractal member (dev5).
The paired-only tailnet carrier is configured by
`nixpkgs/home-manager/dev5.nix`; its address derives from `hosts.nix` and
`system-ports.nix`, not from a privileged socket. Before making a grant, the
root agent can evaluate the intended carrier address on dev3:

```sh
nix eval --impure --raw --expr 'let h = import ./hosts.nix; p = import ./nixpkgs/shared/system-ports.nix {}; in "http://${h.nixos.dev5.tailnet.ipv4}:${toString p.hosts.dev5.infrastructure.smalltalkClientGateway.http}"'
```

Call that exact result `MEMBER_URL`. Check an unauthenticated request to
`MEMBER_URL/v1/client/agents` returns **403** before pairing. Never replace the
carrier with the daemon's privileged socket. HTTP is acceptable only over the
encrypted tailnet; do not publish this listener to a shared LAN or the public
Internet.

In Johannes's dev5 `as-schickling` PTY:

```sh
fractal devices --as person/schickling pair --read-only "Attention tray mbp2025"
```

Keep the returned pairing ID, short-lived code and person-root fingerprint in
the human session. Verify the fingerprint over that trusted member session,
not a value learned solely from the remote gateway. Do not select
`--full-control`, `--assistant` or `--fabric-gateway`.

In Johannes's mbp2025 `as-schickling` PTY:

```sh
umask 077
mkdir -p "$HOME/.config/attention-tray/fractal"
chmod 0700 "$HOME/.config/attention-tray" "$HOME/.config/attention-tray/fractal"
XDG_CONFIG_HOME="$HOME/.config/attention-tray" \
  fractal devices complete MEMBER_URL PAIRING_ID --fingerprint sha256:FINGERPRINT
```

Replace the three nonsecret placeholders with the trusted carrier URL, pairing
ID and fingerprint. Enter the single-use code at the hidden prompt. This
isolated config home prevents reusing an unrelated client's credential. The
current Fractal CLI writes the `fractal/stui-devices.json` profile; the old
`st` name's pairing commands are equally person-scoped and are not an agent
workaround. Record the actual device ID and expiry in `CREDENTIALS.md` after
provisioning; do not claim a minted grant before that step.

No 1Password item or op-proxy approval panel is involved. The human pairing
challenge and code are the approval boundary. If completion reports a local
save failure after consuming the code, inspect and revoke its reported orphan
device on the original member before making another challenge.

### 2. Johannes authenticates the browser

On mbp2025, in Johannes's `as-schickling` PTY:

```sh
fractal web
```

This opens an authenticated local browser session using the daemon's protected
rendezvous. Do not print, paste or cache `web-launch-url`. If it reports missing
assets/person configuration, repair the existing daemon installation; do not
start a second daemon, invent an origin or silently select another host.

### 3. Root agent deploys only after Johannes approves

From the intended dotfiles checkout on mbp2025:

```sh
nix run .#mbp2025-substrate-deploy -- -m "Install paired Attention tray for pending asks"
```

This is the AGENTS.md entrypoint, not `home-manager switch` or
`darwin-rebuild`. Do not sit at an interactive sudo prompt. Pairing and local
web availability are prerequisites: otherwise the tray cannot show asks or
open authenticated links.

### 4. Root agent verifies; Johannes checks the UI

On mbp2025 in the active GUI session:

```sh
nix run .#nix.activations.aarch64-darwin.mbp2025.attention-tray.verify
codesign --verify --deep --strict "$HOME/Applications/AttentionTray.app"
launchctl print "gui/$(id -u)/com.schickling.attention.tray"
```

The shared domain resolver selects `gui/<uid>` when Aqua exists, else
`user/<uid>`; use that same resolved domain for inspection outside a GUI
session. Expect one job with a live PID and no repeated nonzero exits.
Logs are `~/.local/state/attention-tray/{stdout,stderr}.log`; do not dump
credential files or process environments when diagnosing failures.

Johannes opens the menu-bar icon and checks that configuration is valid,
freshness is current (not stale), and current pending asks appear. Compare with
Fractal's authenticated attention view, respecting the tray's supported-answer
filter and full pagination rather than comparing to an unfiltered first page.
Wait at least one 30-second interval and observe refresh. Click an actual ask:
the browser must open `/w/<agent>` for its conversation/requester, not a Boss
URL, and the authenticated Fractal page must load. The synthetic notice section
is not evidence of successful live ask retrieval.

Run a second unchanged sanctioned activation only if rollout verification
requires it; compare the launchd PID and confirm there was no extra restart or
permission prompt. Do not revoke/re-pair merely to retry a display failure.

For retirement/rotation Johannes uses the original member's human PTY:
`fractal devices --as person/schickling revoke device/DEVICE_ID --reason "Retired Attention tray grant"`.
Re-pair for expiry, update the inventory and restart only this tray job in the
resolved domain so it rereads the profile. Do not restart Fractal or Boss.
