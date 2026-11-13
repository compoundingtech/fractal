# Attention tray

A native macOS 14+ SwiftUI menu-bar client for one person's Fractal: the
attention inbox and the objectives every Fractal client reads. It reads only
Fractal, through `FractalClient`, at launch and every 30 seconds.

- **Asks** (all attention pages, classified as Fractal web's
  `data/attention.ts` does) are answered in the panel: a decision's or choice's
  named answers (recommended first; request-changes takes words), a choice's
  custom words or feedback text, Yes/No for an untyped question, or Nothing to
  do when there are no named answers. Custom forms and harness prompts link to
  Fractal. Controls appear on hover or focus.
- The tray offers no Dismiss and no snooze. Web's Dismiss sends a `work.done`
  that the asker reads as an answer; dismissing and snoozing are person
  attention controls, distinct from answering and shared by every client.
- Answers mirror Fractal web `alert-action-port.ts`: re-read the card, require it
  open, offering `work.done` and at the revision the person read, then submit
  one `work.done` fenced on that read. The action ID and idempotency key are
  derived from the card revision and the whole answer, so a repeated send of the
  same answer replays Fractal's first receipt. A card shows "Sending…", then
  "committed" only after Fractal's result, or Fractal's refusal verbatim; it is
  never removed optimistically.
- **Updates** are listed apart and not counted; Mark read sends the server's
  prefilled `work.done` (`Read`, answer `read`).
- **Objectives** (`GET v1/client/slo`): rows by Fractal's served tone, missed
  first, then stale, then met; unmeasured objectives collapse; budgets follow.
  The menu bar adds a gauge when any objective is missed or budget nearly spent.
- Asks and objectives refresh independently: one failing keeps its last data
  with its own stale note and never blanks the other.
- Times are relative with the absolute time on hover. The footer shows the build
  identity; launch and each change of refresh outcome are logged to stderr.

## Packaging and configuration

`nix build ./flakes/attention-tray#attention-tray` produces an unsigned
`Applications/AttentionTray.app` for `aarch64-darwin`, using Apple's host Swift
SDK. SwiftPM consumes the exact locked Fractal Swift client via
`FRACTAL_CLIENT_PATH`; builds use two Swift jobs. The standalone flake pins
Fractal `main`; the root tray input follows the root Fractal source, so the
deployed tray uses the client of the daemon it talks to. The tray uses only
client API present in both.

The bundle carries `Contents/Resources/nix-build-stamp.json` (`type`, `version`,
`rev`, `dirty`, `fractalClientRev`). Its `rev` is `src.<hash>` of the tray's own
sources plus the FractalClient revision, so unrelated repository commits neither
rebuild nor restart the tray; the footer shows `machineVersion`
(`0.2.0+src.<hash>`) with the FractalClient revision on hover.

Run tests with `nix develop ./flakes/attention-tray -c /usr/bin/swift test`.
The compiler-free shell avoids nixpkgs' xcrun stub; explicit `/usr/bin/swift`
uses Apple's host toolchain, consistent with the package build.

