# Manual onboarding test rig

`scripts/onboarding-e2e`, `scripts/onboarding-vm-test` and
`scripts/onboarding-mac-test` share one development runner. They install a verified release
archive in disposable machines; they do not build the candidate, install real providers,
copy provider logins or run paid models. The real-machine backends refuse CI environments.

Run commands from the repository root. Inspect the current options without provisioning anything:

```sh
scripts/onboarding-e2e --help
scripts/onboarding-vm-test --help
scripts/onboarding-mac-test --help
```

Repository qualification follows the [review standard](../review.md#what-a-go-means), which
requires native instances or VMs. Select `--backend incus` or `--backend external` for Linux;
use Tart for macOS. The shared `onboarding-e2e` entry point defaults to a container backend,
so select the VM backend explicitly. The runner executes scenarios sequentially.
Use a fresh output directory for each attempt; an existing directory is refused.

## Candidate and harness boundaries

Supply the immutable release archive with `--archive PATH`. A local archive needs its adjacent
`.sha256` file or `--sha256 HEX`; substitute its actual SHA-256 for `HEX`. Use an archive for
the guest's architecture and libc baseline. Package metadata alone does not prove compatibility
with an older guest. `--strict-release` adds the runner's archive/version checks.

Linux provider scenarios use stand-ins from `scripts/fractal-boot-canaries` and
`scripts/onboarding-fixtures`. `--harness-only` checks driver/channel plumbing independently of
the built-in Fractal Assistant. `--require-wrap-up` instead requires a completed onboarding
mission; the two options are mutually exclusive. A stand-in read/claim pass is not model or
onboarding-content evidence. The separate [onboarding play](../../evals/fractal/onboarding-play/README.md)
controls its own explicitly paid runs.

## Linux guests

The small-machine scenarios currently depend on a container backend and are outside this
VM procedure. For a disposable VM, prepare an account that already has Incus and KVM access, a default
profile with a root disk and network, and capacity for one guest. The runner does not install
host packages, initialize Incus, change host groups or use sudo. Image download, cloud-init and
guest package installation require network access. Coordinate the VM with other host users.

For a prepared host and matching candidate archive:

```sh
scripts/onboarding-vm-test setup --backend incus --ubuntu 24.04 \
  --archive /tmp/fractal-candidate.tar.gz --sha256 HEX \
  --out /var/tmp/onboarding-vm-candidate
```

The managed backend creates a uniquely named Ubuntu cloud VM, prepares its `ada` test user and
user manager, installs the archive, and checks setup, service and PTY behavior. Its reboot
check probes the user manager before a test-user login can hide missing lingering. By default
it removes only its own guest; `--keep` retains that guest for inspection.

For a separately prepared fresh guest, select `--backend external` and name its actual Ubuntu
release with `--ubuntu`. Provide `ST_VM_EXEC` for commands as `ada`, with the correct guest HOME,
XDG runtime directory and user-bus address. It must pass standard input through.
`ST_VM_ROOT_EXEC` supplies the privileged guest commands needed for reboot proof;
`ST_VM_REBOOT` can override the guest reboot command. These values are split into argv without
shell evaluation; use a wrapper executable for complex transports. The external backend
creates and deletes no machine. Restore or remove that disposable guest yourself after reading
its report; never target a workstation's regular home.

## macOS guests

Run Tart manually on an Apple Silicon Mac with Python and noninteractive SSH available.
Prepare a clean local base VM before invoking the runner. Inside it, create the test user
`ada`, arrange automatic GUI login and an unlocked login session, enable Remote Login for that
user, and install Python 3.11 or newer in its fresh login PATH. Leave Fractal, provider CLIs,
GitHub CLI and developer toolchains absent. Do not pre-grant the app's privacy permissions.

Authorize only a dedicated host test key's public half in the guest's
`/Users/ada/.ssh/authorized_keys` (directory mode 0700, file mode 0600). Keep the private key on
the host and unlock it in the host's SSH agent if necessary. Verify noninteractive key access,
that `ada` owns `/dev/console`, and that `launchctl print gui/UID` succeeds for the actual UID;
SSH login alone does not create the GUI domain used by the service. The person prepares OS
settings, passwords and keychain access directly. Stop the prepared base before the run.

With that prepared base, key and matching `aarch64-apple-darwin` release archive:

```sh
scripts/onboarding-mac-test --scenario first-run \
  --archive /tmp/fractal-aarch64-apple-darwin.tar.gz --sha256 HEX \
  --tart-image studio-clean --ssh-key /path/to/onboarding-test-key \
  --strict-release --keep --out /var/tmp/onboarding-mac-first-run
```

Tart supports `setup`, `first-run` and `no-harness`, using a fresh clone for each selected
scenario. The runner clones only the named local base, checks installed app/signing identity,
setup, login PATH, launchd service and real guest PTY behavior, then power-cycles its clone.
The final service check is after GUI auto-login; it does not prove service availability before
login. `--keep` retains the clone. Privacy guidance printed by the service does not establish
that macOS granted access: inspect the retained guest window and test service-owned access.
Any signing identity or team must already be explicitly prepared in the guest keychain.

## Evidence and cleanup

Read each case's `report.md` and `result.json`, alongside the captured commands, PTY and service
logs. A baseline's expected failure or unexpected pass is distinct
from a candidate pass. Keep failed receipts when comparing attempts.

`--no-reboot` explicitly omits reboot proof and must be disclosed. A Docker restart is not a
VM boot. Stand-in transport, successful help, permission guidance and app signature checks are
separate evidence boundaries; none proves a paid model, platform permission or content result.
For a retained instance, inspect its exact recorded name and remove only that instance when
finished. The external backend leaves lifecycle and cleanup to its operator.
