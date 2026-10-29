# Claim backups

`st backup create FILE` saves one live daemon snapshot as a private file. It reads signed sync
envelopes on a pooled read worker in pages of 512, then downloads the completed archive. Writes
and other requests continue while it reads. The server returns headers after a bounded health
check; archive preparation follows the response stream lifetime, rather than the ordinary
15-second header deadline. Only one live export may prepare or download at a time; another
receives `backup-busy` (503). Dropping the response cancels preparation, and the slot remains
occupied until its worker has released the snapshot. A slow download does not keep the database snapshot
open. The daemon needs temporary disk space for the archive; the destination is published only
once its checksum and end record verify. Existing backup files are never overwritten. If
replication has admitted claims whose projections are still pending, or the graph is stale,
export refuses before writing an archive; retry after the graph has recovered. A missing writer
predecessor also prevents publication until that history arrives.

Preparation can take minutes before the first archive bytes arrive. The CLI has no total or
idle read timeout on this download; the normal local Unix-socket path has no HTTP proxy.
A separately configured HTTP proxy must allow that preparation interval. A producer failure
after headers is reported as a terminal `failure` record with a code and message, never a valid
backup; the CLI reports the cause and leaves no destination file. Older clients still reject
such an incomplete export but may show a generic format error. Successful archive bytes and
the restore format are unchanged.

```sh
st backup create before-upgrade.jsonl
st backup restore before-upgrade.jsonl --database /var/lib/example/recovery/claims.sqlite3
```

Restore runs offline, builds a temporary database at the current schema, and publishes it only
once admission, projection and digest checks pass. It refuses a destination with existing claims.
Without `--database`, restore selects the configured state's `claims.sqlite3`. Stop the node before
restoring there. Neither command installs services or starts a daemon.

A backup of a member also holds the whole text of the [long messages](long-messages.md) that
member owns, which are files outside the database; restore checks each against its hash and its
claim and returns them beside the restored database. The restore report says how many it returned
and how many long messages the claims name in all, since another member owns the rest.

For rehearsals, make a consistent SQLite copy first, then export that copy:

```sh
st backup create rehearsal.jsonl --database /var/lib/example/copy/claims.sqlite3
st backup restore rehearsal.jsonl --database /var/lib/example/restored/claims.sqlite3 --json
```

The offline exporter migrates and finishes upgrade recovery on its input, so use a copy. Its header identifies the exporting
build and the schema of that migrated snapshot. A pinned old daemon writes the compatibility test
fixture; the current exporter archives a copy of that graph and the current restorer verifies
that its envelope inventory, claim sources and all graph table digests match.

## Preserve suspected unsigned history before maintenance

A populated v0.3.4 store can contain sealed delegation grants without claim
signatures. Preserve raw state and keys **before** doctor, restart, checkpoint,
or offline export: those operations can seal pending work or migrate their input.
A claim archive does not preserve private keys or all pending unsealed work.

Keep the original database, keys, configuration and exact binary together with
checksums and capture times. A consistent SQLite copy must include committed WAL
state; copying only the live database file is insufficient. Use SQLite's read-only
source connection and backup API, or a consistent filesystem snapshot that also
accounts for keys and configuration. Retain an untouched original and inspect a copy.

From a clean checkout, use Python 3 to run the bounded stored-evidence audit:

```sh
python3 scripts/audit-claim-signatures /path/to/preserved/claims.sqlite3 > audit.json
```

The path is an invented example. The script opens SQLite with `mode=ro`,
`query_only` and one read transaction; it never opens the store through Fractal,
seals claims or changes verdicts. A missing database fails without creating one.
Its `--help` lists query, row and payload caps. Optional `cbor2` enables inspection
of original sealed payload maps; without it, payload inspection is incomplete.
Install dependencies away from the specimen or inspect a preserved copy elsewhere.

Exit 0 means the bounded audit completed, including when it found unsigned grants;
exit 2 means incomplete evidence. Read `complete` and `limitations` along with the
stored-signature, sealed-payload and cached-invalid counts. This tool does not
recompute cryptographic validity, and retained history on one replica does not
establish the state of its peers. Keep raw results private.

Preserved unsealed work needs an isolated upgrade rehearsal with its original keys
and a verified build before any rollout. Already-sealed unsigned delegations remain
affected: restoring their envelopes, signing an outer wrapper or issuing a child
grant cannot supply the missing inner signature. Do not rewrite envelopes, insert
signature rows, delete verdicts or reset identities to clear warnings. Preserve the
evidence and obtain an operator's recovery decision for the affected authority chain.

## Rehearse a restore

Use a healthy current daemon and a separate empty directory. These commands neither start a
daemon nor connect the recovered state to your live fleet:

```sh
(
  set -eu
  umask 077
  st_recovery_root=$(mktemp -d "${TMPDIR:-/tmp}/st-recovery.XXXXXX")
  st backup create "$st_recovery_root/claims.jsonl"
  mkdir -p "$st_recovery_root/restored"
  st backup restore "$st_recovery_root/claims.jsonl" \
    --database "$st_recovery_root/restored/claims.sqlite3" --json \
    > "$st_recovery_root/restore-report.json"
  printf 'Private rehearsal files: %s\n' "$st_recovery_root"
  cat "$st_recovery_root/restore-report.json"
)
```

