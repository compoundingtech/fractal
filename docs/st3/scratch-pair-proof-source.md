# Slice 0 fixed pair fixture: source review input

Status: authored and unrun. The source/testing review precedes any invocation. This is a
candidate fixture for Operations' isolated root trigger; it neither installs that trigger
nor implements scratch authority. Its guest-local bootstrap actor needs explicit review.

Fixed argv proposed for the root-owned fixture list:

```text
/usr/bin/python3 /opt/st-scratch/st3-scratch-pair-proof a
/usr/bin/python3 /opt/st-scratch/st3-scratch-pair-proof b
```

Only the fixed reviewed fixture name selects these argv; the caller cannot change their
roles, command, path, image, environment, endpoints or data. The image contains Python's
standard library and the pinned release st binary. No code executes from `/state` or `/tmp`.
The per-member runtime requires UID65534, a read-only root, separately owned noexec/nosuid/
nodev tmpfs mounts `/state` (256MiB) and `/tmp` (64MiB), and at most2GiB/4CPU/512PIDs.
The outer run is at most1800s with operator-owned cleanup. This script's lifecycle result
only covers its child processes; it cannot certify container/network/volume cleanup.

Candidate st input: source `21fd968d3c5534a1ebb33458e8ff25c6d4e2da90`, existing release binary
SHA256 `eeae8c11562b4c048ce59c52dd0b75b543bb139e68caae274ad45da06d3f5938`.
Read-only ELF inspection found interpreter `/lib64/ld-linux-x86-64.so.2` and dependencies
`libm.so.6`, `libc.so.6`, `ld-linux-x86-64.so.2`. This is dynamically linked; an exact qualified
rootfs must carry its required loader/libraries and Python dependencies. It is not sufficient
to put that binary in an empty rootfs. No rootfs/image/tarball has been built or loaded.

The fixed image environment sets exactly HOME `/state/home`, XDG_CONFIG_HOME `/state/config`,
XDG_STATE_HOME `/state/data`, XDG_RUNTIME_DIR `/state/run`, PATH `/usr/bin:/bin`,
PYTHONDONTWRITEBYTECODE `1`, and the container-set HOSTNAME. Any extra key or changed value
refuses, including a token under an otherwise unknown name. No host identity, SSH agent,
production home/credentials/socket or caller environment is imported.
Before creating state, the guest checks NoNewPrivs=1, CapEff=0, no IPv4 or IPv6 default route,
only the embedded 127.0.0.11 resolver, failure to resolve example.com, and exactly lo/eth0
interfaces. The receipt records each boolean. These are fail-closed honest-guest interlocks,
not hostile-code isolation or a substitute for Operations' external egress test.

The image's operator-owned program creates a new synthetic guest-local
`person/scratch-fixture` configuration for the existing invite API. This is test bootstrap,
not a host person, host agent impersonation, delegated grant or production admission. It is
an explicit source-review decision. Intake accepted this bootstrap only for the old
`83f183d7…` script hash; this changed script requires a fresh own-diff review before any run.
The interim receipt exposes the guest fleet ID and SHA256 fingerprints of the UTF-8 public
member-key strings for both members. Operations must check that fleet ID against every
production fleet before trusting a
result. No production ancestry or person guard is weakened. The existing `st3-fixture`
binary is not used.

Both members use foreground real release `st3 up` and `replication-worker` processes, fresh
private stores and native signed fleet invite/join. Ports31313(a)/31314(b) stay inside the
private per-run two-member network. Fixed loopback relays connect those native endpoints to the
exact Docker aliases `a`/`b`; no arbitrary destination is accepted. Each member's relays
allow at most64 connections and64MiB total transfer in both directions during the run;
each connection remains limited to16MiB per direction. This is local TCP
tunneling, NOT Fabric/Tailscale performance or cross-host proof. Operations must test internal
network egress refusal out of band before enabling the fixture. The `tailscale` transport
flag is a candidate CLI configuration requirement here; the loopback relay is the actual
fixture transport and supplies no Tailscale proof. Unsupported
loopback announcement/join behavior at the candidate pin is a native fixture failure, not
permission to change the production transport/admission code.

