---
title: "GitHub integration"
description: "Turn repository events into finite, evidence-backed agent work."
order: 9
---

# GitHub integration

Fractal can observe repository state and route new pull requests or issues into finite review and triage missions. The durable agent serves many runs; the observer and subscriptions decide when work starts.

## Authenticate the daemon

Install [GitHub CLI](https://cli.github.com/) and sign in as the OS user that runs the daemon:

```sh
gh auth login
gh auth status
fractal doctor
```

Do not assume a login from another account or interactive shell is visible to the daemon. For an explicit credential file, configure:

```toml
[github]
token_file = "/path/to/github-token"
```

The daemon account must own the file. Use mode `0600`, trusted parent directories, and a real file rather than a final-component symlink. An unsafe or unreadable configured token file fails authentication instead of silently falling back. Never put its contents in declarations or Git.

## Design intake before enabling it

1. Declare a worker with the repository access it needs.
2. Publish finite review and triage missions with a source resource input.
3. Give each run an explicit goal and evidence contract. Review is not permission to publish, approve, label, close, or merge.
4. Declare a `github.repository` observer for the repository fields you need.
5. Subscribe to matching events with mission delivery, passing the discovered resource to the child mission.
6. Keep the intake run alive with an explicit retirement gate, then start it.

Publish child missions before the intake that references them. An empty mission finishes immediately and does not keep subscriptions alive. A worker processes one claimed step at a time even if several events create runs.

Start with observation and review-only work. Add write authority only as a deliberate, separately reviewed workflow decision.

## Inspect and stop

Use `fractal missions ls`, `fractal work ls`, and `fractal agents queue agent/garden/worker` to distinguish observed events from child work actually running. Read step results and evidence rather than relying on a chat summary.

When the intake trial ends, cancel its run explicitly with `fractal missions cancel RUN --reason TEXT --as person/ada`, or answer its planned retirement gate. Do not remove a local file and assume the published observer stopped.

See [resource subscriptions](https://github.com/compoundingtech/fractal/blob/main/docs/st3/resource-subscriptions.md) for complete observer and subscription declarations, and [Missions in practice](missions-in-practice.md) for finite work and review.
