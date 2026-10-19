#!/usr/bin/env python3
"""Replay R15's source-only build-input rewrite from the pure directory-move commit.

Run: python3 scripts/r15-rename-build.py
Generated Buck/workflow outputs remain owned by Genie; regenerate after updating
pnpm-lock.yaml with: nix develop .#genie -c genie
Nix output attributes, derivation names, executables, environment and installed
component paths deliberately retain their existing consumer contracts.
"""
from pathlib import Path
import json
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
OLD = r"(?:st3(?:-[a-z0-9-]+)?|st2(?:-[a-z0-9-]+)?|st-[a-z0-9-]+|stui)"


def package(name: str) -> str:
    if name == "st2":
        return "fractal-legacy-runner"
    if name == "st3":
        return "fractal"
    if name == "stui":
        return "fractal-tui"
    return re.sub(r"^st[23]?-", "fractal-", name)


def source_paths(text: str) -> str:
    text = re.sub(r"crates/(" + OLD + r")(?=[/\s'\"`])",
                  lambda match: "crates/" + package(match[1]), text)
    text = re.sub(r"clients/typescript/st3-(client|views|scenarios)",
                  r"clients/typescript/fractal-\1", text)
    text = text.replace("apps/fractal-web/packages/st3-sdk", "apps/fractal-web/packages/fractal-sdk")
    text = text.replace("clients/swift/St3Client", "clients/swift/FractalClient")
    text = text.replace("Sources/St3Client", "Sources/FractalClient")
    return text


def clippy_ancestry(text: str) -> str:
    # Ratchet base commits keep their original Cargo names. A one-to-one rename
    # is an ancestry edge, not a source alias or permission to add warnings.
    base = "e9a9167884ea56ae22ed0605bdf284610b6fa9fb"
    def historical(path):
        return subprocess.check_output(["git", "show", f"{base}:{path}"],
                                       cwd=ROOT, text=True)
    ancestry = {**json.loads(text), **json.loads(historical(".github/clippy-splits.json"))}
    for entry in tomllib.loads(historical("Cargo.lock"))["package"]:
        old = entry["name"]
        if package(old) != old:
            ancestry[package(old)] = old
    return json.dumps(ancestry, indent=2, sort_keys=True) + "\n"


files = {ROOT / name for name in (
    "flake.nix", ".gitignore", ".gitattributes", "BUCK.genie.ts", ".buckconfig.genie.ts",
    ".config/nextest.toml",
)}
for directory, suffixes in (("nix", {".nix", ".json"}), ("buck2", {".ts"}),
                            (".github", {".ts", ".json"})):
    files.update(path for path in (ROOT / directory).rglob("*") if path.suffix in suffixes)
files.update(ROOT.glob("clients/typescript/*/BUCK.genie.ts"))

for path in sorted(files):
    before = path.read_text()
    after = source_paths(before)
    # Cargo package selectors only: --bin and externally exposed Nix names stay put.
    after = re.sub(r'("(?:-p|--exclude)"\s+)"(' + OLD + r')"',
                   lambda match: match[1] + '"' + package(match[2]) + '"', after)
    # Provider mapping keys are Cargo identities; values are stable installed wasm names.
    if path.name == "flake.nix":
        after = re.sub(r'"(st2-[a-z-]+-component)"(?= = "st2_)',
                       lambda match: '"' + package(match[1]) + '"', after)
    after = re.sub(r'package\((=?)(' + OLD + r')\)',
                   lambda match: "package(" + match[1] + package(match[2]) + ")", after)
    after = re.sub(r'("package":\s*")(' + OLD + r')(")',
                   lambda match: match[1] + package(match[2]) + match[3], after)
    if path.name == "clippy-baseline.json":
        after = re.sub(r'"(' + OLD + r')"', lambda match: '"' + package(match[1]) + '"', after)
    if path.name == "clippy-splits.json":
        after = clippy_ancestry(before)
    if path.name == ".gitattributes":
        after = after.replace("@smalltalk/st3-scenarios", "@fractal/scenarios")
    if after != before:
        path.write_text(after)
        print(path.relative_to(ROOT))
