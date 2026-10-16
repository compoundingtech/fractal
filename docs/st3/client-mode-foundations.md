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

Directory traversal opens each component relative to the previous descriptor using `O_DIRECTORY|O_NOFOLLOW` and checks the opened directory's owner/mode. Ancestors must belong to root or the effective account and be non-writable by others. A root-owned sticky temporary ancestor permits private account-owned children: other UIDs cannot replace those child entries. On macOS only the fixed root-owned aliases `/var -> /private/var` and `/tmp -> /private/tmp` are translated before descriptor traversal; other symlinks are refused. Immediate state and marker files are private owned regular files opened with `O_NOFOLLOW`. No arbitrary canonicalization fallback is used. Producers and status both ignore HOME/XDG for authority and coordination, always using the effective account NSS home inode. A temporary HOME can select test configuration but cannot select another mode or lock. Hidden/unopenable account homes refuse. Linux supports ext4, XFS, Btrfs, tmpfs and overlay filesystems; macOS is restricted to APFS, pending native qualification. Network/FUSE/unknown filesystems refuse. Hostile same-UID namespace substitution is not detected without an independent host anchor. Native macOS alias behavior and replacement-race tests remain unqualified.

This guards accidental/environmental bypass and inconsistent retained enrollment evidence. It does not promise protection against a hostile process with the same UID. In particular, loss of **all** durable enrollment/mode/recovery evidence cannot be distinguished from a never-enrolled account without an independent anchor. Such an anchor is not implemented.

Custody is separately gated: authenticated fresh payload-present proof at a fixed fenced cut; complete local-origin history and dependency closure; durable peer retention; bounded verification; exact daemon and separate worker drain/stop plus supervisor behavior; recovery and reversible routing/authentication. UNKNOWN refuses. Storage policy consent, exact-head source review, signed test evidence, native/service qualification and Nathan's chosen flip moment are separate gates.

macOS authority is explicitly unavailable in this draft: POSIX ownership/mode bits do not rule out independent allow ACLs. Opened-descriptor ACL validation and native qualification must precede enabling it. This also prevents legacy Mac startup through these guarded paths; the draft is not a deployable upgrade. No warning-only rollout or production enforcement is authorized by this code.

The lease binding in `run_up`/`run_worker` lasts through the async body. Fallible returns and detached tasks mean this alone does not prove that every process-wide writer has ended before the lease drops. Drain/join of every producer is still a prerequisite to enrollment/ENTER; no such transition is exposed.

Before any release, qualify the compatibility and rollout matrix below. This draft has no observe-only doctor rollout; it must not be deployed as that stage. A reviewed observe-only doctor/event stage must establish real host verdicts before an enforcing release. Downgrading this foundation needs no state cleanup because no production enrollment command exists. There is no bypass flag for an unsafe anchor.

| Caller/environment | Current behavior and remaining qualification |
| --- | --- |
| `run_up` | Takes the shared lease before configuration and retains it through its async body; detached-writer closure remains unproved |
| Separate replication-worker dispatch | Opens its own shared lease before configuration and retains it through `run_worker().await` |
| Temporary-HOME test or secondary daemon | Uses the real NSS account authority/lease; starts while that account is Local, and is inhibited if that account is in Client/Transition mode |
| Named instance | Uses an isolated state namespace but the same conservative account-wide coordination inode |
| Nix development shell | Mandatory NSS anchor still applies; changing HOME does not redirect it |
| Nix build sandbox/CI with absent, root-owned or hidden NSS home | Refuses visibly; sandbox daemon checks need explicit qualification before release, with no environment bypass |
| Linux service/CI runner | Requires trusted existing account anchor and supported local filesystem; fleet-wide ownership/ACL/filesystem inventory is not complete |
| macOS launchd/child shells | Unavailable until ACL-aware validation and native behavior are qualified; no Mac release |

Operations must obtain approved read-only filesystem/owner/mode/ACL facts for every rollout host and qualify process/child inheritance and supervisor behavior. The earlier Bluey label inventory does not authorize new reads or establish those facts. No transition is exposed until full admission fencing, drain/join, recovery and exact service/worker handling are complete.
