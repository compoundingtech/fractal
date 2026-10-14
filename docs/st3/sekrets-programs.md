# Run-only system services

A gateway can serve a root-enrolled Linux system process without an st daemon, a
seat or fleet membership. It authenticates the socket's kernel UID and the process's
exact systemd unit, and rechecks its process birth and placement before a request.
It reports a `program/HOST/NAME` caller, never the person who owns its grant.

`setup --person person/nathan --uid RUNNER_UID` alone does **not** enable this.
A person mapping requires a logind login scope; a system runner is not a person.
Do not map a runner UID as a person or give it profile management.

The operator installs a separate `sekrets` user/store/gateway and a pinned st binary
on ci1/ci2. NixOS installation belongs in the reviewed host module, not an ad hoc
execution of the Debian-style setup script. The following is the gateway config
portion; actual UID, unit, interpreter and script paths must come from that host's
reviewed installation. Do not copy example IDs to another host.

```toml
path = ["/nix/store/PYTHON-PIN/bin", "/nix/store/GIT-PIN/bin", "/nix/store/OPENSSH-PIN/bin"]
checkout_roots = ["/var/cache/ci1/work"]

[people]
"1000" = "person/nathan" # actual operator login UID

[programs."999"] # actual dedicated system UID, separate from every person
principal = "program/ci1/ci1-1"
person = "person/nathan"
unit = "github-runner-ci1-1.service"
command = ["python3", "/nix/store/SIGNER-PIN/evidence-sign"]

[programs."999".checkout]
backing_root = "/var/cache/ci1/work/ci1-1"
mount_root = "/w"
```

The entry applies only to `/system.slice/github-runner-ci1-1.service` and its
children. A login session, user-manager service, similar unit name, different UID
or reused/exited original process does not inherit this grant. UIDs must be
non-root and distinct from person mappings. Program names are literal three-part
identities; no wildcard program enrollment/grant exists.

`command` is the **entire** accepted argv. Added flags, another script, a shell,
login or profile-management request is refused. An explicit profile grant is also
required; mapping to Nathan does not give ownership of Nathan's profiles. Profile
and grant policies both apply, including expiry, revocation and locks. Program
callers cannot change profiles/keys/grants, make authorized HTTP requests or read
the person's audit log. The human reads `st sekrets log` from their login session;
allowed calls/refusals/exits retain the program identity in the gateway's local log,
without needing a daemon to mirror it.

From Nathan's actual login session after the reviewed installation:

```sh
st sekrets profile create nathan/evidence-sign \
  --allow 'python3 /nix/store/SIGNER-PIN/evidence-sign'
st sekrets grant nathan/evidence-sign --to program/ci1/ci1-1 \
  --allow 'python3 /nix/store/SIGNER-PIN/evidence-sign'
```

Operations provisions `.config/st/evidence-sign.json`, the key (0600), and the
private bare scratch repository **inside this profile's private HOME**, readable
only by sekrets. No key/config goes into a job, checkout, runner HOME or job env.
Pin the signer source/binary and interpreter via immutable root-owned Nix paths
and the install manifest; an argv policy alone is not an executable hash pin.
The gateway's tool lookup must select that reviewed interpreter. The signer must
be the hardened wrapper-owner implementation, not a caller-authored script.

The existing wrapper interface stays unchanged:

```sh
EVIDENCE_SIGNER='["st","sekrets","--profile","nathan/evidence-sign","--","python3","/nix/store/SIGNER-PIN/evidence-sign"]' \
  scripts/evidence-wrap ...
```

The wrapper sends its measured JSON on stdin. Sekrets passes stdin unchanged and
requires the authorized checkout; Program calls never fall back to profile HOME.
CI's `/w` is a private per-runner bind mount. The gateway takes the received
checkout descriptor, resolves its relative path under that unit's configured
backing root, and compares device/inode with a descriptor-relative, no-symlink
walk of that root. Wrong owner, outside root, symlink, missing path or mismatched
mount refuses the call. Only the received checkout is bound read-only at its
logical `/w/...` path inside the gateway's protected execution namespace. All
configured backing roots and mount anchors are otherwise hidden. No runner unit
BindPaths, WorkingDirectory or restart is required by this source algorithm.

Operations must provide a root-controlled backing-root parent, a runner-owned
backing root, search-only ACLs for sekrets on the root/ancestors, and an existing
root-controlled empty `/w` mount anchor on the gateway host. The anchor contains
no job data and is hidden in the sandbox before the actual directory is bound.
The gateway never changes into its host's `/w` to locate a checkout. Nested
checkout directories must also be searchable; an inaccessible directory refuses.
Linked worktrees whose Git directories escape this runner root refuse too.
The new standalone NixOS gateway module and actual service/user/mount acceptance
are separate installation inputs; no no-restart deployment has been qualified.

The interpreter and absolute fixed argv inputs are checked as root-controlled
paths before execution. The signer derives
the requested commit's tree and lock digest from the bound checkout, validates
its structured input and constructs the statement itself; it must not accept
arbitrary bytes, caller-selected key/config, Git executable overrides or a clock
override in production. No GitHub observation API is needed for these local facts.

Every process inside the enrolled runner unit is the same Program caller,
including code the job executes. The gateway identifies the runner, not the job,
and `exit_code` and `output_sha256` are caller-supplied. A note signed through this
route means “a job on this runner reported this result”; it does not prove the
check ran or that a caller-provided commit is the actual PR head. Keep this
Program signer at a distinct, weaker signer principal/level than the central
protected observer in `.st/evidence-keys`. A Program-signed `wrapper` note must
never satisfy an `observed` skip policy. The verifier owner must test this
negative boundary; this gateway does not implement skip decisions.

Do not grant the signer to a unit that runs untrusted fork or unreviewed PR code.
First installation must use the reviewed merge-group-only runner unit and its
actual routing restrictions. A shared runner UID alone cannot distinguish jobs;
per-job identity or a protected measurement producer is a separate prerequisite
before expanding enrollment. This isolates key access within that scope. Verifier policy
must compare exact tree/check/input keys and signer role. Developer/cooperative
trust is a **different principal/level**, not an invented `cooperative` value in
the existing `full/degraded/test` schema. The service never decides whether CI is
skipped. Keep PR and landing policy separate.

Authored source tests and isolation VM controls cover unit/UID classification, exact argv,
management refusal, grant/revocation, audit and store/key unreadability before
any st daemon starts, including private-mount binding, read-only checkout and stdin.
They must pass before installation; they have not been executed for this source
pin yet. Those controls do not claim that ci1/ci2 are installed or a
real merge-group note is accepted. Record the actual image, config, grant, run,
attempt, tree, signed note and verifier job before switching consumers/removing
the obsolete job-visible key. Namespace's remote signing path and the developer
Fabric service are subsequent steps of the same evidence-signing-service mission.

Existing people/agent/host callers keep their behavior. Upgrade the gateway before
adding `[programs]`: older gateways reject the unknown config and caller kind.
Rollback removes program enrollment/grants first; never convert a program into a
person mapping. No st graph/schema/replay or gateway DB migration is introduced.
