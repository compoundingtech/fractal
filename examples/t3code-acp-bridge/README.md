# t3code ↔ Fractal ACP bridge spike

An opt-in, out-of-tree ACP v1 stdio agent for [t3code's Local ACP command](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-acp.md). It drives the Fractal daemon's managed Codex agents through the existing generated `@fractal/client`. It never launches Codex, another harness, or another daemon. No core package/crate API is changed. Refs [#4101](https://github.com/compoundingtech/fractal/issues/4101).

## Register in t3code

1. On the **machine running the t3code server**, use Node 24.18 or newer and prepare this repository's dependencies:

   ```sh
   mr apply --worktree-mode commit --only effect-utils
   nix develop .#web --command pnpm install --frozen-lockfile
   ```

2. Have a running, configured Fractal daemon with the Codex managed-agent harness and an HTTP client-v0 endpoint. The daemon, not this bridge, owns model authentication, workspace access, launch policy, and sessions. Use a dedicated agent for this bridge; do not send concurrent messages through another UI or the CLI.
3. In **Settings → Providers → Add provider → Show more → Local ACP command**, set:
   - executable: the absolute path to `node` on the server;
   - one literal argument: `/absolute/path/to/fractal/examples/t3code-acp-bridge/stdio.ts`;
   - display name: `Fractal ACP spike`.
4. Add environment overrides:
   - `FRACTAL_ACP_URL`: the daemon's HTTP base URL;
   - `FRACTAL_ACP_TOKEN`: bearer credential if the endpoint requires one;
   - optionally `FRACTAL_ACP_HOST`, `FRACTAL_ACP_MODEL`, `FRACTAL_ACP_EFFORT` to select the managed agent's host/model/reasoning effort.
5. Select this provider for a project whose absolute workspace path exists on the daemon's agent host. Keep the server and daemon on the same workspace host for this experiment. Use t3code's **Supervised** permission mode when observing approval dialogs; t3code may otherwise answer permissions automatically.

The executable is launched directly, **not through a shell**. Do not enter shell expressions, an `export` prefix, or a combined argument string. Dependency installation is a separate setup step; the stdio command must not print startup banners. stdout is exclusively newline-delimited JSON-RPC; failures go to stderr. The example uses the repository's existing dependencies and adds no dependencies or lockfile entries. Its private package manifest only selects ESM parsing.

## Supported surface and boundaries

- `initialize` accepts t3code's v2 offer and selects **v1**, using the v1 response shape. A repeated initialization before session operations is allowed. It does not advertise v2 preview support.
- `session/new` creates one Fractal-managed agent. The returned identity binds `session/load` to that same daemon agent across bridge restarts; there is no bridge-local session database and no harness adoption by cwd/PID guessing.
- `session/load` validates the existing managed agent and workspace and replays conversation history before responding. The session must be idle before prompting; loading an actively running agent is not an attach-to-active-turn feature.
- `session/prompt` accepts text and resource links; links become explicit textual context, not an assertion that their contents were fetched. Live assistant and tool updates become ACP `session/update` notifications.
- Permission requests use ACP `session/request_permission`, with allow-once/reject-once choices. Unknown options, malformed responses, client errors, and cancellation never authorize a tool. The answer is submitted to the daemon's existing approval surface, not to a second harness.
- `session/cancel` invokes the daemon's actual interrupt action. It is a notification, not a second prompt. Closing stdio detaches the observer without stopping/retiring the daemon agent.
- One in-flight prompt per session; no steering, fork, rollback, model/mode changes, MCP attachment, extra workspace roots, images/audio, client filesystem operations, terminal execution, authentication UI, native session listing/import UI, or subagent lineage is advertised. Existing daemon authentication must already work.

### Turn completion is intentionally Codex-specific

The typed Fractal conversation feed has entry finalization, **not a correlated turn-completed event**. The bridge must not treat an assistant message ending, an idle card, or a quiet socket as `end_turn`. This spike reads retained native Codex `event_msg` records, pairs a new `task_started` with `task_complete` using the exact native `turn_id`, and rejects competing starts. Other harness normalizers are not supported.

This requires an initially idle, **exclusive-writer** agent. The public client API does not expose the managed harness's message-to-accepted-turn ledger. Consequently, correlation against an external writer racing the bridge is not proven; this is not a multi-client-safe product adapter. An interrupted/disconnected feed fails the prompt rather than silently retrying an ambiguously delivered message or claiming completion. Reconnect by loading the same identity once its managed agent is idle.

Output granularity is the daemon's normalized native transcript, not provider token streaming.
The example maps inline text and tool bodies. Rich/image message blocks, owner continuations,
and explicit truncation/redaction markers are rejected rather than silently omitted. Creation
requires `collection.agents.exact-selector` and waits for an actual running/current/idle agent
card through that subscription before returning.

## Checks

From the repository root after dependency setup:

```sh
pnpm --dir packages/fractal-sdk exec vitest run --config ../../examples/t3code-acp-bridge/vitest.config.ts
pnpm --dir packages/fractal-sdk exec tsc --project ../../examples/t3code-acp-bridge/tsconfig.json
```

Protocol tests drive a fake ACP client, including v2→v1 negotiation, assistant/tool streaming, permission replies, cancellation while approval is pending, malformed parameters, unsupported operations, and reconnect/load identity. Backend tests exercise the real generated client with a controlled HTTP/socket transport, following the SDK's existing transport integration-test convention. This is **not** proof of an actual daemon/harness run or t3code UI interoperability. Fractal's existing isolated daemon fixtures are private Rust test helpers; there is no TS-callable local daemon fixture process to reuse here.

A subprocess test also runs the actual Node entrypoint, checks JSON-RPC-only stdout, v2→v1
initialization, malformed-JSON errors, unsupported-method errors, and clean EOF exit. It does
not start or connect to a harness.

## Open questions and recommendation

- Can the client-v0 API expose accepted message → native turn identity and durable turn-completion events, so ACP completion is harness-independent and safe with concurrent writers?
- Can an ACP session bind durably to a particular native conversation incarnation when a Fractal agent is restarted or reconfigured?
- How should replay handle daemon history retention, truncated content, unsupported native blocks, and stale/disconnected feeds without implying a complete transcript?
- How should Fractal-host workspace paths and credentials be selected when t3code's server is on another host?
- What should an actual t3code supervised approval/cancel/reconnect UI run demonstrate before treating this as a usable integration?

**Recommendation:** continue with ACP as the bounded integration path, not a native `ProviderAdapterV2` fork yet. Local-command registration is a real extension point and preserves daemon ownership. Require a real t3code + daemon supervised round trip and a stable daemon turn contract before making a product recommendation. Fake-client/transport tests establish mapping behavior only; they do not answer those end-to-end questions.

Protocol references: [v1 initialization](https://agentclientprotocol.com/protocol/v1/initialization), [v2 negotiation](https://agentclientprotocol.com/protocol/v2/initialization), [session setup](https://agentclientprotocol.com/protocol/v1/session-setup), [tool calls and permissions](https://agentclientprotocol.com/protocol/v1/tool-calls), [t3code provider guide](https://github.com/pingdotgg/t3code/blob/main/docs/internals/adding-a-provider.md).
