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

The fixed image environment sets HOME `/state/home`, XDG_CONFIG_HOME `/state/config`,
XDG_STATE_HOME `/state/data`, XDG_RUNTIME_DIR `/state/run`. No host `ST_AGENT`, ST3 variables,
SSH agent, production home/credentials/socket or caller environment is imported. The program
refuses such an environment; it does not clear or unset the invoking seat's identity.

The image's operator-owned program creates a new synthetic guest-local
`person/scratch-fixture` configuration for the existing invite API. This is test bootstrap,
not a host person, host agent impersonation, delegated grant or production admission. It is
an explicit source-review decision, not automatically safe because the environment is empty.
If Intake does not accept this fixed operator bootstrap path, the fixture must refuse and
the next source slice must supply a reviewed alternative before a run. No production ancestry
or person guard is weakened. The existing `st3-fixture` binary is not used.

Both members use foreground real release `st3 up` and `replication-worker` processes, fresh
private stores and native signed fleet invite/join. Ports31313(a)/31314(b) stay inside the
private two-member network. Fixed loopback relays connect those native endpoints to the
exact Docker aliases `a`/`b`; no arbitrary destination is accepted. This is local TCP
tunneling, NOT Fabric/Tailscale performance or cross-host proof. Operations must test internal
network egress refusal from inside the member before enabling the fixture. Unsupported
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
matching complete admitted-input hashes and every semantic document field against a; a
captures again before acknowledging success. Arrival `store_index` and `created_index` are
kept locally or explicitly excluded from cross-member semantic equality; they are not global
cut identity. The envelope admission/digest check is diagnostic, not a raw cache oracle.

This certificate relies on exclusive, fixed fixture writers with no repair/trim/reindex or
external raw writes. Index equality would not certify those operations on a general live
store. This fixture does not certify every shared projection table, checkpoint ancestry or
an iOS session. If the requested proof needs a broader canonical-input/projection certificate,
that accessor is missing and the result must be labelled incomplete for that broader goal.
Do not rename this narrow fixture cut into a full graph certificate.

Each member emits one final secret-free JSON line below48KiB, fitting Operations'64KiB tail.
It includes the cut fields, field-comparison result and child-reap status. Invitation tokens,
native private keys and raw API/child errors never appear in that receipt. Nonzero status,
missing receipt or failed operator cleanup stays a failed/incomplete proof.

## Authored controls and qualification plan

`scripts/st3-scratch-pair-proof-test` contains unrun controls for same-count wrong content,
duplicate documents, differing binding/timestamps, differing admitted input, unsafe mounts
and overbudget history refusing before document body copies. Python AST parsing and
diff-check are the only local validations. Tests and fixture have not run.

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
