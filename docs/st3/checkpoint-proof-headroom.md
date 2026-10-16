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

While SQLite executes a copy, capture or proof, an activity-driven progress handler checks after
at most 1,000 virtual-machine opcodes. It interrupts if available space would cross the reserve
and margin, the new scratch allocation exceeds the admitted budget, or live WAL growth uses its
allowance. This is a bounded reaction point, not an atomic filesystem quota. A separate PR will
replace the whole copy and put `max_page_count` on construction. `ENOSPC` remains an error path;
it never publishes verification. A seal published before this proof attempt remains in place.

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
not **DISK**. The 4× waiver and cleanup HOLD remain in effect.
