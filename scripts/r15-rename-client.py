#!/usr/bin/env python3
"""Reproduce R15 client source rewrites after the pure directory-move commit.

Run: python3 scripts/r15-rename-client.py
Generated artifacts are deliberately left to their owning generators:
  cargo run -p fractal-client-codegen --bin st3-client-codegen
  genie (root package/workspace and fractal-sdk package manifests)
  python3 scripts/ci-fractal-web-licenses (after pnpm lockfile regeneration)
This script does not change wire values, service names, or NSError domains.
"""
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = {
    '@smalltalk/st3-client': '@fractal/client',
    '@smalltalk/st3-scenarios': '@fractal/scenarios',
    '@smalltalk/st3-views': '@fractal/views',
    '@smalltalk/fractal-ui': '@fractal/ui',
    '@st3/sdk': '@fractal/sdk',
}
PATHS = {
    'clients/typescript/st3-client': 'clients/typescript/fractal-client',
    'clients/typescript/st3-scenarios': 'clients/typescript/fractal-scenarios',
    'clients/typescript/st3-views': 'clients/typescript/fractal-views',
    'clients/swift/St3Client': 'clients/swift/FractalClient',
    'packages/st3-sdk': 'packages/fractal-sdk',
}
GENERATED = {
    'clients/typescript/fractal-client/Client.generated.ts',
    'clients/typescript/fractal-client/Models.generated.ts',
    'clients/typescript/fractal-client/Schema.generated.ts',
    'clients/swift/FractalClient/Sources/FractalClient/Client.swift',
    'clients/swift/FractalClient/Sources/FractalClient/Models.swift',
    'clients/swift/FractalClient/Sources/FractalClient/Contract.generated.swift',
}
SOURCE_SUFFIXES = {
    '.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs', '.swift', '.py',
    '.css', '.scss', '.sass', '.less', '.html', '.htm', '.vue', '.svelte',
    '.astro', '.mdx',
}
EXCLUDED_ROOTS = {'scripts', 'docs', '.st', 'nix', 'buck2', '.github', 'repos', 'release-notes'}


def selected(path: Path) -> bool:
    relative = path.relative_to(ROOT)
    if relative.parts[0] in EXCLUDED_ROOTS or relative.as_posix() in GENERATED:
        return False
    if relative.as_posix() == 'apps/ios/modules/st-fabric/build.sh':
        return True
    if path.name.startswith('BUCK') or path.name == 'Cargo.toml':
        return False
    if relative.parts[0] == 'crates':
        return relative.parts[1] == 'fractal-client-codegen' and path.suffix in {'.rs', '.in'}
    if path.name == 'package.json':
        return not path.with_name('package.json.genie.ts').exists()
    if path.suffix in {'.json', '.jsonc'}:
        # Source configuration is live; recorded data and immutable release
        # classifications retain the identities at their historical source.
        return 'config' in path.stem
    return path.suffix in SOURCE_SUFFIXES


def rewrite(text: str, path: Path) -> str:
    if path.relative_to(ROOT).as_posix() == 'apps/ios/modules/st-fabric/build.sh':
        return text.replace('libst_fabric_bridge.a', 'libfractal_fabric_bridge.a')
    if path.relative_to(ROOT).as_posix() == 'apps/ios/scripts/test_release_delivery.py':
        # The isolated release fixture derives package names from source paths.
        # Its manifests must use the same canonical names as its dependency edges.
        text = text.replace('"@smalltalk/" + Path(relative).name',
                            '"@fractal/" + Path(relative).name.removeprefix("fractal-")')
    for old, new in PACKAGES.items():
        text = text.replace(old, new)
    for old, new in PATHS.items():
        text = text.replace(old, new)
    # Relative source imports do not always include the full repository path.
    for package in ('client', 'scenarios', 'views'):
        text = text.replace(f'../st3-{package}', f'../fractal-{package}')
    # Public source identifiers change; NSError domains are stable error strings.
    text = re.sub(r'\bSt3Client(?=Tests\b|\b)', 'FractalClient', text)
    text = text.replace('domain: "FractalClient"', 'domain: "St3Client"')
    text = re.sub(r'\bst3Client(?=[A-Z])', 'fractalClient', text)
    # Source-only crate references (including Rust emitted through templates).
    text = re.sub(r'\bst3_([a-z][a-z0-9_]*)(?=::)', r'fractal_\1', text)
    text = re.sub(r'crates/(?:st3|st2|st)-([a-z0-9-]+)', r'crates/fractal-\1', text)
    text = text.replace('crates/stui/', 'crates/fractal-tui/')
    if 'fractal-client-codegen' in path.parts:
        text = text.replace('cargo run -p st3-client-codegen', 'cargo run -p fractal-client-codegen --bin st3-client-codegen')
    return text


# Repair only this script's earlier rewrites of immutable historical fragments.
# A fragment with unrelated changes is not silently overwritten.
historical_base = 'e9a9167884ea56ae22ed0605bdf284610b6fa9fb'
historical_notes = subprocess.check_output(
    ['git', 'ls-tree', '-r', '--name-only', historical_base, '--', 'release-notes'],
    cwd=ROOT).decode().splitlines()
for name in historical_notes:
    path = ROOT / name
    if not path.is_file():
        continue
    before = subprocess.check_output(['git', 'show', historical_base + ':' + name], cwd=ROOT).decode()
    current = path.read_text()
    if current != before and current == rewrite(before, path):
        path.write_text(before)

paths = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
changed = 0
for name in paths:
    if not name:
        continue
    path = ROOT / name
    if not path.is_file() or not selected(path):
        continue
    before = path.read_text()
    after = rewrite(before, path)
    if after != before:
        path.write_text(after)
        changed += 1
print(f'Rewrote {changed} client source/package files; generated outputs require owning generators.')
