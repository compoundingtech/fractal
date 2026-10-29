# Runtime readiness recovery

This model-free eval starts an isolated Fractal daemon with a fake Codex process.

It proves that stale readiness cannot cross a runtime incarnation. It also proves the 60-second
attention deadline, same-incarnation recovery, and durable ready-work messages.
