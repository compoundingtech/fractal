#!/usr/bin/env python3
"""Replay R15's source-only build-input rewrite from the pure directory-move commit.

Run: python3 scripts/r15-rename-build.py
Generated Buck/workflow outputs remain owned by Genie; regenerate after updating
pnpm-lock.yaml with: nix develop .#genie -c genie
Nix output attributes, derivation names, executables, environment and installed
component paths deliberately retain their existing consumer contracts.
"""
from pathlib import Path
import re

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
    if path.name in {"clippy-baseline.json", "clippy-splits.json"}:
        after = re.sub(r'"(' + OLD + r')"', lambda match: '"' + package(match[1]) + '"', after)
    if path.name == ".gitattributes":
        after = after.replace("@smalltalk/st3-scenarios", "@fractal/scenarios")
    if after != before:
        path.write_text(after)
        print(path.relative_to(ROOT))
