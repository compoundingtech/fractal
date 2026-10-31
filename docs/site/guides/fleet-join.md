---
title: "Fleet membership"
description: "Understand invitations, member trust, and safe removal."
order: 6
---

# Fleet membership

A fleet is a set of trusted members that replicate one claim history. For a runnable setup, follow [Connect two machines](two-machines.md).

## Trust and identities

Each member has a private signing key. A node name and key form an incarnation; a wiped machine that rejoins under the same name is a new incarnation. The anchor is the founding member whose key is pinned by the others.

Members are trusted to write claims and run agents with shell access. A shared fleet secret authenticates traffic; member signatures bind a writer's envelopes to its incarnation. Tailscale or Fabric supplies transport encryption. Never put keys, fleet secrets, or invitation codes in Git, logs, or support reports.

## Invitations

```sh
fractal fleet invite beacon --via tailscale
fractal fleet invites
fractal fleet join --transports tailscale
```

Run join on the new machine and enter the code at its no-echo prompt. An invitation expires and admits one member identity. Recovery of a lost response can reuse that same identity until expiry; it does not make the code reusable by a different machine. If the code expires after admission but before recovery, remove the stranded member and issue another invitation.

The sponsor must have a reachable listener. A dial-out-only member cannot sponsor a new joiner. Prefer a prompt or protected code file to passing secrets in command-line arguments.

## Verify synchronization

```sh
fractal fleet status --json
fractal fleet wait
fractal replication status
```

Membership alone does not prove catch-up has finished. Wait for the initial sync and inspect the state required by your workflow.

## Leaving and removal

`fractal fleet leave` takes the local machine out after its writes reach another member. `fractal fleet remove NAME` removes another member; consult `--help` for required reasons and options.

Removal is not history erasure, secret rotation, or a command to stop all declared agents. A partitioned member may not yet know a removal happened. Preserve the graph and keys when investigating membership disputes.

See [replication](https://github.com/compoundingtech/fractal/blob/main/docs/st3/replication.md) and [checkpoints](https://github.com/compoundingtech/fractal/blob/main/docs/st3/checkpoints.md) for the engineering guarantees.
