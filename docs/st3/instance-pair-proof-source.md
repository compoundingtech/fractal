# Slice 0 two-instance fixture: source review input

Status: **Draft source only; all controls UNRUN.** `scripts/st3-instance-pair-proof` is a
candidate fixed program for **one disposable hosted or development job** owned by the same
ordinary user. It runs two native `st --instance NAME` processes using private homes. It is
not a container, a hostile-code boundary, a credential boundary between processes of that
same UID, or a production replication proof. No job, daemon, invitation, document fixture,
native control, or build was run for this source review. Intake must review this exact script
hash and testing plan before Operations binds a command or any fixture starts.

Nathan's no-container direction supersedes closed [#2862](https://github.com/compoundingtech/smalltalk/pull/2862)
and [#2878](https://github.com/compoundingtech/smalltalk/pull/2878). Their source reviews
remain historical; the old guest bootstrap acceptance for SHA256 `05d5249b…` does **not**
authorize this successor. The October 16 container receipt target is withdrawn. There is no
Docker, podman, OCI, image, root trigger, container egress test, or root-session prerequisite.

## Fixed candidate and native behavior read from source

The candidate `st3` source is `21fd968d3c5534a1ebb33458e8ff25c6d4e2da90` and the
supplied release binary digest is
`eeae8c11562b4c048ce59c52dd0b75b543bb139e68caae274ad45da06d3f5938`.
These are inputs, not a qualified binary or CLI receipt. At that source pin,
`crates/st3/src/instance.rs` resolves each instance under its process's `HOME` as
`.st-instance/NAME`, setting separate state, config, data, cache, runtime sockets, harness
accounts, work roots, service names and default node names. The script adds **two different
private HOME directories**, `/var/tmp/st3-ip/a` and `/var/tmp/st3-ip/b`; it uses instance
names `proof-a` and `proof-b` and no service install. Its fixed peer listeners are
`127.0.0.1:31313` and `127.0.0.1:31314`. Its local API sockets remain in the respective
instance run directories. The job's reviewer must verify the actual bound addresses and
no inherited production state, sockets, credentials, HOME, config or service access.
The launcher starts every subprocess with a constructed environment dictionary from an
**empty parent map**, and reads `/proc/PID/environ` after each long-lived child starts to
require its exec-time environment exactly matches those five supplied keys. `st --instance`
then adds private XDG paths within the child. A same-UID process can still access another
same-UID process's files or socket; these paths do not defend against hostile guest code.

Candidate CLI source has `--instance`, `fleet create/join --no-service`,
`--advertise-loopback`, `fleet invite --via loopback`, and join `--via` with an HTTP loopback
URL. `MemberSettings.transports` accepts CLI strings and the replication worker enables
auxiliary Tailscale/Fabric only for those exact names; loopback routes are considered
separately. The script therefore proposes `--transports loopback` and checks the generated
fleet file and sponsor route **before either worker starts**. The CLI help lists only
Tailscale/Fabric transports; this source-derived spelling is **unqualified** until an
exact-head Intake read and native CLI check. If the CLI rejects it, writes a different fleet
file, or starts an auxiliary transport, the candidate has a concrete primitive gap: refuse
the run and request an explicit native loopback-only selector. No direct edit of the
generated fleet file, invented alternate flag, or fallback transport is authorized.

The proposed launcher accepts no caller arguments. It has a fixed candidate binary path
`/var/tmp/st3-ip-bin/st3`, fixed short job root `/var/tmp/st3-ip`, and a strict five-key
parent environment: `HOME=/var/tmp/st3-ip/launcher`, `PATH=/usr/bin:/bin`, `LANG=C`,
`TMPDIR=/var/tmp/st3-ip/tmp`, `PYTHONDONTWRITEBYTECODE=1`. It refuses root, changed EUID,
preexisting root, symlinked binary, wrong digest, or any added environment key. Each child
gets only a constructed environment with its own HOME and instance run directory. A
preflight bind checks both fixed loopback ports are free before creating any state; the
post-start child-owned listener check catches a port taken after preflight. A
fixed operator-owned job must pin the absolute launcher path, binary, script digest, user,
work directory, and lifetime; callers cannot choose argv, endpoint, content, instance name,
or package. This source does not install such a job.

## Proof flow and narrow cut

The script creates fresh private roots and a fresh invented `person/instance-proof-…`
identifier. Native fleet creation and join generate a new fleet ID and distinct member keys;
no production actor or key is borrowed. A local synthetic person configuration is still a
bootstrap decision requiring **new** exact-script review. The script writes exactly 50
fixed invented document bodies on `a`, starts the two native daemon and replication-worker
processes in the same job, joins `b` through the loopback invitation, and waits for its
first sync. The invitation is held only in private files and never printed.

The cached replication digest is a readiness observation only. Each member then captures
health/build/store index, one complete claim page capped below 500 claims and 8 MiB per
response, all 50 document rows and body bytes, binding claim IDs/body/owner/timestamps,
fleet ID, exactly two current member names and distinct public key fingerprints, and zero
admission residue. The script excludes physical arrival `store_index` and document
`created_index` from cross-member semantic equality but checks them locally. It compares
the full admitted-input hash and every document field at the same stable physical cut,
then recaptures `a` before success. A differing-input receipt lists at most 20 claim IDs
and kinds. The public receipt carries the fleet ID and SHA256 fingerprints of the
UTF-8 public member-key strings so Operations can compare them with production fleets;
it never carries native private keys, invitation codes, API bodies, child logs, or raw errors.
This scope is `exhaustive-fixture-documents-and-admitted-claims-v1`, **not** a global Source,
checkpoint, shared projection, Fabric, iOS, or live-fleet oracle.

After each native start, the launcher inspects each child's `/proc/PID/fd` socket inodes
against `/proc/net/tcp` and `/proc/net/tcp6`. It requires exactly one TCP listener for
that member, owned by its daemon child and bound to that member's fixed
`127.0.0.1` port; it refuses any other child TCP listener. At the final cut it requires
every **observed** child TCP remote to be loopback or unset, and writes both members'
listener/remote booleans and socket counts in the receipt. A missing proc read or a
disappearing child fails closed. This is a sampled, child-owned TCP observation; it cannot
prove that no short-lived connection occurred between reads, no UDP or other process used
the network, or that a same-UID process could not interfere. Operations still needs an
out-of-band egress and process/socket observation before qualifying the job.

The 8 MiB per response, 128 MiB aggregate local API wire budget, 2,048 request cap,
50-document set, 48 KiB final receipt cap, fixed two members, and bounded readiness and
child shutdown times limit the authored program. Operations must provide an external hard
aggregate owned-byte quota (candidate 2 GiB across package, both homes, all writable
state, logs and metadata), process count, whole-job time (candidate 1,800 seconds), and
cleanup attestation before any child. These values are **unbound**; source limits cannot
replace the runner's hard controls. A cleanup failure or unreaped child makes the receipt
fail. The job's separate cleanup must attest that the whole disposable job, including the
binary package outside the script-owned root, is gone.

## Controls carried and withdrawn from #2862

| Earlier control | This candidate |
| --- | --- |
| F2 exact environment allowlist | Carried: parent environment equality, constructed per-instance child environments, no host XDG or token forwarding. |
| F3 guest isolation interlocks | Container-specific mount, capabilities, no-new-privileges, private network interface and embedded DNS assertions **fall away**. Replaced only with private own-user roots and external disposable-job review, which cannot isolate same-UID processes from one another. |
| F5 fleet identity and two-key binding | Carried: new fleet ID, exactly two current members, distinct public key fingerprints, no production comparison inferred locally, all 50 documents and bounded differing claims. |
| F7 relay connections and byte budgets | Relays and their 64-connection/64 MiB counters **fall away**. Direct native loopback routes replace them; 2,048 API requests and 128 MiB local API bytes bound authored reads, while Operations must inspect actual peer sockets/bytes and external egress. |
| F8 receipt, refusal and cleanup controls | Carried where meaningful: secret-free closed errors, fixed build, exact roots, invitation privacy, bounded API and receipt, adverse cut cases, child reap and explicit cleanup failure. All source tests and native controls remain UNRUN. |
| Old image/rootfs/UID65534/read-only mount/cgroup/egress controls | Withdrawn. No container-specific security claim transfers to a same-UID process fixture. External job quota and egress review are new bindings, not a substitute for that old threat model. |

The companion `scripts/st3-instance-pair-proof-test.py` defines pure source controls for
50 exact document names/content, binding claim and physical-cut checks, a bounded
differing-claim list, secret-free errors, constructed child environments, and an unreaped
child's failing receipt with its root preserved for the disposable-job owner. It also
checks exact daemon-owned loopback bind, an extra or wildcard listener, loopback and
nonloopback child TCP remotes, and preexisting-root refusal. It imports
the program but starts no native fixture. The test file is submitted for review, **UNRUN**.

The meaningful source controls for the new script are: altered/extra environment, preexisting
or symlinked root, wrong binary, CLI rejection of `loopback`, unexpected fleet route, wrong
fleet/member/key, nonempty admission fields, changed physical cut, duplicate/missing or
wrong-content document, missing/mismatched binding claim, exhausted request/byte/claim
budget, bounded differing-claim report, leaked token-shaped exception staying out of the
receipt, child failure, and failed cleanup. Source parsing and diff review can inspect these
paths; **none of those controls has been executed**. A native fixture run requires an exact
Intake source/testing disposition, reviewed operator command, real candidate binary/CLI
receipt and separate Operations executor/egress/cleanup binding. The first reviewed command
target is conditional, not a promise to run.
