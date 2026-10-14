# Synthetic seat container proof: fixed source recipe

Status: **Draft source only; all controls UNRUN.** This is a proposed evidence recipe for
[#2839](https://github.com/compoundingtech/smalltalk/issues/2839), not a deployed adapter,
admitted executor, qualified image, or runtime authorization. The machine-readable fixed inputs
are in [synthetic-container-proof-recipe.json](synthetic-container-proof-recipe.json). A missing
provider or observation yields **incomplete**, never a synthetic pass. The result covers one
invented seat and one reviewed operator-owned deployment binding. It does not establish a
host-root, live-memory, global-forensics, real-seat, or production guarantee.

## Admission and fixed boundary

An operator first reviews and pins a single image digest, fixed unit and root trigger, native
OMP/PTY/daemon session provider, generation and handover provider, UID and GID maps, owned host
persistence inventory, external egress observer, executor slot and quota provider, cleanup
provider, and receipt signer. The request cannot choose an executable, arguments, paths, image,
network endpoint, unit, UID map, or seat. No real seat, person, key, daemon, provider state, or
production host participates. The fixture's invented seat and handover text are fixed in the
recipe. The volatile canary is generated **inside the guest** after the actual native session
starts; its random suffix does not enter the receipt or bridge.

The shared candidate class is normal ephemeral Namespace8x16, priority 3. It requires three
**serial, separately reviewed jobs** with fresh owned state: F1 (hard 256 MiB), Slice0 pair
(hard 2 GiB), and this synthetic-seat job (hard 2 GiB). These are hard **aggregate owned**
limits, including tmpfs, engine metadata, images, all writable guest state, and logs. The
quota is enforced before a child starts; accounting only for guest memory or only for the
container root does not qualify. A real slot, runner, aggregate quota, memory/core controls,
and teardown attestation are still unbound. The job refuses admission until those values are
reviewed and pinned. No overlap or state reuse among the three jobs is allowed.

Before any child, record the reviewed binding and its immutable digest, exact UID **and** GID
maps, unit identity, owned mount and persistence inventory, cgroup version/path, finite memory
and task limits, hard aggregate quota, `memory.swap.max=0`, `memory.swap.current=0`, zswap state
where supported, core `RLIMIT_CORE=0`, and coredump storage disabled. Refuse if the host cannot
attest these controls. The UID/GID host ranges are intentionally unfilled in the source recipe:
Operations must allocate and pin an invented, nonoverlapping map. The bridge app and broker IDs
must map to distinct expected host IDs, never overflow UID 65534. No host allocation is inferred
from a personal or production seat design.

## Ordered evidence

The operator captures a bounded technical receipt with each observation's origin, fixed binding,
attempt, result, and limitation. An assertion made only by guest code cannot prove host cleanup
or external egress denial. The run has these ordered phases:

1. Verify admission and fixed bindings, then intentionally present a wrong UID or GID map to
   the operator-owned launcher. It must refuse **before a child** exists. Restore the exact
   pinned map and start a fresh invented seat.
2. Observe the real private daemon, OMP native session, and PTY binding through a reviewed native
   provider. Record opaque session/incarnation IDs and the generation. A `NativeReady` frame,
   active unit, open socket, or generated test string alone is insufficient.
3. Generate a volatile turn canary inside that guest session; hold its exact bytes only in
   bounded volatile test memory. Commit the separate invented handover at the fixed private
   persistent location with atomic replacement, file and directory sync, and a
   generation-bound marker. Prove the handover read or write by provider evidence without
   putting document bytes in the bridge or receipt.
4. Attempt each denial in the table below independently. Fence new work, observe zero in-flight
   writes **after** the fence, suspend, prove the whole cgroup empty, then unmount/destroy the
   generation's volatile storage. A failed fence, commit, stop, or erasure is failure/unknown,
   not suspended.
5. Scan the **enumerated owned host persistence paths** for the exact canary with an external
   observer. Record the scan inventory, exclusions, path identities, counts, and result; keep
   the canary bytes out of the receipt. Check that the owned volatile tmpfs is gone/empty using
   external mount and usage evidence. Private handover is an explicit content exception and
   must not contain the volatile canary. This does not assert absence from unowned backups,
   host-root-accessible memory, or every host file.
6. Resume through the fixed trigger with a larger generation and **new actual native** OMP and
   PTY session IDs. The reviewed provider must show the new session read the committed handover
   first, before application readiness or work. Do not restore the old transcript. Present a
   changed UID or GID map in a separate operator-owned negative attempt and prove refusal
   before a child; it must not become a second resumed seat.
7. Stop; prove cgroup empty and owned storage cleanup again. If cleanup fails, mark the whole
   result fail and retain the named residual for operator recovery. Publish technical evidence
   separately from the plain-words result.

| Control | Attempt and independent evidence required |
| --- | --- |
| Fresh native session | Native provider ties private daemon, OMP, PTY, unit, generation, and incarnation together before ready, then shows new IDs after resume. Missing native provider is **incomplete**. |
| Generation and first handover read | Durable generation increases; provider observes generation-bound committed handover read before readiness. No handover bytes in receipt. |
| Volatile canary absent from owned host persistence | Guest generates the canary; external scanner enumerates pinned owned persistence paths after suspend and finds zero exact matches. Include scan scope and exclusions. |
| Owned tmpfs empty and cgroup empty | External unit/cgroup and mount evidence after suspend and stop; no remaining tasks, mount, or owned volatile bytes. |
| Host decoy files inaccessible | Operator creates an invented decoy outside guest mounts, proves it exists on host, then guest open and directory lookup are refused; mount inventory confirms it was not exposed. |
| Swap, zswap, core and coredump disabled | Host cgroup readings before/after, guest core limit, host coredump configuration and owned-path inspection; unsupported zswap state is stated, not silently assumed. |
| Socket refused | Guest attempt to open the host controller listener fails; mount/socket inventories show it was not present. |
| Wrong peer UID refused | Real Unix connection with wrong pinned UID fails at `SO_PEERCRED` check before a frame is accepted; server-side decision evidence. |
| Wrong seat refused | Correctly credentialed test connection sends a different invented seat in the typed frame; server rejects it. |
| Unexposed API refused | Guest attempts a fixed host API operation outside the lifecycle/status contract; bridge rejects it, with no raw host API fallback. |
| Filesystem refused | Guest attempts fixed host path read/write, including the decoy; kernel error and host-side unchanged-file evidence. |
| Network IPv4 and IPv6 refused | Guest attempts fixed IPv4 and IPv6 canary destinations; separate external packet counters/sink observation show zero delivery. Guest DNS failure or timeout alone is only an interlock. |
| Wrong UID/GID map refused | Operator supplies an intentionally wrong map; launcher rejects before child or socket creation; observe zero new child tasks. |
| Changed UID/GID map refused | After first generation, attempt a changed map; refuse before child and keep the previously pinned map for the real resume. |
| Hard aggregate quota and cleanup attested | Operator proves all owned bytes count under the fixed hard limit before launch and after teardown, including engine/image/log state; provider signs cleanup result. |

Socket absence, peer-credential rejection, wrong-seat rejection, API rejection, filesystem
denial, and IPv4/IPv6 network denial are distinct observations. A successful one never stands
in for another. Deliberate negative attempts use only invented identities and operator-owned
fixtures. Every attempt must be bounded by the reviewed runner, with no unbounded retries.

## Receipt and decision

The technical receipt has the fixed recipe/binding digest, image digest, operator signer,
executor admission and aggregate quota evidence, invented seat identifier, opaque generation
and native session IDs, exact UID/GID-map observations, owned path inventory, per-control
attempt/result/evidence-origin/limitation, cgroup/mount/swap/core observations, external
network observation, and cleanup outcome. It contains **no** turn canary, handover bytes,
transcript, key, token, environment dump, command output, or arbitrary guest text. Limit the
whole receipt to 64 KiB; reject missing, duplicate, unknown, or overlong control IDs. Source
checks or a self-reported guest result do not confer runtime proof. The signer must be bound
by Operations; an unsigned receipt is incomplete.

Each control is `pass`, `fail`, or `incomplete`. `pass` requires the named external or native
observation. Any failed security or cleanup control makes the overall result **fail**.
Missing binding, native provider, observer, quota, or attempted control makes it
**incomplete**. No result from this recipe can qualify a real seat by analogy.

The final public page uses plain words, at most 8 KiB, with one row per control: **what was
tried, what happened, pass/fail/incomplete, and what remains unproved**. It reports the actual
scope of the owned persistence scan and names missing native or external evidence. It has no
body hashes, secret contents, or raw command output. The separate technical receipt can carry
digests and opaque IDs for reviewers. A suggested opening is: “An invented seat was tested in
one reviewed container job. The result applies only to that job and its listed owned paths.”

## Required review before any run

Intake must review this exact source head. Operations must supply and review the actual
image/package, fixed root trigger and deployment adapter, native OMP/PTY/daemon provider,
handover provider, slot and aggregate quota, pre-child and post-stop attestation, and receipt
signer. Security must review filesystem, credentials, and an **out-of-band** IPv4/IPv6 egress
test. The executor owner must admit a real serial slot and hard owned-byte accounting. Only
then can an operator run a **synthetic** fixture. This document authorizes no container,
daemon, fixture, host probe, real seat, production key, or production Hetz/Hetz2 start.
