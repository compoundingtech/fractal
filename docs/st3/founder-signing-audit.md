# Audit founder claim signatures

Stores populated by v0.3.4 can hold delegation grants sealed without claim signatures; later claims
that rely on them fail with `delegation <claim-id> is unsigned`. v0.3.5 and later sign preserved
unsealed work but cannot repair sealed payloads.

## Preserve evidence before running diagnostics

On a suspected affected store, capture a bounded read-only audit before running `st doctor`,
restarting services, founding/joining a fleet, or invoking a checkpoint. Doctor can seal pending
batches. Those operations on an old build can change the evidence or turn recoverable unsealed
work into immutable unsigned history. A previously captured doctor report should be retained.

Preserve the original store and private keys together with configuration, the exact installed
binary, `BUILD.json`/`RELEASE.json` when available, archive and binary checksums, and timestamps.
Keep raw records private. For a disposable VM, preserve a consistent whole-guest snapshot before
experiments. A live SQLite backup must include the committed WAL state: copying only
`claims.sqlite3` while it is in use is insufficient. SQLite's read-only source connection and
backup API can make a consistent database copy without opening it through st. Preserve local key
files separately, and coordinate a consistent filesystem snapshot if keys or configuration may
change while copying. Work on additional copies; retain the untouched original.

[Claim backups](backups.md) are useful for shared envelope history, but are not a substitute for
this initial raw capture: offline export opens and migrates its input, and an envelope archive
does not preserve private keys or pending unsealed claims. Backup restore preserves old sealed
payloads and therefore does not repair an unsigned delegation.

## Run the bounded read-only audit

From a checkout containing this guide, using Python 3:

```sh
python3 scripts/audit-claim-signatures /path/to/claims.sqlite3 > audit.json
```

The database path is an invented example. Select the actual configured database; do not post
real host names, private claim bodies, keys, credentials or fleet secrets. If Python's optional
`cbor2` module is available, the script also reads signature maps from the original sealed CBOR
payloads. Install audit dependencies away from the specimen, or audit a preserved copy on another
machine. Without the decoder it still prints SQLite counts and marks payload inspection
incomplete. `--help` describes the limits.

The script uses `mode=ro`, `query_only`, and one read transaction. Queries have an instruction
budget; grant and cached-invalid lists have row caps. Sealed payload checks also have per-grant,
per-payload and total-byte caps. It never runs doctor, seals claims, migrates the schema, changes
verdicts or opens the database through st. A missing database fails without creating one.

| Output | Meaning |
| --- | --- |
| `grants_observed` | Current retained `principal.key-granted` claims examined. |
| `sealed_without_stored_signature` | Grants in stored envelopes without a local signature row. Check their original payload maps too. |
| `unsealed_without_stored_signature` | Pending grants without a signature row; preserve keys and raw state before upgrade. |
| `cached_invalid_observed` | Cached invalid verdicts at this snapshot, across all claim kinds. |
| `unsigned_delegation_errors_observed` | Observed invalid reasons that name an unsigned delegation. |
| `sealed_payloads` | Actual grant/signature presence and agreement with stored signature rows, when decoding is available. |
| `complete` / `limitations` | Whether all requested bounded inspections completed; caps, missing decoder and errors are explicit. |

Exit status is zero for a completed audit, even when it finds affected grants. Status 2 means
incomplete evidence. A cap, interrupted query, unsupported schema or decoding failure must not be
reported as a clean result. Counts cover retained history on the inspected replica only, and
cached verdicts can be stale or absent. This audit checks stored evidence; it does **not**
recompute cryptographic verdicts. Repeat separately on each accessible peer. Label public results
with invented names such as `orchard` and `meadow`, include snapshot indices/times and limitations,
and explicitly list uninspected peers. Trimmed history is outside this audit.

## Choose the outcome for the captured state

**Preserved unsealed work:** use a verified build containing the prevention fix and retain the
original node, person and agent keys. First rehearse the upgrade on an isolated copy with no production
transport. Record the build identity, original claim IDs and body bytes, key inventory, and grant
chains before and after. Only after raw capture, let the patched daemon seal and verify claims.
Inspect `claim-signatures` separately from the overall doctor result. Then exercise populated
restart, founder activation, continued person and agent work and peer replication in that rehearsal.
Do not infer signature acceptance from digests alone. Coordinate any actual rollout with the
operator after the rehearsal; this audit does not restart or upgrade a live system.

**Already-sealed unsigned delegations:** an upgrade prevents further startup ordering failures
but retains these payloads and their signature warnings. Preserve the exact invalid reasons and
the original grant/signature maps. Do not rewrite envelopes, insert signature rows by hand,
delete verdicts, suppress doctor warnings, or reinterpret old claims as authenticated. Restoring
an envelope backup or signing the envelope's outer wrapper cannot provide the missing inner
claim signature.

For future operations, a person may enroll a new seat or signing device through existing
supported workflows **when its authority chain already verifies**; see [device
signing](device-signing.md). Verify the new grant and a new signed operation on each participating
replica. A fresh child grant still depends on its parents: issuing one under an unsigned root or
delegation does not bypass that failure, and a new grant cannot authenticate earlier claims.
If the necessary authority chain is affected, retain the old graph as evidence and request a
separate operator decision about future setup or recovery. Do not reset identity, discard the
graph, or rebuild it merely to make diagnostics green.
