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

Run Fabric with `--runtime-peers` and install `st-fabric-adapter` on both machines. Keep the external adapter running as a separate service on the sponsor. Create the fleet with `--transports fabric`; the machines need no prior Fabric trust or manual grants.

On studio:

```sh
fractal fleet invite beacon
```

On beacon:

```sh
fractal fleet join CODE
```

Prefer the private prompt (`fractal fleet join`) rather than putting the credential in shell history. The code pins the sponsor's Fabric identity and carries an expiring, single-use pairing permit. Pending attempts grant no ordinary access. The adapter verifies signed fleet admission before granting exactly `st3/fleet/FLEET_ID` and `st3-client-v0` under owner `st/fleet/FLEET_ID`; the joiner authenticates the sponsor before reciprocal access.

Terminal access requires an explicit static operator grant for `st3/pty/FLEET_ID`, which the adapter never edits. Unconfirmed attempts expire after a Fabric restart; issue a fresh invitation rather than manually granting trust. Preserve adapter journals across restarts so uncertain confirmation can be recovered with the original code, name and local member key. Static or other-owner grants are not revoked by pairing or membership reconciliation.

### Connect a client over Fabric

Use the remote host's Fabric NodeID or its unique trusted Fabric peer name:

```sh
fractal --endpoint fabric://studio/st3-client-v0 agents ls
```

The client asks the local Fabric daemon to dial `st3-client-v0`; no TCP listener,
manual tunnel, or HTTP proxy is needed. Peer names match without regard to case.
Use a NodeID when names are ambiguous. Fabric must already trust the remote
machine and grant this protocol; the membership adapter manages the grant
for current fleet members.

For paired client reads and actions, supply the remote gateway's bearer credential
through `FRACTAL_CLIENT_CREDENTIAL`, preferably injected by your credential manager.
Do not put it in the endpoint URI or command arguments. The CLI does not silently
reuse an HTTP gateway's saved pairing profile for a different Fabric endpoint.

This transport exposes only `/v1/client/` routes and `/v1/health`. Native harness,
mailbox, internal, and administration routes still require their appropriate local
transport. Fabric routing does not bypass client API pairing, bearer credentials,
or device signing: the remote daemon applies the same client authorization rules.

See [Fleet membership](fleet-join.md) for trust and invitation behavior and [replication](https://github.com/compoundingtech/fractal/blob/main/docs/st3/replication.md) for digests and recovery. Keep raw state and keys when investigating inconsistent history; do not reset a replica merely to make its status look healthy.
