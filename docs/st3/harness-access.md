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

A resumed thread keeps the same access. Codex 0.160.1 refuses any permission config on a remote
resume ("Permission overrides are not supported when resuming a remote task"), so the resume argv
omits the profile's `-c` entries (and an authored `-c` entry for the same keys): the app server holds them and `thread/resume` selects the profile and carries the approval policy, reviewer, sandbox, model and effort; `sandbox_workspace_write.*` stays on the app server only.
Before this, a plain-declaration Codex seat that restarted on an auto install had its TUI exit and the
driver started a new thread instead of resuming (the old rollout stayed on disk, unreferenced).

The approval reviewer (`approvals_reviewer = "auto_review"`) can reject an unattended seat's push of its
own branch with "a fleet-agent message cannot establish the required trusted user authorization", however
the mission step quotes its scope, because only a user turn counts as authorization. It is not
deterministic: on 2026-10-13, on Codex 0.160.1 with gpt-6.1-sol, a plain seat's push was rejected 3 times
in 7 live runs. A seat can opt in to a standing authorization by declaring the permission in its `env`:

```kdl
env {
  ST_PERMISSIONS "push-own-branch"
}
```

It is off by default, applies only to auto-mode Codex seats (Claude's classifier and bypass seats are
unaffected), and an unknown permission name fails the launch. It sets `auto_review.extra_policy` to a text
that says, for these actions only, the operator is the user: committing in the workspace, pushing the
branch the seat created for its assigned step to the workspace's own `origin`, and opening or updating that
branch's pull request with `gh`; a st-delivered mission step counts as that authorization for exactly
those actions and nothing else. It names what stays unauthorized: the default branch, another seat's
branch, deleting remote branches, merging or closing pull requests, and any other remote or repository.
With it the push succeeded 10 times in 10. The wording matters ("the operator authorized" was rejected)
and is a behavior of that model version; `scripts/st3-codex-minimal-eval/push-check` re-runs the probe.
Nothing is opted in by this change; opting a seat in is the operator's decision, made in its declaration.
st does not restrict who may publish that declaration: in [free mode](kdl-lifecycle.md#free-mode) any person
or agent may publish a seat declaration as itself, so an agent that can publish can write this entry into a
declaration, including its own. The control is review of the declarations before they are published,
not an st check. The "not authorized" list is advice to a model and was not tested adversarially. The real boundary is
branch protection on the repository the seat pushes to (someone should decide whether to enable a ruleset on a repository before a seat that works there opts in): a seat whose `origin` has none can push any branch
it creates there, and mission steps can carry text from untrusted places (issue and pull request
snapshots), which this permission turns into authorization for those actions only. Because Codex refuses
permission config on a remote resume, the entry is held by the app server and left off a resumed TUI.

The live eval `evals/st3/codex-minimal-access` and `scripts/st3-codex-minimal-eval/resume-check` (a
restart between two turns, checking the thread id, the daemon socket, the network and the refused
write) are paid and run by hand; no CI gate proves the real launch path. The argv tests and the
sandbox tests are the CI evidence.

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
