#!/usr/bin/env python3
"""Own an isolated test's entire process tree, including double-forked PTY servers.

The guardian detaches from seat ancestry but retains pidfds for both its launcher and
the test runner. It is a Linux subreaper: orphaned descendants become its children,
so cleanup needs neither a live daemon API nor a scan of unrelated fleet processes.

With --pid-namespace, where unprivileged user and PID namespaces are available, the guardian
runs as PID 2 of a fresh PID namespace under a minimal PID 1. Detached from the seat, it would otherwise be
adopted by the nearest subreaper, which on a seat host is the same-uid, non-dumpable
`systemd --user`: a daemon's caller-ancestry walk then fails reading that manager's
environment and refuses person commands. Inside the namespace the walk ends at PID 1, as it
does on a CI runner or over SSH, where PID 1 adopts the orphan. The walk itself stays strict.
Only a single test body asks for it (`supervise_test`): host-wide tools that coordinate by PID,
such as the cargo shim's queue, cannot see out of a PID namespace, so a stage or script that may
run cargo is supervised as before. Without the flag, where namespaces are unavailable, or with
FRACTAL_TEST_PID_NAMESPACE=off, the guardian runs as before.
"""
import argparse
import ctypes
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def reap_exited_descendants(task_pid):
    """Retire adopted PTYs during the test, preserving the task's exit status.

    A stopped PTY must disappear from /proc before suspend/resume checks continue.
    Only wait on our direct children; leave live descendants owned until teardown.
    """
    children = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children")
    for pid in map(int, children.read_text().split()):
        if pid != task_pid:
            try:
                os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                # A SIGCHLD callback can retire the same adopted child while a
                # preceding child notification is being handled.
                pass


def reap_descendants(released=frozenset()):
    """Kill before reaping, keeping each PID reserved while signalling its group.

    Killing an adopted parent can adopt more children (even ones in new sessions).
    Repeat until waitpid confirms we have no children at all, or only `released`
    ones: those keep running and pass to the next subreaper when this guardian exits.
    """
    children = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children")
    while True:
        owned = [pid for pid in map(int, children.read_text().split()) if pid not in released]
        for pid in owned:
            kill_group(pid)
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            while os.waitpid(-1, os.WNOHANG)[0]:
                pass
        except ChildProcessError:
            return
        if not owned:
            return
        time.sleep(0.01)


def read_released(control):
    """The PIDs, four bytes each, that the owner sent before closing its end of the socket."""
    message = b""
    while chunk := os.read(control, 4096):
        message += chunk
    return frozenset(int.from_bytes(message[i:i + 4], "big") for i in range(0, len(message), 4))


CLONE_NEWNS = 0x00020000
CLONE_NEWUSER = 0x10000000
CLONE_NEWPID = 0x20000000
MS_NOSUID, MS_NODEV, MS_NOEXEC, MS_REC, MS_PRIVATE = 0x2, 0x4, 0x8, 0x4000, 0x40000


def enter_namespaces():
    """Move into fresh user, PID and mount namespaces, keeping this uid and gid.

    Only this process's later children join the PID namespace; the first becomes its PID 1.
    """
    uid, gid = os.getuid(), os.getgid()
    os.unshare(CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNS)
    Path("/proc/self/setgroups").write_text("deny")
    Path("/proc/self/uid_map").write_text(f"{uid} {uid} 1")
    Path("/proc/self/gid_map").write_text(f"{gid} {gid} 1")


def drop_namespace_capabilities():
    """Leave the guardian as readable as the test it runs.

    Entering the user namespace granted every capability in it, and the kernel refuses to
    show /proc environ of a process whose capabilities exceed the reader's, so the walk
    could not read the guardian. Each credential change also clears dumpable, which only an
    exec would restore; the guardian forks without exec, so set it back.
    """
    libc = ctypes.CDLL(None, use_errno=True)
    header = (ctypes.c_uint32 * 2)(0x20080522, 0)  # _LINUX_CAPABILITY_VERSION_3, this thread
    data = (ctypes.c_uint32 * 6)()  # effective, permitted, inheritable: twice 32 bits, all clear
    if libc.capset(header, data) != 0:
        raise OSError(ctypes.get_errno(), "capset")
    if libc.prctl(4, 1, 0, 0, 0) != 0:  # PR_SET_DUMPABLE
        raise OSError(ctypes.get_errno(), "PR_SET_DUMPABLE")


