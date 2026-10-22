# Connect two machines

Each daemon keeps a local replica of the graph and catches up when it reconnects. This connects **your own machines**; each person currently runs their own fleet.

Finish [getting started](getting-started.md) on the first machine. Install Fractal, set `person = "person/ada"`, and start its user service on the second machine too. Harnesses and their logins are needed only on machines that will run seats.

The examples call the first machine **studio** and the second **beacon**. Choose one route below. Create your first fleet on studio only once; if it already belongs to your fleet, skip `create` and use the endpoints shown by `st fleet status --json`. Creating a fleet preserves your existing graph. To try the worker and mission examples below, first follow [missions in practice](missions-in-practice.md#declare-a-worker-seat) on studio.

## Tailscale

Install and connect Tailscale on both machines. Allow TCP port 31313 between them in your tailnet policy. On **studio**:

```sh
tailscale ip -4
st fleet create --name studio --port 31313 --transports tailscale
st fleet status --json
st service status
st fleet invite beacon --via tailscale
```

The replication worker listens on `127.0.0.1:31313` and studio's detected local Tailscale addresses on port 31313. Status shows the advertised tailnet endpoints. The invitation carries those literal IP addresses, so beacon does not need a manual peer URL or MagicDNS name. Tailscale userspace networking without a local tailnet interface cannot provide this listener; see [Tailscale setup](st3/tailscale.md).

On **beacon**, join and paste the code when asked:

```sh
st fleet join --transports tailscale
st fleet status --json
st replication status
```

Join restarts the installed services and waits for the first sync. Beacon also listens on loopback and its own Tailscale addresses, using port 31313 or the next free port. Add `--dial-out` to `join` if beacon should accept no inbound replication connections; its outgoing connections still synchronize both directions.

## Fabric

Use this route when the machines are already trusted Fabric peers. Run Fabric as a service on both, and allow the fleet's protocol in the peers' Fabric service grants. See [fleet join](fleet-join.md#fabric) for the exact exposure, dial, and service-grant configuration.

On **studio**:

```sh
st fleet create --name studio --port 31313 --transports fabric
st fleet status --json
st service status
```

The replication worker listens on `127.0.0.1:31313` and exposes that listener through Fabric. Status shows the `fabric://NODE_ID/PROTOCOL` endpoint: the node ID comes from studio's Fabric identity, and the protocol belongs to this fleet. Grant that exact protocol to the other machine in Fabric before joining.

On **studio**, issue an invitation containing that Fabric endpoint:

```sh
st fleet invite beacon --via fabric
```

On **beacon**, paste it at the join prompt:

```sh
st fleet join --transports fabric
st fleet status --json
st replication status
```

Beacon listens on its own loopback port and exposes it through Fabric. The invitation sets the sponsor's peer address; the replication worker creates and reacquires the local Fabric tunnel. No manual `fabric dial` or `[[peers]]` entry is needed. Add `--dial-out` to accept no inbound connections on beacon.

With their user services installed, Tailscale, Fabric, and the Fractal replication worker restart after reboot and reconnect after network interruptions. A terminal does not need to stay open.

### Reconcile Fabric runtime policy

`st-fabric-adapter` is a separate process, not part of the st daemon. Start Fabric
with `--runtime-peers`, then run the adapter on each machine:

```sh
st-fabric-adapter --once
st-fabric-adapter --status
```

Without `--once`, it polls every five seconds and prints a JSON status after each
attempt. `--st-socket`, `--fabric-socket`, and `--state-dir` select isolated daemons
and adapter state; the defaults are `$XDG_RUNTIME_DIR/st3.sock`,
`$FABRIC_HOME/run/control.sock` (or `~/.local/share/fabric/run/control.sock`), and
`~/.local/state/st-fabric-adapter`. Keep that state directory across restarts and
use only one adapter per state directory.

The adapter reads the local [membership and leave-state contract](st3/replication.md).
It grants exactly `st3/fleet/<fleet-id>` (the replication carrier) and `st3-client-v0`
(client API access) to verified current Fabric members under owner `st/fleet/<fleet-id>`.
It never grants the terminal protocol `st3/pty/<fleet-id>`: remote terminal access over
Fabric stays an explicit operator grant in static policy, which the adapter does not
edit. A receiver
retains only a leaving member's replication carrier until the persisted 120-second
receipt deadline, never terminal or client grants. Confirmed, removed, or expired
remote members lose this owner's records. The leaving machine's own final carrier
remains CLI-owned until receipt completion and settings removal (or explicit
offline leave). Re-keying withdraws the old NodeID and grants the new NodeID in one
revision-checked batch.

`applied` means this owner's policy matches membership; `leaving` reports retained
carriers; `leave_unconfirmed` reports missing confirmation after carrier expiry.
`runtime_peers_disabled` requires restarting Fabric with `--runtime-peers`.
`residual_access` identifies static or other-owner grants that the adapter cannot
revoke: remove those at their source before treating a departure as fully revoked.
Read failures leave policy untouched; inspect the JSON `message` and retry.
`--status` reads the last attempt, so inspect `observed_unix_ms` for freshness.

The adapter does not bootstrap Fabric pairing or replace the initial trust/grants
required for joining above. Service-manager units are configured separately.

## SSH as a last resort

An SSH tunnel can carry the same protocol between loopback listeners. On studio, create the fleet with `st fleet create --name studio --port 31313 --advertise-loopback`, then issue `st fleet invite beacon --via loopback`. On beacon, keep a separate terminal running:

```sh
printf 'SSH destination for studio (for example ada@studio): '
read -r studio_ssh
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:31313:127.0.0.1:31313 "$studio_ssh"
```

Join on beacon with `st fleet join --dial-out --via http://127.0.0.1:31313` and paste the invitation. Studio's worker listens on `127.0.0.1:31313`; SSH provides that same address locally on beacon, which has no inbound replication listener in dial-out mode. This foreground tunnel dies when its terminal closes, its network connection fails, or the machine reboots. You must reopen it to resume replication. Prefer the service-managed Tailscale or Fabric routes above for ongoing use.

## Check what arrived

On **both machines**:

```sh
st agents ls
st missions ls --all
st replication status
st replication status --json
st doctor
```

Ordinary `st agents ls` and `st agents tree` read the latest published roster without
requesting a refresh. Immediately after starting or changing an agent they can show the
earlier roster; after daemon startup the roster may still be being prepared. When you need
to see your own write, pass its same-host receipt `store_index` as `min_cut` to the agents
client read and check the served snapshot index. This is not a replicated or cross-host cut.

The worker and missions declared on studio should appear on both machines. `replication status` reports the last successful exchange, backlog, authority digest, graph digest, and table digests. After both members have caught up and writes settle, matching authority digests mean they hold the same replicated history. With matching builds and no waiting claims, matching graph and table digests also confirm the same shared projections. During an upgrade, a reader may hold claims that need a newer build; update it before comparing projections.

If doctor reports invalid claim-signature warnings, retain the report and follow the [operator guidance](st3/founder-signing-audit.md); matching replication digests alone do not establish valid claim signatures.

To show that a write travels back from beacon, send a message **on beacon**:

```sh
st conversations send agent/garden/worker --from person/ada \
  --subject 'Replication check' --body 'Hello from beacon.'
```

Read it **on studio** after the next exchange:

```sh
st conversations ls agent/garden/worker
```

| Replicates | Stays on the owning machine |
| --- | --- |
| Declarations, missions, runs, step results, messages, reviews, and referenced graph documents/blobs | Checkouts and ordinary workspace files |
| Membership and durable agent state | Harness installations, credentials, private keys, configuration, and live terminal processes |

The graph records a seat's host. Replication does not copy its workspace or move its running harness. Clone a repository and log in locally before declaring a seat on another machine; start a mission run on the machine where its workspace exists.

## A sleeping machine is normal

Put beacon to sleep or disconnect it from the network. On studio, inspect the fleet again after about 90 seconds:

```sh
st fleet status --json
st replication status
st doctor
```

Beacon becomes `last-seen`, with the time of the last exchange. Its absence does not remove it or make a healthy graph an error; transport diagnostics can retain the last failed connection. Authentication or admission failures are different and need investigation. Wake beacon or restore its connection and it catches up automatically, with no new invitation. Retries back off, so reconnection can take a little time.

See [replication](st3/replication.md) for digest interpretation and recovery, and [fleet join](fleet-join.md) for Tailscale/Fabric routes, invitations, leaving, and removal.
