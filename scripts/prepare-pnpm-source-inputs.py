#!/usr/bin/env python3
"""Prepare the locked workspace using the same contract in CI and local shells."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def prepare(repo, operation, publisher=None, cache_only=False):
    if operation in ("apply", "prepare"):
        if not shutil.which("nix"):
            raise ValueError("nix is required on PATH to materialize locked megarepo members")
        lock = json.loads((repo / "flake.lock").read_text())
        revision = lock["nodes"]["effect-utils"]["locked"]["rev"]
        if len(revision) != 40 or any(c not in "0123456789abcdef" for c in revision):
            raise ValueError("flake.lock effect-utils revision must be a full commit SHA")
        nix_options = (["--max-jobs", "0", "--option", "builders", "",
                        "--option", "external-builders", "[]"] if cache_only else [])
        try:
            subprocess.run([
                "nix", "run", *nix_options, f"github:overengineeringstudio/effect-utils/{revision}#megarepo",
                "--", "apply", "--worktree-mode", "commit", "--git-protocol", "https",
                "--lock-sync", "off",
            ], cwd=repo, check=True)
        except subprocess.CalledProcessError:
            if cache_only:
                raise ValueError("The workspace toolchain closure is not available in the binary cache; "
                                 "delivery refuses to build it locally or remotely. "
                                 "Publish the closure and configure the cache substituter before retrying.") from None
            raise
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
    parser.add_argument("--cache-only", action="store_true",
                        help="Refuse local, remote and external Nix builds; use binary caches only")
    args = parser.parse_args()
    try:
        prepare(Path.cwd(), args.operation, args.publisher, args.cache_only)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"source_inputs_preparation_failed: {error}\n")
