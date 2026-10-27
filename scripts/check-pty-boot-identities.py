#!/usr/bin/env python3
"""Refuse a Smalltalk rollout while any running PTY record lacks a boot identity."""

import argparse
import json
from pathlib import Path
import sys


def fail(message):
    print(f"check-pty-boot-identities: {message}", file=sys.stderr)
    return 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pty_root", type=Path, help="the PTY registry directory")
    args = parser.parse_args()

    try:
        sessions = json.load(sys.stdin)
    except (OSError, json.JSONDecodeError) as error:
        return fail(f"cannot read `pty list --json`: {error}")
    if not isinstance(sessions, list):
        return fail("`pty list --json` did not return a list")

    running = []
    seen = set()
    for session in sessions:
        if not isinstance(session, dict) or not isinstance(session.get("status"), str):
            return fail("`pty list --json` returned a malformed session")
        if session["status"] != "running":
            continue
        name = session.get("name")
        if not isinstance(name, str) or not name or Path(name).name != name:
            return fail("`pty list --json` returned an invalid running session name")
        if name in seen:
            return fail(f"`pty list --json` returned duplicate running session {name}")
        seen.add(name)
        running.append(name)

    missing = []
    for name in running:
        path = args.pty_root / f"{name}.json"
        try:
            record = json.loads(path.read_text())
        except FileNotFoundError:
            missing.append((name, "registry record is missing"))
            continue
        except (OSError, json.JSONDecodeError) as error:
            return fail(f"cannot read registry record {path}: {error}")
        if not isinstance(record, dict):
            return fail(f"registry record {path} is not an object")
        boot_id = record.get("daemonBootId")
        if not isinstance(boot_id, str) or not boot_id.strip():
            missing.append((name, "daemonBootId is absent or empty"))

    print(
        "pty boot identity preflight: "
        f"{len(missing)}/{len(running)} running sessions missing daemonBootId"
    )
    for name, reason in missing:
        print(f"check-pty-boot-identities: {name}: {reason}", file=sys.stderr)
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
