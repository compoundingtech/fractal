# Managed harness access

The install's `config.toml` selects `access_mode = "auto"` or `"bypass"`.
Service installation writes `auto` when creating a new config. An existing config
without this key means `bypass`; installing again preserves its bytes. Home Manager
keeps its existing default and accepts `services.smalltalk.accessMode = "auto"`
for a new installation. Restart the daemon after changing the mode; `st doctor`
shows the mode that daemon actually started with.

Auto is not a security boundary. It reduces accidents; it does not isolate seats.
Real sandboxing is separate later work. Codex hook trust is bypassed in both modes.

| Mode | Codex | Claude |
| --- | --- | --- |
| bypass, including a missing key | `--dangerously-bypass-approvals-and-sandbox` and `--dangerously-bypass-hook-trust` | `--dangerously-skip-permissions` |
| auto | Named `st-managed` profile extending `:workspace`, the seat's workspace root, networking enabled, and the daemon socket explicitly allowed; `on-request` approval with `auto_review`; `--dangerously-bypass-hook-trust` | `--permission-mode auto` |

These are driver defaults for st-launched seats. They do not modify the provider's
account settings or change a manually launched provider. Authored permission options
and Codex config profiles remain authoritative, including old bypass arguments.
New seat and planner declarations omit permission arguments. An automatically resumed
Codex thread receives its named profile through `thread/resume`, before the TUI
loads it, and the driver verifies the returned active profile.

Codex's workspace profile allows reads and temporary-directory writes according to
its built-in rules. On Linux, enabling networking also permits other Unix sockets:
the socket entry is not a socket-only security boundary. Writes outside the workspace
and permitted temporary roots remain restricted. The recorded Codex 0.160.1 controls
in `crates/st-drivers/tests/fixtures/codex-managed-profile-probe.json` show workspace
writes and socket connection succeeding, an outside write failing with a read-only
filesystem, and a socket connection failing with EPERM when networking is disabled.
The fixture also retains an invalid initial probe invocation and the successful
outside-workspace `/tmp` write, so neither is mistaken for sandbox enforcement proof.

Codex update checks remain disabled with `check_for_update_on_startup=false`.
Managed Claude runs with `DISABLE_AUTOUPDATER=1`; pi and omp already disable their
startup update checks. When a recognized update menu still appears, the daemon
records a `harness.diagnostic` with code `provider-update-available`, status
`available`, provider, incarnation, and the observed notice. Its evidence points
to the retained startup-prompt diagnostic. It remains after the prompt is resolved
and can be read through the seat's trace. Operations owns provider upgrades.

The Codex Skip behavior from #2332 still requires the complete recognized menu,
a fresh screen and the same incarnation. Failed input, changed screens and a menu
that does not clear retain the attention path; a partial notice receives no key.

Provider references: [Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference)
and [Claude environment variables](https://code.claude.com/docs/en/env-vars).
