# Fractal with Home Manager

Import `inputs.smalltalk.homeManagerModules.default` and configure the user service:

```nix
services.smalltalk = {
  accessMode = "auto";
  enable = true;
  person = "person/ada";
  declarations = {
    seats = [ ./seat.kdl ];
    missions = [ ./mission.kdl ];
  };
  # ptyPackage = inputs.pty.packages.${pkgs.system}.default;
};
```

The module installs `st3`, writes `st3/config.toml`, and starts a systemd user
service on Linux or a launchd agent on macOS. When declarations are provided,
an additional oneshot service applies seat files and publishes mission files
as `person`, retrying while the daemon starts. `declarationsApply.enable = false`
disables this step. Reapplying is safe, but removing a file does **not** delete
its previously published declaration; this module does not yet use managed-set apply (#646).
The optional `ptyPackage` replaces the bundled `pty` for both the daemon and
seats, working around the executable-directory PATH precedence in #633.

On Linux, `memoryMax` (default `8G`) and the optional `memoryHigh` set the daemon's systemd memory
limits; `null` leaves a limit unset. Size `memoryMax` above the host's measured peak RSS: catch-up
after downtime raises memory use, and an OOM kill restarts the daemon into the same catch-up.

Activation atomically installs a real `st3` executable at `stateDir/bin/st3` (by default
`~/.local/state/st3/bin/st3`) before restarting the daemon, with `st` as an alias in that directory.
The daemon, declaration-apply service, and service PATH use this stable location so running seats
can follow new builds without ending their harness sessions. Unchanged contents are not rewritten.
Package references in the systemd unit or launchd plist still trigger daemon restarts on upgrades.
Seats started from store paths before this change remain stale until restarted; see
[seats across deploys](st3/seat-deploys.md).

## Optional Fractal command

Neither the default Nix package nor the default Home Manager configuration installs
`fractal`; updating a Smalltalk pin does not claim that command name. To opt in for
the Home Manager-owned stable executable:

```nix
programs.smalltalk.fractalBinary.enable = true;
```

With `services.smalltalk.enable = true` and `binary.path = null`, activation creates
`stateDir/bin/fractal -> st3`. Add that stable bin directory to your interactive PATH
to use it. `fractal` and `fractal tui` open the terminal UI, `fractal web` opens the web
UI, and other arguments behave like `st`. The existing `st` and `st3` commands do not
change.

Activation leaves foreign files, directories, and symlinks at `stateDir/bin/fractal`
untouched and prints guidance; `st` remains usable. A relative `fractal -> st3` link
is recognized as Smalltalk-owned. Disabling the option removes only that recognized
link, not a foreign command. With an externally deployed `binary.path`, this option
is ignored: the external deployer remains responsible for aliases.

For a standalone Nix-profile installation, explicitly select the flake's `#fractal`
package instead of the default package. It contains only `bin/fractal`, a symlink to
the same `st3` executable; install `#st3` alongside it if you also want the existing
commands in your profile. Unlike module activation, Nix profile collision handling
governs profile-installed commands.

## Opt-in host cutover staging

<!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032 -->
Updating the module alone retains the legacy service names and state directories.
For an existing installation using the default `stateDir`, explicitly stage the
replacement before performing the host cutover:

```nix
services.smalltalk.fractalMigration.phase = "stage";
programs.smalltalk.fractalBinary.enable = true;
```

Apply that Home Manager configuration first. It installs `fractal.service` and,
when declarations are configured, `fractal-apply.service` on Linux. Neither unit
has `WantedBy`; staging does not enable or start them. On macOS it writes
`~/Library/LaunchAgents/com.compoundingtech.fractal.plist` and the optional
`com.compoundingtech.fractal-apply.plist` without loading them. The daemon plist
already includes `RunAtLoad` and crash-restart `KeepAlive` flags for the eventual
cutover. Persistent launchd disablement, not missing lifecycle flags, prevents
staging from starting it; staging never bootstraps either job. The staged plists
remain outside Home Manager's automatic launchd agent lifecycle. This module does
not manage replication; retain your existing replication setup and include it in
the host cutover.

Staging retains the existing legacy service definitions unchanged, so it does
not intentionally restart or stop the legacy daemon. Stage the same package
and service settings as the running generation; changing those settings still
has Home Manager's normal upgrade behavior. Staging does not rename directories,
move drop-ins, or stop services.

After staging, run `fractal migrate host` for the deliberate per-host cutover.
That command owns stopping legacy services, renaming configuration and state,
writing the receipt, creating migration symlinks, starting the replacement,
probing its API, and retiring legacy service files and whole drop-in directories.
Do not start the target units manually before this handoff.

Target commands use `~/.config/fractal/config.toml` and
`~/.local/state/fractal`; their stable binary is in the migrated state's `bin`
directory. The migration's `st3 -> fractal` directory symlinks keep old binary
and socket paths accessible. External `binary.path` ownership remains unchanged.
The target refuses to run without the exact state-migration marker followed by
a newline in **both** `~/.config/fractal/.fractal-migration-completed` and
`~/.local/state/fractal/.fractal-migration-completed`, while either legacy
directory is a real directory rather than a symlink, or while a legacy daemon
is active. The old service definition may remain during the API probe for
rollback; its mere presence does not block the deliberate target start.

After the cutover succeeds, set:

```nix
services.smalltalk.fractalMigration.phase = "completed";
```

Apply Home Manager again to complete the declarative ownership handoff. This
phase removes legacy definitions and manages only the Fractal services. It
requires valid migration receipts, migrated directories, an inactive legacy
daemon, and the persistent configuration fence **before** activation's write
boundary. Both legacy and stage phases fail before that boundary if either
receipt path exists (including an incomplete receipt), requiring the explicit
completed configuration instead of resurrecting the legacy service. Completed
services can start normally on subsequent activation/login; their runtime guards
remain in force.

The host cutover preserves a foreign `fractal/config.toml` symlink pointing to
`config.home-manager.toml`, a regular TOML file whose final line is
`# arn:lmig:smalltalk:2026-10-16-service-labels-fractal Home Manager legacy-generation fence`.
Completed activation atomically updates that backing file with the marker intact;
neither configuration path is owned by Home Manager's file linker. Through the
legacy directory bridge, an immutable pre-cutover generation sees this foreign
symlink and fails its existing `checkLinkTargets` step before it can rewrite the
binary, config, or service definitions. The marker makes the contents unequal
even when the settings otherwise match. A regular TOML file alone would **not**
be sufficient: Home Manager's backup extension/command modes accept regular-file
collisions, but reject this foreign symlink.

Home Manager's explicit `force = true` bypasses collision checks altogether.
Stage/completed configurations reject forced ownership of `st3/config.toml` or
a containing path. Do not remove the foreign link; old generations with force
overrides are not safe rollback candidates and must be removed before cutover.
Likewise, do not bypass `checkLinkTargets` when invoking
old activation scripts. The migrator must install the journaled fence before
publishing the legacy configuration directory bridge and disable old boot/login
activation before moving directories, so an interrupted cutover cannot leave
an unfenced legacy generation or job writing into migrated state.
<!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal -->

## Externally deployed binary

If another deployer updates your Nix profile or installs the executable, set
`binary.path` to its stable absolute path:

```nix
services.smalltalk = {
  enable = true;
  person = "person/ada";
  binary.path = "/opt/smalltalk/bin/st3";
};
```

The default is `null`, which keeps the installation and restart behavior above.
When a path is set, Home Manager does not install `package`, copy an executable
to `stateDir/bin`, or create the `st` alias there. The daemon and declaration
commands execute `binary.path`, and their PATH starts with its containing
directory. The daemon's `--pty-binary` also selects `pty` in that directory.
The systemd units and launchd agents omit the Fractal and PTY package store
paths and the package restart trigger. Changing only `package` or `ptyPackage`
therefore does not restart the daemon on a Home Manager switch.

The external deployer must install the executable before the service starts,
provide any CLI aliases (such as `st`), install `pty` beside the configured
executable, and restart the daemon when an upgrade needs it. `ptyPackage` is
ignored in this mode, so a profile update selects both tools from the same
stable directory.
For running seats to follow replacements, use a real executable at a stable
path: a Nix-profile symlink alone resolves to the package's store executable
and does not provide the replaceable executable described in
[seats across deploys](st3/seat-deploys.md).

## Host setup

Linux user-manager lingering and macOS `st service permissions` remain host
setup prerequisites. Fleet/replication setup is not managed by this module.

See [getting started](getting-started.md) for harness login and your first mission, and
[upgrading st](upgrading-st.md) for checking a deployment.
