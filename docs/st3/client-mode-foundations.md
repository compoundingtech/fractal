# Client-mode foundations (draft)

`st client-mode status` reads nonsecret local authority without contacting a daemon or creating state. This foundation does not implement ENTER, EXIT, recovery, remote routing, custody or credential storage. Do not use it to switch a production machine. A manually written transition can inhibit local commands without a recovery command.

The authority path is `<passwd-home>/.st3-client-mode/default/client-mode.json`, independently of `HOME` and `XDG_CONFIG_HOME`. Named instances use `.st3-client-mode/instances/<name>/client-mode.json`, so an instance named `default` cannot alias the default namespace. The dedicated directory avoids group-writable shared `.local` ancestors. Any retained mode, marker or recovery artifact at the earlier draft location requires explicit recovery; it is never silently migrated or ignored. Named instances are separate explicit namespaces; alternate database or endpoint paths must not be treated as a way to bypass a future transition guard. Database aliases and every producer still require qualification before ENTER.

A never-enrolled account is uninitialized Local. The persistent `.lock` file is enrollment evidence; the already-existing trusted passwd-home directory inode is the account-wide coordination object. Startup/status do not create synchronization infrastructure. Enrollment fsyncs its marker and parent before publishing a generation or permitting transition effects. State publication is a private temporary file, file fsync, atomic rename and directory fsync. There is no production state writer in this foundation.

| Enrollment marker | Mode file | Coordination lease | Result |
| --- | --- | --- | --- |
| Absent | Absent | Free | Uninitialized Local; startup takes a shared lease before reading |
| Absent | Absent | Exclusive | Startup refuses; enrollment may be beginning |
| Present | Absent | Any | Refuse and require recovery, including interrupted first enrollment |
| Absent | Present | Any | Inconsistent; refuse and require recovery |
| Present | Invalid, unreadable or unsupported | Any | Refuse and require recovery |
| Present | Valid Local | Shared or free | Startup can take its own shared lease and proceed |
| Present | Valid Transition or Client | Any | Local startup refuses |
| Any | Any | Conflicting lease | Refuse immediately; no polling |

The marker currently records presence, not a versioned operation journal. Filesystem modification times do not establish generation order. Versioned recovery journals, marker/state generation binding, holder PID/start identity diagnostics and a complete artifact consistency table are required before exposing transition/recovery commands. No recovery artifact is written by this foundation.

Daemon startup and the separate replication worker each hold their own shared `flock` lease until their async run finishes. Publication first takes an exclusive lease on the same passwd-home directory inode. Every named instance shares this conservative account-wide lock. Kernel locks disappear when their last open description closes; stale file contents are not a lease. Descriptors are close-on-exec, so exec'd seats/helpers do not retain the lease. Fork-only children require an explicit descriptor-lifetime audit before ENTER; a future transition must drain both producers and perform an explicit daemon-mediated lease handoff. A successful startup check alone is not a durable writer fence across CLI, API, hooks, drivers and supervisors.

Directory traversal opens each component relative to the previous descriptor using `O_DIRECTORY|O_NOFOLLOW` and checks the opened directory's owner/mode. Ancestors must belong to root or the effective account and be non-writable by others. A root-owned sticky temporary ancestor permits private account-owned children: other UIDs cannot replace those child entries. On macOS only the fixed root-owned aliases `/var -> /private/var` and `/tmp -> /private/tmp` are translated before descriptor traversal; other symlinks are refused. Immediate state and marker files are private owned regular files opened with `O_NOFOLLOW`. No arbitrary canonicalization fallback is used. Producers reject HOME when it resolves to another device/inode; status always uses NSS directly. Hidden/unopenable account homes refuse. Linux supports ext4, XFS, Btrfs, tmpfs and overlay filesystems; macOS is restricted to APFS, pending native qualification. Network/FUSE/unknown filesystems refuse. Hostile same-UID namespace substitution is not detected without an independent host anchor. Native macOS alias behavior and replacement-race tests remain unqualified.

This guards accidental/environmental bypass and inconsistent retained enrollment evidence. It does not promise protection against a hostile process with the same UID. In particular, loss of **all** durable enrollment/mode/recovery evidence cannot be distinguished from a never-enrolled account without an independent anchor. Such an anchor is not implemented.

Custody is separately gated: authenticated fresh payload-present proof at a fixed fenced cut; complete local-origin history and dependency closure; durable peer retention; bounded verification; exact daemon and separate worker drain/stop plus supervisor behavior; recovery and reversible routing/authentication. UNKNOWN refuses. Storage policy consent, exact-head source review, signed test evidence, native/service qualification and Nathan's chosen flip moment are separate gates.
