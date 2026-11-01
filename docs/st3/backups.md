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
Without `--database`, restore selects `claims.sqlite3` in the configured `store_dir`, or
`state_dir` when `store_dir` is unset. Stop the daemon before restoring there. Neither command
installs services or starts a daemon.

A backup of a member also holds the whole text of the [long messages](long-messages.md) that
member owns, which are files outside the database; restore checks each against its hash and its
claim and returns them to the product state root. Moving the database with `store_dir` does not
move those files. The restore report says how many it returned and how many long messages the
claims name in all, since another member owns the rest.

An explicit `--database` matching the configured database still reads or restores bodies in
`state_dir`. For an arbitrary `--database` override, its parent remains the auxiliary product
root, preserving standalone rehearsal behavior. For an offline export of a copied database,
copy its `message-bodies` alongside it too if those bodies should be archived.

For rehearsals, make a consistent SQLite copy first, then export that copy:

```sh
st backup create rehearsal.jsonl --database /var/lib/example/copy/claims.sqlite3
st backup restore rehearsal.jsonl --database /var/lib/example/restored/claims.sqlite3 --json
```

The offline exporter migrates and finishes upgrade recovery on its input, so use a copy. Its header identifies the exporting
build and the schema of that migrated snapshot. A pinned old daemon writes the compatibility test
fixture; the current exporter archives a copy of that graph and the current restorer verifies
that its envelope inventory, claim sources and all graph table digests match.

## Relocate only the SQLite store

