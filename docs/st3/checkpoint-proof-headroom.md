# Checkpoint proof scratch headroom

The checkpoint proof currently makes a full scratch copy of the live SQLite database on filesystems
without reflinks. On an ext4 member this uses `VACUUM INTO`; its preparation and comparison also
write a rollback journal and SQLite temporary files. A proof must leave capacity for the live
writer even when the scratch filesystem is full or the copy is interrupted. This is the first
storage change in the [checkpoint capture design](checkpoint-capture.md). A later change will
construct the proof database from only the sealed cut.

Before it creates a capture or proof scratch directory, the gate opens a SQLite read snapshot and measures page size `P`, page count `N`, and logical image `S=P*N`. Committed WAL pages are included in that SQLite image. It separately measures main-file allocation and length, WAL length and allocation, and available bytes on the live, scratch, and resolved SQLite temp devices. The temp directory follows SQLite's process override and Unix VFS search order. Existing staging is already reflected in available space.

The live reserve remains **8 GiB plus max(8 GiB, allocated live DB bytes)**, including the WAL growth allowance. The proof admission derives a minimum from a copy bound `C`, initial rollback-journal bound `J(N)`, and an empty proof-schema floor `F`. `C` covers the larger of the SQLite snapshot image and the rounded raw main-plus-WAL clone until WAL fold. The VACUUM branch reserves `S` at admission and has a separate guarded destination cap `S + (2*B + pointer_map_pages + G_w)*P`: `B` is the count of table and index b-trees in the same snapshot as `P,N`, plus the `sqlite_schema` b-tree; `pointer_map_pages` is zero without auto-vacuum and otherwise `ceil(N / (usable_page_size/5))`; `G_w = floor(wal_growth_allowance/(P+24))` converts the existing live WAL growth allowance to whole frames. The two-page b-tree term allows a rebuilt tail leaf and interior/root page; the pointer-map term covers auto-vacuum layout. This is an enforced refusal cap, not a guarantee that VACUUM will succeed. Destination growth above `S` draws from the enforced remainder on the scratch device, and growth beyond the cap is refused before its write. For a 4 KiB-page store under 8 GiB allocated main-file size, the existing 8 GiB WAL allowance yields `G_w = 2,084,935` pages, so it dominates the 654-page b-tree term in the current schema. The practical limit is the scratch device remainder `E_d`: available bytes minus its live reserve (which already contains the WAL allowance), 256 MiB reaction margin, and admitted minimum. The unit tests inject smaller caps to exercise refusal. Capture also reserves one staging image. The journal includes a page and record header per protected page plus header and sector slack. A 256 MiB reaction margin is retained on each device. VACUUM sorter files and later proof or replay growth draw from the remaining admitted capacity on their actual device, including SQLite's resolved temp device. The gate can refuse while physical headroom remains, and the refusal names the controlling device and term.

For a proof, a uniquely named SQLite VFS wraps the platform default on proof connections. It tracks each file's rounded logical high-water and charges possible new growth once; repeated writes at the same offset add no peak charge. Cumulative `xWrite` requests are diagnostics, while `st_blocks` and `statvfs` remain separate measurements of observed allocation and free bytes. The VFS refuses a projected crossing before forwarding an allocation-capable write, truncate, shared-memory map, or size hint. It caps a clone at admitted `C`, a VACUUM destination at its separately derived cap, and a rollback journal at `J(Q)` with `Q` refreshed from the proof copy before later transactions. The canonical sealed-claims ordering runs in one explicit `BEGIN IMMEDIATE`/`COMMIT` transaction, so its journal peak is governed by the one `J(Q)` bound. A 16 MiB request cap and 256 MiB reaction margin remain. An over-budget self-test must refuse before copying. After either VACUUM or a reflink clone and WAL fold, the copy's SQLite page count and EOF are measured again against admission. The source-WAL allowance stays independent of proof scratch and temp accounting.

