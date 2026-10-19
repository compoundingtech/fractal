#!/usr/bin/env python3
"""R15 mechanical Cargo/Rust rename, after the pure directory-move commit.

Run from any directory: python3 scripts/r15-rename-rust.py
Only Cargo identities, Rust crate paths, and source-file references change.
Executable targets, wire values, environment names, and state paths stay intact.
Generator sources and generated Rust clients belong to r15-rename-client.py.
"""
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
FILES = [ROOT / name for name in subprocess.check_output(
    ["git", "ls-files", "-z"], cwd=ROOT
).decode().split("\0") if name]


def rename(name):
    if name == "st2":
        return "fractal-legacy-runner"
    if name == "st3":
        return "fractal"
    if name == "stui":
        return "fractal-tui"
    for prefix in ("st3-", "st2-", "st-"):
        if name.startswith(prefix):
            return "fractal-" + name[len(prefix):]
    return name


MANIFESTS = {p: tomllib.loads(p.read_text()) for p in FILES
             if p.name == "Cargo.toml" and p.exists()}
PACKAGES = {d["package"]["name"]: rename(d["package"]["name"])
            for d in MANIFESTS.values() if "package" in d
            and rename(d["package"]["name"]) != d["package"]["name"]}
# Keep the map useful on repeat invocations after Cargo identities are renamed.
for p, d in MANIFESTS.items():
    if p.parent.parent == ROOT / "crates" and p.parent.name.startswith("fractal"):
        suffix = p.parent.name.removeprefix("fractal")
        if suffix and suffix != "-tui":
            for prefix in ("st3", "st2", "st"):
                PACKAGES.setdefault(prefix + suffix, p.parent.name)
PACKAGES.update({"st2": "fractal-legacy-runner", "st3": "fractal", "stui": "fractal-tui",
                 "st3-client-tui": "fractal-client-tui", "st-fabric-bridge": "fractal-fabric-bridge"})
for component in ("github-issue", "github-pr", "pty-stats", "vista"):
    PACKAGES["st2-" + component + "-component"] = "fractal-" + component + "-component"

IDENTIFIERS = {old.replace("-", "_"): new.replace("-", "_")
               for old, new in PACKAGES.items()}
# `st3` is a dependency alias of the daemon, not the program marker library.
IDENTIFIERS["st3"] = "fractal_daemon"
IDENTIFIERS["st3_package_marker"] = "fractal_package_marker"
IDENTIFIERS["st3_package_self"] = "fractal_package_self"
for d in MANIFESTS.values():
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for key, value in d.get(section, {}).items():
            if isinstance(value, dict) and "package" in value and key.endswith("_self"):
                IDENTIFIERS[key] = re.sub(r"^(st3|st2|st)_", "fractal_", key)
# Repeatability for self aliases even after manifests have already changed.
SELF_ALIAS = re.compile(r"\bst3_([a-z0-9_]+_self)\b")
PACKAGE_TOKEN = re.compile(r"(?<![\w-])(" + "|".join(map(re.escape, sorted(PACKAGES, key=len, reverse=True))) + r")(?![\w-])")
IDENTIFIER_TOKEN = re.compile(r"\b(" + "|".join(map(re.escape, sorted(IDENTIFIERS, key=len, reverse=True))) + r")\b(?=\s*::)")
IMPORT_TOKEN = re.compile(r"\b(use|extern\s+crate)\s+(" + "|".join(map(re.escape, sorted(IDENTIFIERS, key=len, reverse=True))) + r")\b")
# Directory substitutions are limited to source-tree references, never XDG/state paths.
DIRECTORIES = {old: new for old, new in PACKAGES.items()
               if (ROOT / "crates" / new).is_dir()}
SOURCE_PATH = re.compile(r"(?P<base>crates/|\.\./)(?P<name>" + "|".join(map(re.escape, sorted(DIRECTORIES, key=len, reverse=True))) + r")(?=/|[\"'\s`\\]|$)")


def source_paths(text):
    return SOURCE_PATH.sub(lambda m: m["base"] + DIRECTORIES[m["name"]], text)


