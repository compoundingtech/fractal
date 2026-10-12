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
| auto | Named `st-managed` profile extending `:workspace`, the seat's workspace root, networking enabled (also through `sandbox_workspace_write.network_access`), and the daemon socket explicitly allowed; live web search; `on-request` approval with `auto_review`; `--dangerously-bypass-hook-trust` | `--permission-mode auto` |

These are driver defaults for st-launched seats. They do not modify the provider's
account settings or change a manually launched provider. Authored permission options
and Codex config profiles remain authoritative, including old bypass arguments.
The daemon supplies the resolved mode through `ST_ACCESS_MODE`, including `bypass`
for an existing config without the key. Hand-run `st driver` commands without this
environment value preserve their provider arguments; they can set it explicitly
to use the install policy.
New seat and planner declarations omit permission arguments. An automatically resumed
Codex thread receives its named profile through `thread/resume`, before the TUI
loads it, and the driver verifies the returned active profile.

What the auto profile does for a Codex seat, as run through the real launch path (an app server
plus a remote TUI) on Codex 0.160.1:

- It writes the seat's workspace and the temporary directories and reads everything else. A write to
  the home directory or anywhere else fails with a read-only file system.
- It reaches the st daemon socket and the network (git and gh over https, package registries). The
  remote TUI that owns the thread flattens the named profile and drops its `network` table, so the
  profile alone leaves the daemon socket at `EPERM`. The driver therefore also passes
  `sandbox_workspace_write.network_access=true`, the one network key that reaches the thread.
- Web search is live, because Codex only defaults to live search with no sandbox.
- It cannot write the workspace's own `.git`, or the home-directory package caches, without an
  escalation. Codex keeps `.git` read-only, and a remote TUI rejects `--add-dir` and
  `sandbox_workspace_write.writable_roots` ("configure additional workspace roots on the server")
  and ignores `filesystem` entries in a named profile, so no setting grants them. A seat that needs
  a commit, a push or a package install asks for an escalation, which `on-request` approval with
  `auto_review` answers. That is the next narrowest setting; the bypass would be wider.

Codex also cannot express two of the rules the policy wants. It cannot allow the network while
naming the only local socket: with networking on, any local Unix socket the seat's OS user can
reach connects. That includes other seats' PTY and attach sockets, the ssh agent, and any local
daemon socket. With networking off, even the named socket is refused. A control in
`crates/st-drivers/tests/codex_managed_sandbox.rs` keeps this visible and fails the day Codex
learns the rule. And Codex's `domains` allowlist is not enforced by `codex sandbox`, so egress is
not narrowed to git, gh and registries.

The live eval `evals/st3/codex-minimal-access` is paid and run by hand; no CI gate proves the real
launch path. The argv test and the sandbox tests are the CI evidence.

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