def mount_namespace_proc():
    """Give the namespace its own /proc, so PIDs read there are the namespace's PIDs."""
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.mount(None, b"/", None, MS_REC | MS_PRIVATE, None) != 0:
        raise OSError(ctypes.get_errno(), "make mounts private in the test's namespace")
    if libc.mount(b"proc", b"/proc", b"proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, None) != 0:
        raise OSError(ctypes.get_errno(), "mount /proc in the test's PID namespace")


def pid_namespace_available():
    """Whether a throwaway child can build the whole namespace, /proc included.

    CI runners and hardened kernels refuse some step (unshare, the id maps, or the mount);
    any refusal keeps today's path. A child probes, so a refusal leaves nothing behind.
    """
    if os.environ.get("FRACTAL_TEST_PID_NAMESPACE") == "off" or not hasattr(os, "unshare"):
        return False
    probe = os.fork()
    if probe == 0:
        code = 1
        try:
            enter_namespaces()
            init = os.fork()
            if init == 0:
                try:
                    mount_namespace_proc()
                    os._exit(0 if os.getpid() == 1 else 1)
                except BaseException:
                    os._exit(1)
            code = os.waitstatus_to_exitcode(os.waitpid(init, 0)[1])
        except BaseException:
            pass
        finally:
            os._exit(code)
    return os.waitstatus_to_exitcode(os.waitpid(probe, 0)[1]) == 0


def namespace_init(run_guardian, result):
    """PID 1 of the test's namespace: start the guardian as PID 2 and reap until it exits.

    When this exits the kernel kills whatever is left in the namespace, so nothing outlives it.
    """
    mount_namespace_proc()
    guardian_pid = os.fork()
    if guardian_pid == 0:
        try:
            drop_namespace_capabilities()
            run_guardian()
        except BaseException as error:
            print(f"isolated test guardian: {error}", file=sys.stderr)
        finally:
            os._exit(0)
    # Only the guardian reports the result; the launcher reads it once the guardian is gone.
    os.close(result)
    while os.waitpid(-1, 0)[0] != guardian_pid:
        pass


def namespace_holders(namespace):
    """Live processes, seen from here, in the given PID namespace (as /proc/PID/ns/pid reads)."""
    holders = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                if os.readlink(entry / "ns/pid") == namespace:
                    holders.append(int(entry.name))
            except OSError:
                pass
    return holders


def set_child_subreaper():
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")


def guardian(command, watched, result, temporary_roots=(), keep=False, control=None):
    # PR_SET_CHILD_SUBREAPER, before launching any descendants.
    set_child_subreaper()
    interrupted = False

    def interrupt(signum, frame):
        nonlocal interrupted
        interrupted = True

    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, interrupt)
    task = None
    released = frozenset()
    previous_sigchld = None
    roots = []
    evidence_manifest = (tempfile.NamedTemporaryFile(prefix="test-evidence-", mode="w+")
                         if control is None else None)
    code = 1
    try:
        env = dict(os.environ)
        if evidence_manifest is not None:
            env.update(FRACTAL_TEST_SUPERVISOR=str(os.getpid()),
                       FRACTAL_TEST_EVIDENCE_MANIFEST=evidence_manifest.name)
        # Allocate under the guardian, not the killable controller. Remove only
        # these freshly created roots, and only after their processes are reaped.
        for name, prefix, parent in temporary_roots:
            root = tempfile.mkdtemp(prefix=prefix, dir=parent)
            roots.append(root)
            env[name] = root
            if keep:
                print(f"isolated test keeping {root}", file=sys.stderr, flush=True)
        task = subprocess.Popen(command, env=env, start_new_session=True,
                                **({"stdin": subprocess.DEVNULL} if control is not None else {}))
        if control is not None:
            # A duplex inherited socket reports the real daemon PID. Its EOF ends the
            # scope: after the PIDs the owner releases, or empty if the Rust owner unwinds.
            os.write(control, task.pid.to_bytes(4, "big"))
        # Reap adopted exits when SIGCHLD arrives, rather than leaving zombies
        # visible until the owner-watch timer. PTY lifecycle checks can launch
        # their next --once pass immediately after killing a detached process.
        previous_sigchld = signal.getsignal(signal.SIGCHLD)
        signal.signal(signal.SIGCHLD, lambda signum, frame: reap_exited_descendants(task.pid))
        reap_exited_descendants(task.pid)
        with os.fdopen(os.pidfd_open(task.pid), "rb") as task_fd:
            poller = select.poll()
            for fd in [*watched, task_fd.fileno(), *([control] if control is not None else [])]:
                poller.register(fd, select.POLLIN)
            # pidfds avoid PID reuse and the check/launch race. An already dead
            # owner is immediately readable, including after the double fork.
            ready = []
            while not interrupted and not (ready := poller.poll(100)):
                pass
            if any(fd == control for fd, _ in ready):
                released = read_released(control)
    finally:
        if previous_sigchld is not None:
            signal.signal(signal.SIGCHLD, previous_sigchld)
        if task is not None:
            kill_group(task.pid)
            code = task.wait()
        reap_descendants(released)
        if evidence_manifest is not None:
            from fractal_test_evidence import finish
            try:
                finish(Path(evidence_manifest.name), code)
            except Exception as error:
                print(f"isolated test evidence capture: {error}", file=sys.stderr)
                code = code or 1
            finally:
                evidence_manifest.close()
        if not keep:
            for root in roots:
                shutil.rmtree(root)
        # Return status only AFTER every daemon, worker, driver and PTY is gone.
        if control is None:
            os.write(result, str(code).encode())
    return code