def manifest(p, data):
    original = p.read_text()
    lines = []
    section = ""
    for line in original.splitlines(keepends=True):
        header = re.match(r"\s*(\[\[?.*?\]\]?)\s*$", line)
        if header:
            section = header[1]
        # All executable/example/test/bench target names and default-run remain stable.
        if (section in ("[[bin]]", "[[example]]", "[[test]]", "[[bench]]")
                and re.match(r"\s*name\s*=", line)) or re.match(r"\s*default-run\s*=", line):
            lines.append(line)
            continue
        if re.match(r"\s*description\s*=", line):
            lines.append(line)
            continue
        # The only old bare dependency key aliases the daemon package.
        line = re.sub(r"^(\s*)st3(\s*=\s*\{)\s*package\s*=\s*\"st3-daemon\",\s*",
                      r"\1fractal-daemon\2 ", line)
        line = line.replace('"st3/', '"fractal-daemon/')
        line = SELF_ALIAS.sub(lambda m: "fractal_" + m[1], line)
        line = re.sub(r"\bst3_package_marker\b", "fractal_package_marker", line)
        if section == "[lib]" and re.match(r"\s*name\s*=", line):
            line = re.sub(r'"st2"', '"fractal_legacy_runner"', line)
        line = PACKAGE_TOKEN.sub(lambda m: PACKAGES[m[1]], line)
        lines.append(line)
    result = "".join(lines)
    old = data.get("package", {}).get("name")
    if old in PACKAGES and (p.parent / "src/main.rs").is_file():
        # Cargo derives src/main.rs's binary name from package.name unless an explicit
        # target already points there. Preserve the pre-rename implicit executable.
        explicit_main = any(b.get("path") == "src/main.rs" or b.get("name") == old
                            for b in data.get("bin", []))
        if not explicit_main:
            result = result.rstrip() + '\n\n[[bin]]\nname = "' + old + '"\npath = "src/main.rs"\n'
    return result


def rust(p):
    text = p.read_text()
    if p.relative_to(ROOT).as_posix() == "crates/fractal-schema/src/lib.rs":
        text = text.replace("This file is generated from `st3-schema`", "This file is generated from `fractal-schema`")
    text = source_paths(text)
    text = SELF_ALIAS.sub(lambda m: "fractal_" + m[1], text)
    text = IDENTIFIER_TOKEN.sub(lambda m: IDENTIFIERS[m[1]], text)
    text = IMPORT_TOKEN.sub(lambda m: m[1] + " " + IDENTIFIERS[m[2]], text)
    # Explicit tracing targets are runtime identities, not Rust imports.
    for old in ("st3::http", "st3::relay_diagnostic", "st_chatgpt::call"):
        new = IDENTIFIER_TOKEN.sub(lambda m: IDENTIFIERS[m[1]], old)
        text = text.replace('"' + new + '"', '"' + old + '"')
    # Package selection in source comments/examples is not an executable rename.
    text = re.sub(r"(?P<flag>(?:-p|--package)\s+)(?P<name>" +
                  "|".join(map(re.escape, sorted(PACKAGES, key=len, reverse=True))) +
                  r")(?![\w-])", lambda m: m["flag"] + PACKAGES[m["name"]], text)
    return text


changed = 0
for p in FILES:
    if not p.is_file():
        continue
    relative = p.relative_to(ROOT).as_posix()
    if p in MANIFESTS:
        result = manifest(p, MANIFESTS[p])
    elif p.suffix == ".rs":
        if (relative.startswith(("scripts/", "crates/fractal-client-codegen/", "vendor/"))
                or "/vendor/" in relative
                or relative in {"crates/fractal-client/src/lib.rs", "crates/fractal-client/src/generated.rs",
                                "crates/fractal-client/src/contract.rs"}):
            continue
        result = rust(p)
    else:
        continue
    if result != p.read_text():
        p.write_text(result)
        changed += 1
print(f"Renamed Cargo/Rust source identities in {changed} files")