`store_dir = "/absolute/path"` in the daemon's TOML config, or `fractal up --store-dir
/absolute/path`, selects the directory for exactly `claims.sqlite3`, `claims.sqlite3-wal`,
and `claims.sqlite3-shm`. Unset configuration keeps the database in `state_dir`; there is no
automatic file move. Keep the product state directory and its identity, keys, sockets,
message bodies, transcripts, PTYs, runtime files and caches unchanged.

This is an offline filesystem relocation, not a logical restore: it preserves the same
writer and local identity. Use the same binary and config except for `store_dir`. Record the
original effective config, state root, database path and service start command before changing
anything. Do not combine this operation with an upgrade or writer rename.

Startup fails closed if an existing database's identity cannot be verified: it must carry
a matching persisted local writer binding (`meta.local_node_writer`), local runtime launch
receipt, or backup-restore writer marker. An old database with none is not safe to adopt
merely because its filename or config node matches.
Preserve the original files and resolve the missing identity evidence; do not erase or edit
identity metadata to bypass the refusal.

The three database files must not be symlinks; ancestor directory symlinks are supported.
One admitted-store handle retains the directory FD and exclusive flock throughout daemon
and fleet admission. Linux opens through `/proc/self/fd/FD/claims.sqlite3`; before any
SQLite initialization or migration write, the opened filename and database entry must match
the admitted directory and file identity. A removed or replaced explicit directory is never
recreated. macOS uses pathname opening with the same pre-write identity checks, rather than
a Linux `/proc` descriptor path. SQLite's Unix VFS can canonicalize descriptor paths, so the
checks compare pathname metadata rather than SQLite's internal file descriptor. Transient
namespace changes that are switched back between checks, and later VFS sidecar operations,
remain residual risks on both platforms. Keep the directory stable throughout daemon use
and fleet admission; this is not protection against a hostile process rewriting its namespace.

A fleet join that would change an existing configured store's local writer is refused before
redeeming the invite or saving membership. Keep that writer name when joining. Explicit-store
fleet migration checks the database's fleet binding after stopping the daemon and acquiring
the admitted-store lock; it cannot inspect through that exclusive handle while the daemon
still owns it.

1. Stop **only the daemon**, using its service manager (for example,
   `systemctl --user stop st3.service` for the legacy Linux unit; use the actual installed unit).
   Prevent automatic or socket-triggered restarts for the maintenance window. For a foreground
   daemon, terminate that exact daemon gracefully. Do not stop agents, PTY servers, terminal
   sessions or the whole product process tree. Confirm the daemon has exited and no other
   process has the database open before continuing.
2. Set the paths below to the existing database directory and an empty destination on durable
   local storage. The destination must be absolute, owned by the daemon user, and not contain
   any existing trio member. Run as the daemon user, with a private umask.
3. Checkpoint the stopped source using SQLite and **inspect the returned busy field**:

   ```sh
   umask 077
   old=/absolute/current/database-directory
   new=/absolute/new/database-directory
   sqlite3 "$old/claims.sqlite3" 'PRAGMA wal_checkpoint(TRUNCATE);'
   ```

   The first field must be `0`; a successful TRUNCATE normally returns `0|0|0`. A nonzero
   busy field is a failed prerequisite even when the CLI exits successfully. Do not copy,
   delete a WAL, or proceed until all database users have released it and checkpoint succeeds.
4. Copy the database and each sidecar **if it exists**, preserving permissions. Keep a separate,
   private rollback copy of the checkpointed trio outside both active directories before
   removing any source file:

   ```sh
   mkdir -m 700 "$new"
   for file in claims.sqlite3 claims.sqlite3-wal claims.sqlite3-shm; do
     if test -e "$old/$file"; then
       cp -p "$old/$file" "$new/$file"
       cmp "$old/$file" "$new/$file" || exit 1
     fi
   done
   sqlite3 -readonly "$new/claims.sqlite3" 'PRAGMA quick_check;'
   ```

   Require `quick_check` to return `ok`. Byte equality of every copied file verifies that the
   entire store, including its identity and local metadata, is the same, not a newly created
   or logically restored graph. Record and compare the source and destination `meta` rows
   privately if inspecting identity separately (`SELECT key,value FROM meta ORDER BY key;`).
   Do not publish these rows. Check that any absent sidecar is absent at the destination too.
5. Only after all copies and verification succeed, remove the original three named files
   from `old` (do not remove the directory or any other product file). Retaining an original
   active trio leaves ambiguous startup paths; the rollback copy must live elsewhere.
   Set `store_dir` to the absolute `new` path in the config actually used by the service.
   A one-off foreground flag does not change the installed service's config.
6. Restart only the daemon using its original service/start command. Confirm its reported
   database path is the new path, its writer/node and fleet identity are unchanged, the
   existing claims are readable, and long-message bodies and surviving sessions still work.
   Stop immediately if it opens an empty graph, reports another identity, or cannot adopt
   the existing runtime. Never start another daemon against the rollback copy.

### Relocation rollback

If no writes have occurred since relocation, stop only the daemon again and restore the
verified rollback trio to the original location, remove the destination trio from its active
path, restore the original config, then restart. Never overwrite a live database.

If the relocated store has accepted writes, the old rollback snapshot is stale: do not
restart it or copy it over the new graph. Stop the daemon, checkpoint the **current new**
database with TRUNCATE and require busy `0`, then reverse the verified-copy procedure with
that current trio. Remove the now-inactive trio before reverting `store_dir`. Preserve a
private copy of every candidate until recovery is verified; never merge individual database,
WAL or SHM files from different snapshots. A database rollback does not roll back external
product state, so retain the unchanged state root throughout.


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
database (`store_dir` when set, otherwise `state_dir`); that default is not the separate-state
procedure. Stop here if export or restore fails. The commands run in a subshell so the umask
does not change your interactive shell.

Keep the report private. Check the returned `writer`, `envelopes`, `source_graph_digest`,
`graph_digest`, `log_digest` and `projections_match` against the format rules below. A different projection
schema can legitimately rebuild different tables; equal envelope history is mandatory, and a
successful command alone does not establish identical projections across versions.

To run this recovered node later, configure a **new** state directory, select the restored
database with `store_dir` if it is separate, set its returned `writer` as `node`, and use
distinct API and gateway sockets. Use the current verified binary and fresh local
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