Always pass `--database` for this rehearsal. Without it, restore selects the configured live
state directory; that default is not the separate-state procedure. Stop here if export or
restore fails. The commands run in a subshell so the umask does not change your interactive shell.

Keep the report private. Check the returned `writer`, `envelopes`, `source_graph_digest`,
`graph_digest`, `log_digest` and `projections_match` against the format rules below. A different projection
schema can legitimately rebuild different tables; equal envelope history is mandatory, and a
successful command alone does not establish identical projections across versions.

To run this recovered node later, configure a **new** state directory, its returned `writer`
as `node`, and distinct API and gateway sockets. Use the current verified binary and fresh local
keys. Never point the live service at this directory as part of the rehearsal, copy the original
writer's keys into it or start two nodes with the same writer identity. Follow [Starting a
recovered node](#starting-a-recovered-node) for fleet joining. Keep the original member intact
until a separately planned recovery accounts for writes since the snapshot.

## Format and verification

The archive is UTF-8 JSON Lines, format `st3.claim-backup`, version `1`, `2` or `3`. Each record is limited
to 128 MiB; export refuses a larger record before publishing a file:

- `header`: format version, exporting build, database schema and claim registry digest, fleet ID
  and public anchor, complete admitted envelope-log digest, graph digest, each shared table's digest and row count,
  and from version `3` each permanent ledger's digest and row count.
- `envelope`: the unchanged `ReplicaEnvelope` sync wire record, including writer, sequence,
  previous batch hash, payload hash, accept time, base64 CBOR claims and document bytes, and the
  writer's public key and signature when present. Envelopes sort by writer, sequence, hash;
  projections replay claims in their normal canonical total order.
- `signatures`: sync signature records, including additional signatures held for those envelopes.
- `checkpoint`: the sync manifest of the newest applied or in-progress trim, including older
  tombstones. Restore initializes empty checkpoint state through the normal certificate checks,
  even if the source knew a newer certificate it had not applied yet.
- `ledger`: one page of one permanent ledger, in its primary-key order (see below).
- `end`: envelope count and SHA-256 of all preceding bytes, including line endings.

An archive that carries the long-message texts of its member is version `2`. An archive that
carries permanent ledger rows is version `3`. A build that predates a version refuses its
archives cleanly instead of restoring part of them; an archive with neither keeps version `1`,
so every build restores it.

### Permanent ledgers

A trim deletes claims, records and batches, but keeps what readers still need of them in
ledgers that only grow: the retired claim IDs and envelope identities, with the checkpoint that
withdrew each legacy tombstone; the retired claims' operation receipts and ancestry; each
writer's highest batch; and which claim replaced each repaired original. Restoring the envelope
log and the newest checkpoint manifest rebuilds only the rows that manifest and those batches
still imply, so an earlier checkpoint's withdrawn tombstone or a trimmed writer head would be
lost. Version `3` carries every ledger row except the writer heads and repair replacements that
a retained batch or repaired record still implies, which restore rebuilds from the log.

The header records each nonempty ledger's row count and digest. Restore adds the archived rows
by union: an identity is never removed and a writer head never lowered. After admission and the
checkpoint manifest add their own rows, each ledger's digest has to equal the header's; a missing
row would forget a deletion, an extra one would invent one. The checkpoint manifest is still
restored through its certificate checks, as before.

Restore verifies signatures, envelope and claim hashes through normal admission, checks writer
chains, reapplies replicated repairs, and replays projections with the current code. Pre-membership envelopes retain their
unsigned legacy wire representation. As in normal replication, later membership changes do not
revoke envelopes already admitted. Missing or undecodable envelopes fail the restore. Unknown
or quarantined claim records retain the normal admission behavior; the admitted source digest
must still match exactly.

The complete envelope-log digest always has to match, including certified tombstone identities.
With the same registry fingerprint, the claim source digest and count also have to match. The
fingerprint covers the known claim kinds and shared projection layout, as it does for sync.
With the same schema and fingerprint, all table digests and the graph digest also have to match.
An older schema or registry can produce
new projections after an upgrade, including previously unknown claims: JSON output reports the source and rebuilt graph digests,
`projections_match`, and rebuilt table digests so that this difference is visible. Envelope payloads
remain unchanged. Keep old claim readers in the registry when adding new kinds; SQL migrations
are not a backup compatibility mechanism.

## Starting a recovered node

Restore returns a fresh writer named `restored-...`. Set `node` in the recovered node's config to
that exact value before starting it. The database refuses another writer at startup. Use a fresh
state directory, generate new local keys, and join the fleet as that new member. This avoids ever
signing a sequence that the original writer already used, including writes after the backup.

The archive contains durable shared claims, their referenced blobs, public signature material,
checkpoint tombstones and the permanent ledgers. It does not contain private node keys, invite secrets, device capabilities,
driver state, active terminals, local observations, lease renewals, transcripts, staged blobs,
configuration, or other local caches. Protect the file like the original graph: claims and document
bytes can contain private information. A restored standalone graph can be inspected immediately;
running seats and fleet transport require fresh local setup.

The format uses sync records.
