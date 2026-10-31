# Examples

Use [`client-tui/`](client-tui/) for a small Rust client that lists agents, follows a conversation
and sends messages through the public client/feed/model packages. Its
[guide](../docs/clients/build-your-own.md) explains the Rust and TypeScript reuse surfaces.

Use [`t3code-acp-bridge/`](t3code-acp-bridge/) for the bounded ACP stdio bridge spike that lets
t3code's Local ACP command drive daemon-managed Codex agents. Its README documents setup,
exclusive-writer constraints, mapping tests, and unproven end-to-end behavior.

Use [`fractal/`](fractal/) for Fractal seats, missions, and current Fractal patterns. Its
[README](fractal/README.md) indexes the examples by task. The test suite parses and previews every
tracked Fractal KDL example.

Use [`native/`](native/) only for legacy st2 agent declarations. Read the
[st2 legacy guide](../README.st2.md) before publishing them.
