#!/usr/bin/env python3
"""Prepare the locked workspace using the same contract in CI and local shells."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def prepare(repo, operation, publisher=None):
    if operation in ("apply", "prepare"):
        if not shutil.which("nix"):
            raise ValueError("nix is required on PATH to materialize locked megarepo members")
        lock = json.loads((repo / "flake.lock").read_text())
        revision = lock["nodes"]["effect-utils"]["locked"]["rev"]
        if len(revision) != 40 or any(c not in "0123456789abcdef" for c in revision):
            raise ValueError("flake.lock effect-utils revision must be a full commit SHA")
        subprocess.run([
            "nix", "run", f"github:overengineeringstudio/effect-utils/{revision}#megarepo",
            "--", "apply", "--worktree-mode", "commit", "--git-protocol", "https",
            "--lock-sync", "off",
        ], cwd=repo, check=True)
    if operation in ("publish", "prepare"):
        paths = json.loads((repo / "pnpm-source-inputs.json").read_text())["sourceInputPaths"]
        subprocess.run([
            "node", str(publisher or repo / "repos/effect-utils/nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs"),
            "publish", str(repo), ".devenv/pnpm-source-inputs", *paths,
        ], cwd=repo, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("apply", "publish", "prepare"))
    parser.add_argument("--publisher", type=Path)
    args = parser.parse_args()
    try:
        prepare(Path.cwd(), args.operation, args.publisher)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"source_inputs_preparation_failed: {error}\n")
