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

On review-host, `services.attentionTray.enable = true` imports the Home Manager
module. Its activation bundle installs `~/Applications/AttentionTray.app`, signs
it ad-hoc, registers it with Launch Services and converges the GUI launch agent
`com.schickling.attention.tray`. The job starts at login and is kept alive;
there is no separate Login Items or SMAppService enrollment. Activation changes
restart it once, and unchanged activation does not restart it.

The stable wrapper reads one dedicated paired-device profile at
`~/.config/attention-tray/fractal/stui-devices.json` (regular, owned, `0600`;
parent `0700`). It accepts exactly one `person/ada` device and exports
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

## review-host rollout: root agent and Ada only

Preparation is not permission to deploy or pair. The root agent obtains
Ada's approval, keeps the PR draft until the normal merge process allows
it, and uses the intended reviewed revision for rollout. No agent invokes
person-valued `--as` or runs the pairing steps in its bound session.

### 1. Root agent provisions the dedicated read-only device

Ada authorizes credential provisioning, but does not type, copy or paste
the pairing code. The root agent drives his existing idle `as-schickling`
PTYs. Before any Fractal invocation load `/sk-axe`; follow
`nixpkgs/ai/rules/global/principal-input.md:26-37`.
On each host use `pty root --json` and read `.nativeDefault.path`, then:

```sh
pty --root ROOT list --json
pty --root ROOT peek --plain EXACT_NAME
pty --root ROOT send EXACT_NAME --seq 'COMMAND' --seq key:return
```

Select a running `.displayName == "as-schickling"` and target its exact `.name`.
Peek first and only send to an idle shell; never interrupt another command.
The blocks below are commands for the root agent to send, not instructions for
Ada to execute. Send a multiline block using `pty send`'s paste support.
No person-valued Fractal command runs in the agent's bound shell.

The paired-only dev5 tailnet carrier is declared in `home-manager/dev5.nix`.
Evaluate its intended address on review-host from the reviewed checkout:

```sh
nix eval --impure --raw --expr 'let h = import ./hosts.nix; p = import ./nixpkgs/shared/system-ports.nix {}; in "http://${h.nixos.dev5.tailnet.ipv4}:${toString p.hosts.dev5.infrastructure.smalltalkClientGateway.http}"'
```

Call the exact result `MEMBER_URL`. Check unauthenticated
`MEMBER_URL/v1/client/agents` returns **403**. Never use a privileged daemon
socket. HTTP is acceptable only over the encrypted tailnet.

**Preflight transport before minting.** From the review-host person PTY, use the
existing fleet SSH route to dev5, with `BatchMode=yes` and
`StrictHostKeyChecking=yes`; confirm a nonsecret `printf` reaches a redirected
local file. Do not add a key, relax host-key checking, select a different
transport or request human copying on failure. SSH is the fleet's existing
encrypted file transport (`/sk-infra`); this runbook does not authorize a new
route. If that route is unavailable, the missing capability is specifically
**a non-transcript private file transfer from dev5 to review-host**: stop before
minting, report the refused route to Main, and request the smallest approved
transport primitive. No secret-bearing transfer has been exercised in this PR.

Send this to the idle **dev5 person PTY**. The exclusive directory prevents
overwriting another rollout. Run these blocks with a POSIX shell (for example
send `bash --noprofile --norc` first when the idle shell is fish):

```sh
set +x
umask 077
pair_dir="$HOME/.local/state/attention-tray-pairing"
mkdir -m 0700 "$pair_dir" &&
  fractal devices --as person/ada pair --read-only --json \
    "Attention tray review-host" >"$pair_dir/challenge.json" 2>"$pair_dir/error.log" &&
  jq -e '.value | (.pairing_id | type == "string") and
    (.code | type == "string" and length > 0) and
    (.person_root_fingerprint | type == "string" and startswith("sha256:"))' \
    "$pair_dir/challenge.json" >/dev/null &&
  jq '.value | {pairing_id, person_root_fingerprint, status: "challenge-created"}' \
    "$pair_dir/challenge.json"
```

The JSON is a client envelope (`.value.code`, `.value.pairing_id`,
`.value.person_root_fingerprint`). Only the allowlisted pairing ID,
fingerprint and status reach the PTY/output. The fingerprint comes from the
trusted member session, not an untrusted remote gateway. Never select
`--full-control`, `--assistant`, `--fabric-gateway` or `--unpinned`.

Send this to the idle **review-host person PTY**, from the reviewed dotfiles
checkout; replace `MEMBER_URL` with the evaluated nonsecret carrier URL:

```sh
set +x
set -o pipefail
umask 077
pair_dir="$HOME/.local/state/attention-tray-pairing"
mkdir -m 0700 "$pair_dir" &&
  ssh -T -o BatchMode=yes -o StrictHostKeyChecking=yes dev5 \
    'cat "$HOME/.local/state/attention-tray-pairing/challenge.json"' \
    >"$pair_dir/challenge.json" 2>"$pair_dir/transfer-error.log"
```

Proceed only on transfer success. File bytes go from SSH stdout to the local
private file, never to `pty peek`, a tool result, shell argv or agent transcript.
Send the following block next, substituting the independently observed trusted
fingerprint for `TRUSTED_FINGERPRINT` (it is not a secret):

