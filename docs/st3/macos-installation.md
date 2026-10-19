# macOS installation and signing

Source installs (`scripts/install`) and extracted release installs (`install.sh`) put
`st3` inside `~/Applications/SmallTalk.app`. The selected bin directory
contains links to these tools, with `st` linking to `st3`. Other tools remain regular
executables in that directory. Python 3 is required on macOS.

The app path and bundle identifier stay stable across updates. The installer registers
the app with Launch Services and points existing direct daemon and replication
LaunchAgent commands at its executable, preserving their remaining arguments.
Wrapper-based definitions, including staged and completed Home Manager wrappers,
remain byte-for-byte unchanged: the wrapper owns its guard, config and state paths.
The installer does not restart services or agent seats. If a direct definition needs
a different executable path, stop its loaded service before installing; the installer
refuses to rewrite a loaded definition.
For a first daemon install, run `st service install` from the installed app executable;
for an update, restart the daemon and replication services when ready.

When the `--from` directory also holds `StListen.app` (the terminal UI's speech helper, built by
`apps/macos/listen/build.sh DIR` with Xcode's Swift and the macOS 26 SDK), the installer
nests it at `SmallTalk.app/Contents/Helpers/StListen.app`. It signs the helper before the app,
and gives the app the microphone and speech usage strings. macOS asks for the microphone on
behalf of the outermost app, so the prompt names Fractal and the answer lasts across updates
signed with the same identity. `scripts/install` and release archives build the helper when they
can; without it, the terminal UI's voice mode says that it is missing.

Configure one persistent signing identity to retain its code identity across builds:

```sh
export ST_MACOS_SIGNING_IDENTITY='Developer ID Application: Example Developer (EXAMPLE123)'
export ST_MACOS_SIGNING_TEAM='EXAMPLE123'
scripts/install --from target/release
# An extracted release uses the same settings:
./install.sh
```

The identity can be an exact keychain identity name or its full SHA-1 certificate
fingerprint. It must be explicitly configured: the installer never chooses an identity
from the keychain. When a team is configured, the selected certificate's team is checked
before signing and the installed signature is checked again. A missing, ambiguous or
wrong-team identity fails clearly; it never falls back quietly.

With no signing configuration, installation uses ad-hoc signing. This preserves the
existing installation option, including macOS approval prompts after code changes.
A team without an explicit identity is an error. Persistent signing preserves a stable
designated requirement, but existing macOS permission grants may still need one initial
approval when moving from a prior ad-hoc installation.

Optional settings are `ST_MACOS_APP_PATH`, `ST_MACOS_BIN_DIR`, and
`ST_MACOS_BUNDLE_ID`. Choose a fixed path and identifier and keep them unchanged.
`--bin-dir` on the install wrapper overrides the bin-directory setting. The shared
Python helper also accepts `--identity`, `--team-id`, `--app` and `--identifier`.

Before replacing anything, the installer signs and verifies a complete candidate app.
An unchanged payload with the same signing settings keeps its existing signed bundle.
A changed app replaces the whole bundle through an atomic directory swap on the same
filesystem. The installer keeps the prior app, CLI links/files and service plists under
`~/.local/state/st3/macos-installs/`. The helper restores them if its own installation
fails, while still holding its shared transaction lock.

The extracted archive caller also keeps prior command files and transaction locks.
If a later archive step fails after the helper completes its backup, the caller leaves
the current app and command links intact and reports the retained job and files.
The helper's existing restore API cannot check ownership atomically against other
source or historical installers, so the archive caller does not restore the app
automatically after that point. Inspect any newer installation before using the
retained job for recovery, and remove the reported locks only after recovery.

Automation can use the helper's explicit transaction modes (`--prepare-only`,
`--backup-only`, `--install-app APP`, `--verify-app APP`, `--restore-app`) with a
persistent `--job DIRECTORY`. Isolated staging/tests can set `--home DIRECTORY`
(or `ST_MACOS_INSTALL_HOME`) so app metadata, service plists and locks stay outside
the real user installation. Back up before `--install-app`, and keep the directory
until deployment health checks pass. Artifact verification/install with a configured full
certificate fingerprint checks the embedded signature without requiring its private key
on the receiving machine. Building/signing a candidate still requires the explicitly
configured keychain identity. The helper restores installation files, not
application databases; schema compatibility must be checked by the deployment caller.

<!-- LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032 -->
For the default-host Fractal cutover, `st service install --stage-fractal` installs
`com.compoundingtech.fractal` and `com.compoundingtech.fractal.replication` definitions
disabled without loading or starting them. On Linux the same command stages
`fractal.service` and `fractal-replication.service` disabled. Existing legacy service
definitions and drop-ins remain untouched. The future definitions use the XDG
`fractal` config/state directories and retain legacy socket paths; the host migration
must move state/config, provide the legacy socket symlink bridge, create the target
log directory, and activate services. Staging rejects loaded macOS targets or active
Linux targets, named instances, and state/config paths outside the default migration roots.
The Python bundle helper also accepts `--stage-fractal`: it leaves legacy plists
unchanged and invokes native staging from the installed app, never bootstrapping it.
Use the same flag for `--backup-only` when preparing a staged bundle transaction.
Staging backups record both present and absent target plists and the target labels'
launchd disabled overrides. Restore removes definitions created by staging, restores
previous definitions and returns each label to its prior enabled/disabled state,
without bootstrapping jobs or changing legacy or unrelated services. An absent
override means enabled by default; because launchctl has no per-label unset operation,
restore uses an explicit `enable` for that case. Target jobs must still be unloaded
when restoring a staging transaction.
Normal installers retain legacy service names until cutover.
<!-- LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal -->

A second install for testing: `scripts/install --instance NAME` (or `install.sh --instance NAME`) puts
the app, commands, bundle identifier and launchd services below `~/.st-instance/NAME` and never
rewrites the default install's plists. See [Instances](instances.md).
