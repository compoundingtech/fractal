# Binding an agent to a person's terminal

A native driver can run as a child of a shell in an existing Fractal terminal. Declare its seat with `bind-terminal` before starting the driver:

```kdl
version 2
agent "example/terminal-claude" {
  name "Terminal/claude"
  host "orchid"
  workspace "/work"
  harness "claude" {}
  bind-terminal "pty/person/avery/019a0000-0000-7000-8000-000000000001" incarnation="42:created" id="019a0000-0000-7000-8000-000000000002"
}
```

The terminal must already be declared on the same host. Publish with the terminal owner's person credential; agent and daemon credentials cannot create a binding. `incarnation` is the running terminal's PTY incarnation, including its PID and creation timestamp. `id` is a fresh UUID for each harness invocation. For a terminal with an explicit runtime ID, supply that exact ID with the optional `runtime-id` property.

This declaration records a harness configuration but never launches it. The caller runs `fractal driver claude --subject agent/example/terminal-claude -- <provider argv>` as a child of its shell with its chosen permission settings, driver environment, channel and hook registrations. The launcher owns argument merging and unmanaged fallback. Declaring a binding alone does not arrange those registrations.

Person-owned terminal bindings retain native harness execution; they do not select the managed agent-seat wrapper. The driver reads the declaration through its authenticated client before selecting this path. Agent-owned bindings and agents without a binding use the managed launch contract for OMP, Claude, and Codex.

The driver keeps the terminal's `st3.subject` tag. It reads the binding and matches that terminal's tag and exact PTY incarnation in the local registry. Its agent incarnation appends `:bound:<invocation UUID>` to the PTY incarnation. Mail and driver exit reports use that invocation fence, so a prior harness in the same shell cannot end the next one.

Reconciliation adopts the running PTY without a start. Once the driver records an exit for the current invocation, the seat stays exited across daemon restarts while the terminal's shell remains running. A missing or replacement PTY ends the binding without a launch. Stopping a bound seat detaches its graph runtime; it does not stop the harness process or shell. The person can exit the harness from its terminal. Bound seats cannot declare checkouts, rendering, terminals or exec sidecars, rollout, fresh-context recovery or one-shot retirement and cannot be materialized inside mission runs.

A driver that is killed before it records its exit leaves the seat's state unknown: the shell's
liveness alone cannot show whether its harness still runs.

No database migration or full replay is required. Existing declarations serialize identically. New bindings persist a `terminal-bound` member lifecycle. Older binaries cannot interpret that lifecycle; upgrade the terminal-owning daemon before creating bindings. Rollback is supported before bindings exist. Once bindings exist, roll forward: an older reconciler's stop fallback is not aware that the runtime belongs to a surviving shell.

## Native OMP attach admission

The typed `fractal_runtime::native_attach` proof APIs and
`fractal_store::store::native_attach` admission APIs support a separate,
externally owned incarnation. They are intentionally unwired: the in-process
extension transport and CLI are a later delivery slice, not an alternate client
approval route.

Preparation requires kernel process generation, boot and UID evidence, the full
native session ID, a canonical owned transcript with matching device/inode and
an actual open descriptor, and a one-shot, expiring nonce report from that exact
process. Keep the returned witness alive through commit. Nonces must never enter
argv, environment variables, transcripts, logs or work-item text.

`NativeAttachStore::ask_native_attach` creates the native person ask. Its question
names the host, PID/start, full native ID, transcript, agent and workspace.
Only an explicit accepting answer from the assigned person's trusted, unrevoked,
unexpired paired device with `control.work` produces the operation-backed receipt
required by commit. Local answers, delegation, assistant answers and deadline
defaults are not attachment authority. Commit binds the answer claim ID and
candidate digest once, atomically reserves the agent/process/session, and retains
audit predecessors. Identical retries recover the existing result.

The resulting incarnation is `external` and `channel-pending`; it declares no
launch and grants no restart, kill or startup-message authority. The preparation's
absolute expiry remains authoritative even if its ask remains visible longer.
Restart consent is separate. Before connecting dispatch, the transport slice must
detach on an in-process session switch or changed native ID and require a fresh
person approval; ordinary automatic session rebinding is not permitted for these
incarnations.