Exactly50 invented UTF-8 documents are generated inside a. Their names, bytes and idempotency
keys are fixed. b receives them by real membership replication. Cached digest equality is
used only as readiness; it cannot replace the later field/input proof. No third member,
arbitrary fixture data, command, hosted credential or interactive person step is accepted.

## What the cut does and does not certify

The scope is explicitly `exhaustive-fixture-documents-and-admitted-claims-v1`, not a generic
complete Source or projection oracle. The approved outer backend supplies exclusive fixture
writer ownership. Each member captures health/build/index, one complete claim page capped
below500 claims/8MiB wire bytes, all50 document versions and their content, then the actual
index/build again. Any physical index change, paginated source, missing binding claim,
admission residue, wrong build, missing document or byte mismatch refuses. b compares
matching complete admitted-input hashes, fleet ID, exactly two current members `a` and `b`
with the expected public-key fingerprints, and every semantic document field against a; a
captures again before acknowledging success. Arrival `store_index` and `created_index` are
kept locally or explicitly excluded from cross-member semantic equality; they are not global
cut identity. The envelope admission/digest check is diagnostic, not a raw cache oracle.

This certificate relies on exclusive, fixed fixture writers with no repair/trim/reindex or
external raw writes. Index equality would not certify those operations on a general live
store. This fixture does not certify every shared projection table, checkpoint ancestry or
an iOS session. If the requested proof needs a broader canonical-input/projection certificate,
that accessor is missing and the result must be labelled incomplete for that broader goal.
Do not rename this narrow fixture cut into a full graph certificate.

When admitted-input hashes differ, the receipt contains at most20 differing claim IDs and
kinds from the two bounded claim pages. The full claim references travel only over the
private control connection and are omitted from the success receipt. This diagnostic does
not relax the equality requirement.

Each member emits one final secret-free JSON line below48KiB, fitting Operations'64KiB tail.
It includes the cut fields, field-comparison result and child-reap status. Invitation tokens,
native private keys and raw API/child errors never appear in that receipt. Error codes use
a closed source-authored vocabulary; unknown refusal text is reduced to a fixed code.
Nonzero status,
missing receipt or failed operator cleanup stays a failed/incomplete proof.

## Authored controls and qualification plan

`scripts/st3-scratch-pair-proof-test` contains unrun controls for exact admitted binding
identity/body/owner/time, same-count wrong content, duplicate documents, differing
binding/timestamps, differing admitted input and bounded claim diagnostics, unsafe mounts,
and overbudget history refusing before document body copies. New controls cover unexpected
environment keys, in-guest isolation inputs, injected `stj1-`/key-like exception text staying
out of stdout, cleanup failure returning nonzero, fleet ID/member key mismatches, relay
connection/byte budgets, unexpected control peer, and a realistic 50-row receipt width.
Python AST parsing, JSON syntax and diff-check are the only local validations. Controls and
fixture have not run.

After exact source/testing disposition, ordinary hosted qualification must retain both
real member processes, actual binary/image/fixture hashes, INITIAL/TRY/adverse results,
complete cuts and all50 field/content comparisons. Include negative guest-actor/bootstrap,
wrong build, wrong/nonempty state, inherited identity/credential, stale/mutating cut,
extra/duplicate/wrong document, late peer failure and child/outer cleanup controls.
The source controls alone do not replace that native run. No Docker group, host runtime,
new collector or production fixture execution is requested.

Operations still supplies the exact reviewed rootfs recipe/image build, tarball checksum and
local Docker image ID, root-owned list/trigger and outer cleanup/egress proof. Neither image
ID nor native execution is fabricated from the source pin. Packaging's estimate starts only
after the agreed actual reviewed command and image inputs exist.
