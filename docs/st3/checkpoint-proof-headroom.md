# Checkpoint proof scratch headroom

The checkpoint proof currently makes a full scratch copy of the live SQLite database on filesystems
without reflinks. On an ext4 member this uses `VACUUM INTO`; its preparation and comparison also
write a rollback journal and SQLite temporary files. A proof must leave capacity for the live
writer even when the scratch filesystem is full or the copy is interrupted. This is the first
storage change in the [checkpoint capture design](checkpoint-capture.md). A later change will
construct the proof database from only the sealed cut.

Before it creates a capture or proof scratch directory, the store reads allocated live DB bytes
(`st_blocks * 512`), logical DB length, free bytes on the scratch device (`statvfs`), and free
bytes on the live device. It resolves SQLite's temporary directory from the process-wide override
and the bundled Unix VFS search order (`SQLITE_TMPDIR`, `TMPDIR`, `/var/tmp`, `/usr/tmp`, `/tmp`,
current directory). If the temp device differs from scratch, it checks that device separately.
An existing staging file is already reflected in available space and is reported as allocated
scratch; the estimate covers new allocation.

The default live reserve is **8 GiB plus max(8 GiB, allocated live DB bytes)**. The second term is
the allowance for WAL growth while a proof runs. Ops owns any future increase; no configuration
change is part of this change. The scratch estimate uses the greater of live allocated bytes and
logical length for each of copy, journal, replay and sorter space; capture adds one more such term
for staging. Where devices differ, the sorter estimate is charged to SQLite's temp device. A
256 MiB reaction margin is added to each device. The gate reports available, required, deficit,
reserve and each estimated component. A host may refuse every proof under this conservative
policy; that is a capacity deficit to surface before restarting its checkpoint, not a proof pass.

For a proof, a uniquely named SQLite VFS wraps the platform default only on that proof's
connections. It charges each requested `xWrite` before forwarding it, refuses requests above
16 MiB, and refreshes `statvfs` and allocated-byte counts after at most 16 MiB of requested
writes. Its 256 MiB margin covers at most 16 MiB of writes between checks plus one accepted
16 MiB request, leaving 224 MiB for allocation rounding and concurrent activity. It guards
allocation-capable truncate, shared-memory map and file-size hints as well. Every guarded
connection sets `mmap_size=0`; an over-budget self-test write must be refused before the copy
starts. On refusal the connection closes without more SQL and the disposable proof and sidecars
are removed with checked errors. There is no atomic guarantee against another process consuming
the remaining margin between a check and a write. A project quota would supply that stronger
guarantee. `ENOSPC` is still an abort path, never a verification claim. A seal published before
the proof attempt remains in place.

Resumable capture staging keeps the default SQLite VFS and is charged one full live DB-size
allowance up front. Its short capture phases check available space between pages. A VFS refusal
inside its transaction could prevent rollback and leave an unusable staging file. A later PR
replaces the whole proof copy and puts `max_page_count` on construction.

One proof holds `proof.lock` in its scratch directory. It creates and syncs an owner marker before
the proof file, removes the copy and its journal, WAL and SHM sidecars with checked errors, then
removes the marker. Daemon startup and the next checkpoint pass sweep marked orphans only while holding the lock;
unknown unmarked proof files require inspection. Any removal error names the path and refuses the
next proof until the files are cleared. Resumable capture staging has a separate lifetime.

## Read-only host census before merge

Ops records one row for each checkpoint member (`hetz`, `hetz2`, `Silber`, `bluey`): live DB
allocated bytes, live WAL allocated bytes, available bytes and device IDs for live, scratch and
selected SQLite temp directories, then the gate's required bytes, deficit and verdict. This
requires `stat`/`statvfs` only; it does not make a production DB copy. The production DB size,
retained fraction, proof disk peak and elapsed time are still unknown until measured. The prior
2.68× RAW comment is not held evidence. The existing checkpoint heap test measures **MEMORY**,
not **DISK**. Linux/ext4 behavior is the local implementation target; APFS `f_bavail` semantics
and the guarded SQLite VFS on Silber and bluey remain UNKNOWN until tested there. Each proof
performs a deliberate over-budget self-test write and refuses before copying if the guard does
not intercept it. The 4× waiver and cleanup HOLD remain in effect.

Ops took one read-only filesystem census at 16:13 UTC on 2026-10-16. The daemon's in-process
`temp_store_directory` setting was not observable, so these same-device verdicts are
**conditional** on the temp path resolved by the gate at proof time. Free space also changes.

| Member | Configured scratch path | Free bytes | Capture admission required bytes | Point verdict |
| --- | --- | ---: | ---: | --- |
| hetz | `/home/myobie/.local/state/st3/checkpoint` | 239,060,426,752 | 178,518,466,560 | PASS, conditional |
| hetz2 | `/home/myobie/.local/state/st3/checkpoint` | 177,268,850,688 | 172,960,161,792 | PASS, conditional; 4.3 GB margin |
| Silber | `/Users/myobie/.local/state/st3/checkpoint` | 381,645,103,104 | 163,008,110,592 | PASS, conditional |
| bluey | `/Users/nathanherald/.local/state/st3/checkpoint` | 66,230,939,648 | 161,004,036,096 | REFUSE; 94.8 GB deficit |
