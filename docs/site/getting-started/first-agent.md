---
title: "Your first agent"
description: "Declare a durable worker with a harness and a workspace."
order: 4
---

# Your first agent

Finish [installation](install.md). These examples use the invented person `ada` and agent `garden/worker`; substitute your configured person. They use Claude Code, installed and fully signed in under the daemon's user account. Complete native welcome, trust, and login screens before proceeding.

An agent is independent of its missions. It can complete one run and take another without being redeclared.

## Declare the worker

```sh
mkdir -p "$HOME/fractal/garden"
cd "$HOME/fractal/garden"
cat > worker.kdl <<EOF
version 2
agent "garden/worker" {
  workspace "$PWD"
  restart "always"
  harness "claude" {
    model "sonnet"
  }
}
EOF
fractal apply worker.kdl --as person/ada --dry-run
fractal apply worker.kdl --as person/ada
fractal agents show agent/garden/worker
```

The preview must have no blockers. After publication the worker should become ready and idle; it takes no model turn until it receives a message or work. Keep the declaration in version control.

If the agent is waiting at a provider's native screen, attach from your own terminal:

```sh
fractal terminals attach agent/garden/worker
```

Finish the prompt, then detach with **Ctrl+\**. Do not treat provider authentication failure as a reason to repeatedly restart the daemon.

## Send a message

```sh
fractal conversations send agent/garden/worker --from person/ada \
  --subject 'Hello' --body 'Tell me which workspace you are in. Do not change files.'
```

Run `fractal web` to open the daemon's [local web app](../guides/web-and-ios.md) and inspect the reply, or use `fractal conversations ls agent/garden/worker` and the CLI's conversation-reading commands. Replies appear in the agent's conversation, not in a person mailbox. A successful send records durable conversation; it does not prove a task has been completed. Use a [mission](first-mission.md) for work with a result to verify.