Every proof connection disables mmap. The VFS also prevents the SQLite sorter from mapping its temp file; sorter reads and writes stay wrapped. A refusal closes the connection without more SQL, then removes the disposable copy and sidecars with checked errors. Other processes may consume free space between checks, so `ENOSPC` remains an abort path and never a verification claim. A seal published before the proof attempt remains in place.

Resumable capture staging keeps the default SQLite VFS and is charged one full live DB-size
allowance up front. Its capture phases check available space at phase boundaries. A VFS refusal
inside its transaction could prevent rollback and leave an unusable staging file. A later PR
replaces the whole proof copy and puts `max_page_count` on construction.

One proof holds `proof.lock` in its scratch directory. It creates and syncs an owner marker before
the proof file, removes the copy and its journal, WAL and SHM sidecars with checked errors, then
removes the marker. Daemon startup and the next checkpoint pass sweep marked orphans only while holding the lock;
unknown unmarked proof files require inspection. Any removal error names the path and refuses the
next proof until the files are cleared. If an unexpected SQLite file handle remains open, the
process keeps the owner lock until exit so an open file is never unlinked by the sweep. Resumable
capture staging has a separate lifetime.

## Read-only host census before merge

The invented member labels and `$ST_STATE_DIR` paths below stand in for the private host and home identifiers. Ops records one row for each checkpoint member (`linux-a`, `linux-b`, `mac-a`, `mac-b`): live DB
allocated bytes, live WAL allocated bytes, available bytes and device IDs for live, scratch and
selected SQLite temp directories, then the gate's required bytes, deficit and verdict. This
requires `stat`/`statvfs` only; it does not make a production DB copy. The production DB size,
retained fraction, proof disk peak and elapsed time are still unknown until measured. The prior
2.68× RAW comment is not held evidence. The existing checkpoint heap test measures **MEMORY**,
not **DISK**. Linux/ext4 behavior is the local implementation target; APFS `f_bavail` semantics
and the guarded SQLite VFS on mac-a and mac-b remain UNKNOWN until tested there. Each proof
performs a deliberate over-budget self-test write and refuses before copying if the guard does
not intercept it. The 4× waiver and cleanup HOLD remain in effect.

Ops took one read-only filesystem census at 16:13 UTC on 2026-10-16. The daemon's in-process
`temp_store_directory` setting was not observable, so these same-device verdicts are
**conditional** on the temp path resolved by the gate at proof time. Free space also changes.

| Member | Configured scratch path | Free bytes | Capture admission required bytes | Point verdict |
| --- | --- | ---: | ---: | --- |
| linux-a | `$ST_STATE_DIR/checkpoint` | 239,060,426,752 | 178,518,466,560 | PASS, conditional |
| linux-b | `$ST_STATE_DIR/checkpoint` | 177,268,850,688 | 172,960,161,792 | PASS, conditional; 4.3 GB margin |
| mac-a | `$ST_STATE_DIR/checkpoint` | 381,645,103,104 | 163,008,110,592 | PASS, conditional |
| mac-b | `$ST_STATE_DIR/checkpoint` | 66,230,939,648 | 161,004,036,096 | REFUSE; 94.8 GB deficit |

## Staged binary capability for deployment

`CHECKPOINT_SPACE_GATE_VERSION` is the fixed integer `2`, exported from
`smallclaims::store` through `st3::store`. The staged `st3` binary answers
`--checkpoint-space-gate-capabilities` with exactly one JSON line:

```json
{"kind":"checkpoint-space-gate-capabilities","version":2}
```

The flag runs before instance activation, configuration, daemon connection or database access.
It exits successfully only when used alone; the integration test checks the JSON, a rejected
extra argument and an empty temporary HOME/XDG tree after both invocations. A deployment
manifest must bind the exact source commit, the staged binary's SHA-256, eligibility and gate
version. The worker compares the staged binary's answer to the source marker, manifest and
receipt version before a client mode or service switch. A matching JSON answer without that
source and artifact join is insufficient qualification. The worker owns its 5 second process
timeout and the final deployment decision.