Standalone launches require three explicit environment variables:
`ATTENTION_FRACTAL_ENDPOINT`, `ATTENTION_FRACTAL_CREDENTIAL`, `ATTENTION_FRACTAL_URL`.
There are no endpoint, localhost, or Boss fallbacks. The application itself
loads no keychain, 1Password item or device-profile file.
`ATTENTION_TRAY_SNAPSHOT=<png>` refreshes once, renders the open panel to that
file and exits (`ATTENTION_TRAY_SNAPSHOT_REVEAL=1` shows every card's controls).

On review-host, `services.attentionTray.enable = true` imports the Home Manager
module. Its activation bundle installs `~/Applications/AttentionTray.app`, signs
it ad-hoc, registers it with Launch Services and converges the GUI launch agent
`com.schickling.attention.tray`. Its dual `KeepAlive.PathState` conditions start
it while either the configured profile or its legacy filename exists;
`RunAtLoad` is absent. An unpaired installation is inert, including across
unrelated Home Manager activations.
Once paired, missing web prerequisites fail with a clear wrapper log under
`ThrottleInterval = 60`, not a tight loop. There is no separate Login Items or
SMAppService enrollment. Changed paired payloads restart once; unchanged
activation does not restart the process.

Activation verification checks only installed build/configuration facts
(signature and registered launchd job). Runtime credentials and web availability
are checked by both the launcher and the separate `attention-tray-readiness`
command, never by the Home Manager activation. The module asserts that the
rendered XML has `PathState` and no `RunAtLoad`.
This matches [launchd.plist(5)](https://www.manpagez.com/man/5/launchd.plist/osx-10.11.6.php):
`PathState = true` keeps the job alive while the path exists; unlike
`SuccessfulExit`, it does not imply `RunAtLoad`. The shared activation helper
installs the rendered plist directly; neither nix-darwin nor HM adds a launch
condition. Filesystem notifications can be lossy: if a completed pairing does
not start it, inspect the resolved domain and kickstart only this paired job.

The stable wrapper reads one dedicated paired-device profile at
`~/.config/attention-tray/fractal/fractal-devices.json` (regular, owned, readable
owner-only `0400` or `0600`; parent `0700`). It accepts exactly one
`person/ada` device whose `session.scopes` is exactly the pinned
`--read-only` observation set: `read.projections`, `read.glasses`, `terminal.read`.
Missing scopes, extra scopes and a broader person grant fail closed. The profile
stores that session at pinned `device.rs:166-177`; `generated.rs:4523` supplies
the scopes field and CLI `main.rs:2023` defines the read-only set.
The tray also reads `v1/client/capabilities` and shows a read-only note
instead of answer controls unless `work.done` is granted.
Its endpoint and bearer are exported only to the app process. Never put the
bearer into Nix options, launchd configuration, shell arguments or logs.

Launcher/readiness preflight atomically migrates the old `stui-devices.json`
filename to `fractal-devices.json` with a no-replace rename
(`renameatx_np(RENAME_EXCL)` on Darwin, `renameat2(RENAME_NOREPLACE)` on Linux).
The rename preserves the inode and mode and never overwrites an existing
destination, even in a race; the new file wins and a warning is logged. Dual PathState
conditions let an existing profile start the launcher before migration.
Activation also recognizes the old file when deciding whether a changed paired
payload must restart. New pairing writes the configured filename directly.

Run the standalone Linux migration check from the repository root:

```sh
gate-slot --class eval --max-wait 3600 -- nix build --impure --no-link --option builders '' --option max-jobs 2 --expr 'let root = builtins.getFlake ("git+file:" + toString ./.); pkgs = import root.inputs.nixpkgs { system = "x86_64-linux"; }; in (import ./nixpkgs/shared/attention-tray-profile-migration.nix { inherit pkgs; }).test'
```

The Fractal URL has one source of truth: the configured local Fractal daemon's
`<services.smalltalk.stateDir>/web-launch-url`. This is the same private
rendezvous consumed by `fractal web`; the wrapper retains only the HTTP origin,
not its one-use login token. The same owner-only file check applies to this
rendezvous and to the device profile. Missing runtime prerequisites do not
block activation: an explicit launcher/readiness diagnostic identifies them.
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
product-directory defaults. No upstream migration receipt is needed.
The completion `.profile` check is mandatory before deployment and proves the
CLI writer and HM consumer agree. Do not print completion/error files or the
saved profile. Report only status; record actual nonsecret device inventory
through a separately allowlisted metadata read after pairing.

Pinned source evidence (`05b5f5d96c369a37c18766b4740b80959c320102`):
`crates/fractal/src/main.rs:2042-2044` defines `--read-only`,
`:2071-2084` defines `--profile` and `--fingerprint`, `:8738-8745`
uses the explicit profile before considering the product-directory default,
and `:8747-8757` emits a safe completion JSON with `.profile`.
`crates/fractal-client/src/device.rs:921-929` accepts non-terminal stdin.
The challenge fields are defined in `crates/fractal-client/src/generated.rs:4476-4483`;
`main.rs:8898-8905` preserves the `.value` envelope for JSON output.
`clients/swift/FractalClient/Package.swift:5-10` exports `FractalClient`;
`Sources/FractalClient/Client.swift:253` retains `attentionList`.
`crates/fractal-api-core/src/api.rs:592-593` routes attention GETs to
`api/client_reads.rs:848-892`, which returns person-filtered projection pages,
not `attention-migrated`. `api/client_v0.rs:789-801` admits these GETs with
`read.projections`, included in the read-only set above. The migration error
belongs to the legacy mutation `attention.resolve`
(`crates/fractal-api-client-v0/src/api/client_v0.rs:7735-7738`), not these reads.
At the pinned Fractal `3b8e57cb`, `attention.resolve` is always refused as
`attention-migrated` (`crates/fractal-api-client-v0/src/api/client_v0.rs:9479`):
answers and reading an update both complete the source person step
with `work.done`, whose update parameters the server prefills
(`crates/fractal-api-core/src/api.rs:5078`). `work.done` needs
`control.attention` (`crates/fractal-api-core/src/api/client_v0.rs:4573`
`action_scope`). Action admission requires an `action/` ID and a 16-256 byte
idempotency key (`client_v0.rs:9461`) and replays a key's receipt only for the
same request digest (`:9375-9400`).

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
web availability are runtime-readiness prerequisites, not activation
prerequisites: merging and activating before pairing is safe. Activation
verifies only build/config facts, and the agent stays idle until the profile
exists. Once paired, the wrapper logs unavailable web state under the 60-second
launchd throttle.

### 4. Root agent verifies; Ada checks the UI

On review-host in the active GUI session:

```sh
nix run .#nix.activations.aarch64-darwin.review-host.attention-tray.verify
codesign --verify --deep --strict "$HOME/Applications/AttentionTray.app"
launchctl print "gui/$(id -u)/com.schickling.attention.tray"
attention-tray-readiness
```

The shared domain resolver selects `gui/<uid>` when Aqua exists, else
`user/<uid>`; use that same resolved domain for inspection outside a GUI
session. Before pairing, expect a registered but idle job; activation verify
must pass and `attention-tray-readiness` must report `not paired: <path> missing`.
After pairing and browser startup, the separate readiness command validates
private-file modes, exact read-only scopes, runtime configuration and a live
launchd PID; expect no repeated nonzero exits.
Logs are `~/.local/state/attention-tray/{stdout,stderr}.log`; do not dump
credential files or process environments when diagnosing failures. The stderr
log has one line per change of refresh outcome, for example
`attention ok (3 asks, 1 updates); slo ok (28 rows, 7 need attention)`; a
transport refusal such as `NSURLErrorDomain -1022` or `credential-rejected`
appears there verbatim.

Ada opens the menu-bar icon and checks that configuration is valid,
freshness is current (not stale), current pending asks appear and objectives
match `fractal slo` run on the paired member (dev5): member-measured objectives
such as `person-read` or `cpu` describe that member. Compare with Fractal's authenticated
attention view, respecting the tray's supported-answer filter and full
pagination rather than comparing to an unfiltered first page. Wait at least one
30-second interval and observe refresh. Answering is proven on an ask raised for
the purpose (for example `fractal work ask` from an agent seat), never on a real
one: the card shows Sending, then committed, and leaves on the next read; the
asking agent sees the answer. Open in Fractal must open `/w/<agent>` for its
conversation/requester, not a Boss URL, and the authenticated page must load.

Run a second unchanged sanctioned activation only if rollout verification
requires it; compare the launchd PID and confirm there was no extra restart or
permission prompt. Do not revoke/re-pair merely to retry a display failure.

For retirement/rotation the root agent drives the original member's idle person PTY:
`fractal devices --as person/ada revoke device/DEVICE_ID --reason "Retired Attention tray grant"`.
Re-pair for expiry, update the inventory and restart only this tray job in the
resolved domain so it rereads the profile. Do not restart Fractal or Boss.
