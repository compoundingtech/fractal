---
title: "Client API"
description: "Use the supported client contract rather than the privileged daemon surface."
order: 5
---

# Client API

Fractal's web and iOS clients read work and submit authorized actions through the client contract. A custom client should use that contract rather than treating the daemon's privileged internal API as a public network service.

## Start here

- [Build your own client](https://github.com/compoundingtech/fractal/blob/main/docs/clients/build-your-own.md): practical client development.
- [Client contract and gateway](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/README.md): pairing, scope, transport, and API behavior.
- [UI contract](https://github.com/compoundingtech/fractal/blob/main/docs/clients/ui-contract.md): shared presentation and action semantics.
- [Client collections](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/collections.md): collection synchronization.
- [Push contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/push.md): live updates.

## Security boundary

Expose the paired-only gateway through HTTPS or an encrypted tailnet route. Keep the privileged daemon socket private. Pair each trusted device with the scopes it needs, protect its saved credentials, and revoke devices you no longer trust.

A phone or browser client does not replicate the graph or run agents. It needs a live connection to make changes. Show offline or stale state honestly; do not render a cached observation as proof of current success.

Use server-provided subject IDs, current action availability, and revision tokens. A user opening a card is not approval; send an explicit authorized action and handle the returned result.

For user-facing connection guidance, see [Web and iOS clients](../guides/web-and-ios.md).
