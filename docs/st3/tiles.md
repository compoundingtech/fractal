# Tiles

A tile is one agent as its own Unix user, in its own namespaces and its own root-owned cgroup.
Another tile cannot read its files, see or signal its processes, reach its sockets or loopback
ports, or pass for it. A tile is a process group and private directories on the host's kernel,
not a VM and not a full container image. Linux first; the approved design is the st document
`doc/fleet/smalltalk/agent-tiles/design`.

This document describes what is built: the first step of that design, launching a tile on
synthetic accounts. Harness integration, the credential compartment, signed handoffs and
network egress come next and are listed at the end.

## Registry

Root's registry, `/etc/st-tiles/tiles.toml`, is the only source of a tile's identity. Root owns
the file and every directory above it; st refuses one anyone else can change.

```toml
state_root = "/var/lib/st-tiles"      # STATE/NAME/home and STATE/NAME/work, the tile's own
runtime_root = "/run/st-tiles"        # RUNTIME/NAME/endpoint and the tile's root mount point
read_only = ["/nix/store/...-tools"]  # root-controlled closures every tile sees read-only
path = ["/nix/store/...-tools/bin"]   # PATH inside a tile

[tiles.atlas]
uid = 61001                           # a system user with no login home, never a person's
gid = 61001
agent = "agent/fleet/fixture-example/atlas"
person = "person/ada"
supervisor_uid = 1000                 # whose st daemon serves the tile's endpoint
tasks_max = 512                       # ceilings, not reservations
memory_max = "8G"
cpu_weight = 100
```

A tile's name is up to 32 of `a-z`, `0-9` and `_`. Two tiles never share a Unix user, a tile is
never root, never a person and never another tile's supervisor, and a read-only path never
overlaps a tile's private state. Never reuse a tile's Unix user for anything else while its
files, processes or grants remain: st refuses state directories owned by another user.

A process is a tile's when the kernel says it runs as the tile's Unix user inside the tile's
slice, `/st_tiles.slice/st_tiles-NAME.slice/`, which only root can place it in
(`Registry::identify`). What a tile says about itself, such as `ST_AGENT` or `--as`, counts for
nothing.

## Launching

```sh
sudo st tiles run atlas -- python3 -c 'print("hello")'   # wait for it; --pty for a terminal
sudo st tiles stop atlas                                  # end every process in it
```

`st tiles run` starts `st-tile-NAME-tools.service` in `st_tiles-NAME.slice` with the tile's
ceilings (`TasksMax`, `MemoryMax`, optional `MemoryHigh`, `CPUWeight`) and
`KillMode=control-group`. Only one runs per tile. systemd owns the cgroup: nothing is delegated,
so a tile cannot write to it, and the unit ends when its main process ends, for any reason. The
st executable runs as root in that unit, so root must control it and every directory above it;
install st where only root can change it.

The unit's main process, `st tiles enter`, checks it runs in that unit, makes new mount, PID,
IPC, network, UTS and cgroup namespaces, and forks the tile's first process, PID 1 of the new
PID namespace. PID 1 builds the tile's file system on an empty tmpfs:

| Path | What it is |
|---|---|
| each `read_only` path | bound read-only, without its submounts, no set-user-ID |
| `/tile/home`, `/tile/work` | `STATE/NAME/home` and `work`, owned by the tile, mode 0700 |
| `/tmp`, `/dev/shm` | private tmpfs |
| `/proc` | a fresh proc of the tile's PID namespace |
| `/dev` | `null`, `zero`, `full`, `random`, `urandom`, `tty`, a private `pts` |
| `/run/st` | the tile's endpoint directory, read-only |
| `/etc` | `passwd`, `group` and `hostname` for the tile alone |

Nothing else of the host is there: no `/sys`, no homes, no `/run/user`, no host sockets, no
`/var`. The root itself is read-only. The network namespace has only its loopback, so a tile's
own servers listen there and nothing else is reachable; abstract Unix sockets belong to it too.

PID 1 then closes every descriptor but standard input, output and error, drops the capability
bounding and ambient sets and its supplementary groups, becomes the tile's user and group, sets
no-new-privileges and installs a seccomp filter. The filter refuses `unshare`, `setns`, clone
with a namespace flag, `mount`, `umount2`, `pivot_root`, `chroot` and the new mount API with
EPERM, refuses `clone3` with ENOSYS (libc falls back to clone, whose flags the filter reads), and
ends a process that calls through another architecture's table. PID 1 forks the command with a
clean environment (`HOME`, `USER`, `PATH`, `TMPDIR`, `ST_TILE`, `ST_AGENT`, `ST_TILE_ENDPOINT`,
`TERM`), every signal at its default, then reaps until the command exits, and exits with its
status.

## Ending

Every way a tile's launch ends leaves nothing behind. When the command exits, PID 1 exits and the
kernel ends every process left in the PID namespace, including ones that ignore SIGTERM or
double-forked into their own sessions. When PID 1 is killed, the same. When `st tiles enter` is
killed, PID 1 gets SIGKILL from its parent-death signal, and systemd stops the unit and kills
its cgroup. `st tiles stop` does the latter directly. In each case the unit and its cgroup are
gone when `st tiles run` returns, no process of the tile's user remains on the host, and the
tile's mounts, which only ever existed in its own mount namespace, are gone with it. The tile
starts again cleanly.

## Endpoint

`RUNTIME/NAME/endpoint` is owned by the tile's supervisor and bound read-only at `/run/st`. It is
the only host directory where a tile can reach a socket. A server there learns the caller from
the kernel: `SO_PEERCRED` gives the Unix user and process, and the process's cgroup and the
registry give the tile and its agent. The restricted st endpoint and sekrets' tile binding build
on this.

## What it proves

`crates/st3/tests/tiles_root.rs` runs on a real host with `sudo -n`, systemd and cgroups v2,
creating disposable system users and removing them, every unit and every directory afterwards:

```sh
ST_TILES_ROOT_TEST=1 cargo test -p st3 --test integration tiles_root:: -- --nocapture
```

- Identity: the command runs as the tile's user with no groups, no capability in any set,
  no-new-privileges and the filter, sees only its own processes, its own root and `lo`, and
  only standard streams are open.
- Siblings: a second tile cannot reach the first's abstract socket, pathname socket, loopback
  port or System V shared memory, see its files or processes, or signal any host process; the
  host's own loopback port and abstract socket are unreachable too. Outside the namespaces, one
  tile's user cannot read another's state.
- Keys: a node key written as st writes it, and every path of the supervisor's state, are
  unreadable by the tile's user and absent inside the tile.
- Escape: every namespace flag, `setns`, mounts and `chroot` are refused; `fork` works until
  `TasksMax`; the tile's user cannot move a process into its own or a sibling's cgroup.
- Descriptors: a descriptor root's shell left open across `st tiles run` is not in the tile.
- Cleanup: killing the launcher, killing PID 1, stopping the unit and the command exiting each
  leave no unit, cgroup, process or mount, and the tile starts again.
- Refusals: a non-root caller, `enter` outside its unit, a registry or st executable someone
  other than root can change.

## Not built yet

- The restricted st endpoint and sekrets' identity binding for tile callers.
- The credential compartment and one harness running in a tile (design step 2).
- Network egress: a tile has no route off its loopback until a root-owned veth, route and
  firewall policy is added.
- Landlock and an AppArmor or SELinux profile on top of the namespaces.
- macOS.
