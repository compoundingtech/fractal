"""Signal discovered Linux fixture processes only through verified pidfds.

PIDs from files, /proc scans and old receipts are hints, never signal authority.
Open the pidfd first, then recheck the private fixture identity before signalling.
Direct, unreaped subprocess children can instead use their Popen methods.
"""
import argparse
import os
from pathlib import Path
import signal
import sys


def require_linux():
    if sys.platform != 'linux' or not hasattr(os, 'pidfd_open'):
        raise RuntimeError('Linux only: fixture signalling requires /proc and pidfds')


def ancestors():
    result = set()
    pid = os.getpid()
    while pid > 0 and pid not in result:
        result.add(pid)
        try:
            pid = int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            break
    return result


def signal_private(pid, sig, *, endpoint=None, root=None, environment=None,
                   start_ticks=None, contains=None, parent=None, process_group=None):
    """Return False for a vanished/mismatched target; never fall back to kill(pid).

    An exact private endpoint, private root or environment marker is mandatory.
    start_ticks is an additional receipt binding, not a substitute for ownership.
    """
    require_linux()
    if parent is not None and parent <= 1:
        raise ValueError('a parent selector must be greater than 1')
    if not (endpoint or root or environment or parent):
        raise ValueError("a private fixture identity is required")
    if pid <= 1 or pid in ancestors():
        return False
    try:
        fd = os.pidfd_open(pid)
    except (ProcessLookupError, PermissionError):
        return False
    try:
        # A non-dumpable same-uid process can deny these reads. An unreadable
        # identity is not ours to signal, but must not abort later fixture cleanup.
        # Scope this catch to metadata, not resource exhaustion or invalid signals.
        try:
            if not matches_private_process(pid, endpoint, root, environment,
                                           start_ticks, contains, parent, process_group):
                return False
        except (OSError, ValueError, IndexError):
            return False
        # The numeric PID can exit/recycle after the checks; the fd still names
        # only the process opened above. A replacement cannot receive the signal.
        signal.pidfd_send_signal(fd, sig)
        return True
    except (ProcessLookupError, PermissionError):
        return False
    finally:
        os.close(fd)


def matches_private_process(pid, endpoint, root, environment, start_ticks, contains, parent, process_group):
    proc = Path(f"/proc/{pid}")
    if proc.stat().st_uid != os.getuid():
        return False
    stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
    if start_ticks is not None and int(stat[19]) != start_ticks:
        return False
    if parent is not None and int(stat[1]) != parent:
        return False
    if process_group is not None and int(stat[2]) != process_group:
        return False
    env = dict(item.split(b"=", 1) for item in (proc / "environ").read_bytes().split(b"\0") if b"=" in item)
    if endpoint is not None:
        preferred, legacy = env.get(b"FRACTAL_ENDPOINT"), env.get(b"ST3_ENDPOINT")
        if preferred is not None and legacy is not None and preferred != legacy:
            return False
        if (preferred if preferred is not None else legacy) != os.fsencode(endpoint):
            return False
    if environment and any(env.get(os.fsencode(k)) != os.fsencode(v) for k, v in environment.items()):
        return False
    argv = (proc / "cmdline").read_bytes().split(b"\0")
    if contains is not None and os.fsencode(contains) not in b" ".join(argv):
        return False
    if root is not None:
        root = Path(root).resolve()
        cwd = (proc / "cwd").resolve(strict=True)
        in_root = cwd == root or root in cwd.parents
        in_root |= any(arg.startswith(os.fsencode(root / "bin") + b"/") for arg in argv)
        if not in_root:
            return False
    return True


def main():
    try:
        require_linux()
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 2
    if sys.argv[1:] == ['--check-platform']:
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", help="PID, or 'all' to inspect the explicitly selected fixture")
    parser.add_argument("signal", type=int)
    parser.add_argument("--endpoint")
    parser.add_argument("--root")
    parser.add_argument("--env", action="append", default=[])
    parser.add_argument("--contains")
    parser.add_argument("--parent", type=int)
    args = parser.parse_args()
    if args.parent is not None and args.parent <= 1:
        parser.error('a parent selector must be greater than 1')
    pids = (int(proc.name) for proc in Path('/proc').glob('[0-9]*')) if args.pid == 'all' else [int(args.pid)]
    for pid in pids:
        try:
            signal_private(pid, args.signal, endpoint=args.endpoint, root=args.root,
                           environment=dict(value.split("=", 1) for value in args.env),
                           contains=args.contains, parent=args.parent)
        except (ProcessLookupError, PermissionError):
            # One inaccessible/vanished entry cannot terminate an all-PID sweep.
            continue
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