```sh
(
set +x
set -euo pipefail
umask 077
pair_dir="$HOME/.local/state/attention-tray-pairing"
trap 'rm -f "$pair_dir/challenge.json" "$pair_dir/transfer-error.log" "$pair_dir/completion.json" "$pair_dir/error.log"; rmdir "$pair_dir"' EXIT
profile="$(nix eval --raw .#darwinConfigurations.review-host.config.home-manager.users.schickling.services.attentionTray.profileFile)"
mkdir -p "$(dirname "$profile")"
chmod 0700 "$HOME/.config/attention-tray" "$(dirname "$profile")"
pairing_id="$(jq -er '.value.pairing_id' "$pair_dir/challenge.json")"
fingerprint="$(jq -er '.value.person_root_fingerprint' "$pair_dir/challenge.json")"
test "$fingerprint" = "TRUSTED_FINGERPRINT"
jq -er '.value.code' "$pair_dir/challenge.json" |
  fractal devices complete MEMBER_URL "$pairing_id" --fingerprint "$fingerprint" \
    --profile "$profile" --json >"$pair_dir/completion.json" 2>"$pair_dir/error.log"
# Contract check: the CLI saved exactly the profile consumed by the HM wrapper.
jq -e --arg profile "$profile" \
  '.kind == "device-paired" and .profile == $profile' \
  "$pair_dir/completion.json" >/dev/null
test -f "$profile"
printf '%s\n' '{"status":"paired-profile-path-verified"}'
)
```

The explicit supported `--profile` flag bypasses migration-dependent
`product_dirs::resolve(st3, fractal)` defaults. No migration receipt is needed.
The completion `.profile` check is mandatory before deployment and proves the
CLI writer and HM consumer agree. Do not print completion/error files or the
saved profile. Report only status; record actual nonsecret device inventory
through a separately allowlisted metadata read after pairing.

Pinned source evidence (`99dce55f6286c0a03f77bbb6f42daad940ee6994`):
`crates/fractal/src/main.rs:2042-2044` defines `--read-only`,
`:2071-2084` defines `--profile` and `--fingerprint`, `:8738-8745`
uses the explicit profile before considering the product-directory default,
and `:8747-8757` emits a safe completion JSON with `.profile`.
`crates/fractal-client/src/device.rs:921-929` accepts non-terminal stdin.
The challenge fields are defined in `crates/fractal-client/src/generated.rs:4476-4482`;
`main.rs:8898-8905` preserves the `.value` envelope for JSON output.

On **every success or failure**, send this cleanup to the **dev5 person PTY**:

```sh
rm -f "$HOME/.local/state/attention-tray-pairing/challenge.json" \
  "$HOME/.local/state/attention-tray-pairing/error.log"
rmdir "$HOME/.local/state/attention-tray-pairing"
```

Clean only directories whose successful exclusive `mkdir` was observed for
this rollout. If `mkdir` reports an existing directory, stop without touching
its contents; another operation may own it.

If failure occurred before the review-host completion block's cleanup trap, remove
its challenge/transfer-error files and directory in that person PTY too. Track
these private temporary paths in the rollout worktree's
`tmp/attention/TEMPORARY_FILES.md`, recording paths only, never contents.
If the code was consumed but local save failed, inspect/revoke the orphan on
the original trusted member before another challenge; do not blindly retry.
No op-proxy panel or human code-entry step is needed.

### 2. Ada authenticates the browser

The root agent sends this through the idle review-host `as-schickling` PTY:

```sh
fractal web
```

This opens an authenticated local browser session using the daemon's protected
rendezvous. Do not print, paste or cache `web-launch-url`. If it reports missing
assets/person configuration, repair the existing daemon installation; do not
start a second daemon, invent an origin or silently select another host.

### 3. Root agent deploys only after Ada approves

From the intended dotfiles checkout on review-host:

```sh
nix run .#review-host-substrate-deploy -- -m "Install paired Attention tray for pending asks"
```

This is the AGENTS.md entrypoint, not `home-manager switch` or
`darwin-rebuild`. Do not sit at an interactive sudo prompt. Pairing and local
web availability are prerequisites: otherwise the tray cannot show asks or
open authenticated links.

### 4. Root agent verifies; Ada checks the UI

On review-host in the active GUI session:

```sh
nix run .#nix.activations.aarch64-darwin.review-host.attention-tray.verify
codesign --verify --deep --strict "$HOME/Applications/AttentionTray.app"
launchctl print "gui/$(id -u)/com.schickling.attention.tray"
```

The shared domain resolver selects `gui/<uid>` when Aqua exists, else
`user/<uid>`; use that same resolved domain for inspection outside a GUI
session. Expect one job with a live PID and no repeated nonzero exits.
Logs are `~/.local/state/attention-tray/{stdout,stderr}.log`; do not dump
credential files or process environments when diagnosing failures.

Ada opens the menu-bar icon and checks that configuration is valid,
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

For retirement/rotation the root agent drives the original member's idle person PTY:
`fractal devices --as person/ada revoke device/DEVICE_ID --reason "Retired Attention tray grant"`.
Re-pair for expiry, update the inventory and restart only this tray job in the
resolved domain so it rereads the profile. Do not restart Fractal or Boss.
