---
title: "Connect two machines"
description: "Join your own machines into a fleet and verify graph replication."
order: 5
---

# Connect two machines

A fleet connects your own machines. Each daemon keeps a local graph and catches up after reconnecting. Each person currently runs their own fleet.

Finish [installation](../getting-started/install.md) on both machines using the same person identity and distinct machine names. These examples call them **studio** and **beacon**. Install harnesses only where agents will run.

## Using Tailscale

Connect both machines to your tailnet and allow TCP port 31313 between them. On studio, create the fleet once:

```sh
fractal fleet create --name studio --port 31313 --transports tailscale
fractal fleet status --json
fractal service status
fractal fleet invite beacon --via tailscale
```

If studio already belongs to a fleet, skip create and use its existing status. On beacon:

```sh
fractal fleet join --transports tailscale
fractal fleet status --json
fractal replication status
fractal fleet wait
```

Paste the single-use code at the private prompt. Join refreshes installed services and waits for initial synchronization. Add `--dial-out` if beacon should accept no inbound replication; outgoing connections still synchronize both directions.

Success means both machines show the expected membership and initial synchronization finishes. Check an existing mission on the second machine to confirm the state you need is available. The graph does **not** copy repository files: prepare the workspace separately on any machine that runs work.

## Using Fabric

With Fabric running and the machines already trusted, substitute `--transports fabric` at create and join and `--via fabric` when inviting. Grant the exact advertised fleet protocol to the other machine in Fabric before joining. Fractal manages the local tunnel; do not add a second manual peer configuration for the same route.

See [Fleet membership](fleet-join.md) for trust and invitation behavior and [replication](https://github.com/compoundingtech/fractal/blob/main/docs/st3/replication.md) for digests and recovery. Keep raw state and keys when investigating inconsistent history; do not reset a replica merely to make its status look healthy.