def run_supervised(command, owners=None, temporary_roots=(), keep=False, pid_namespace=False):
    owners = owners or [os.getppid()]
    if "ST_AGENT" in os.environ or "ST3_SUBJECT" in os.environ:
        # /proc ancestry reads the process's original environment. Removing an
        # os.environ key after fork does not erase it there; exec a clean launcher
        # before detaching, while retaining watches for both original callers.
        env = dict(os.environ)
        for name in ("ST_AGENT", "ST3_SUBJECT"):
            env.pop(name, None)
        owner_args = [arg for pid in [*owners, os.getpid()] for arg in ("--owner", str(pid))]
        root_args = [arg for root in temporary_roots for arg in ("--temporary-root", *map(str, root))]
        return subprocess.call([sys.executable, str(Path(__file__).resolve()),
                                *owner_args, *root_args, *(["--keep"] if keep else []),
                                *(["--pid-namespace"] if pid_namespace else []),
                                "--", *command], env=env)
    # Open both before forking. The launcher waits for the guardian's result;
    # killing either the launcher or its Rust test runner cancels the whole tree.
    watched = [os.pidfd_open(pid) for pid in [*owners, os.getpid()]]
    namespaced = pid_namespace and pid_namespace_available()
    reader, writer = os.pipe()
    intermediate = os.fork()
    if intermediate == 0:
        try:
            os.close(reader)
            os.setsid()
            if namespaced:
                enter_namespaces()
            if os.fork() != 0:
                os._exit(0)
            if namespaced:
                namespace_init(lambda: guardian(command, watched, writer, temporary_roots, keep), writer)
            else:
                guardian(command, watched, writer, temporary_roots, keep)
        except BaseException as error:
            print(f"isolated test guardian: {error}", file=sys.stderr)
        finally:
            os._exit(0)
    os.close(writer)
    for fd in watched:
        os.close(fd)
    os.waitpid(intermediate, 0)
    try:
        status = os.read(reader, 32)
    finally:
        os.close(reader)
    code = int(status) if status else 1
    return code if code >= 0 else 128 - code


def supervised_main(main):
    """Also protect fixtures launched directly, outside Cargo/nextest."""
    if supervisor_active():
        return main()
    # Fixtures such as delivery-probe consume --binary/--scratch from sys.argv
    # before their entrypoint. Re-exec the original invocation, preserving those
    # options (and interpreter flags), so native proofs cannot silently skip.
    return run_supervised([sys.executable, *sys.orig_argv[1:]])


def supervisor_active():
    """A stale inherited marker must not disable supervision on a later run."""
    from script_process import ancestors
    try:
        supervisor = int(os.environ.get("FRACTAL_TEST_SUPERVISOR", "0"))
    except ValueError:
        return False
    return supervisor > 1 and supervisor != os.getpid() and supervisor in ancestors()


if __name__ == "__main__":
    if sys.argv[1:] == ["--is-supervised"]:
        raise SystemExit(0 if supervisor_active() else 1)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner", type=int, action="append")
    parser.add_argument("--temporary-root", nargs=3, action="append", default=[],
                        metavar=("ENV", "PREFIX", "PARENT"))
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--pid-namespace", action="store_true",
                        help="run the guardian in a fresh PID namespace where available (a single test body only)")
    parser.add_argument("--scope", action="store_true",
                        help="foreground descendant scope, with a duplex control socket on stdin")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("a command is required")
    if args.scope:
        watched = [os.pidfd_open(pid) for pid in args.owner or [os.getppid()]]
        code = guardian(command, watched, 0, control=0)
        raise SystemExit(code if code >= 0 else 128 - code)
    raise SystemExit(run_supervised(command, args.owner, args.temporary_root, args.keep, args.pid_namespace))
