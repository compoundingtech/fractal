#!/usr/bin/env python3
"""Carry reviewed SQLite sites across source-path and Rust-namespace renames.

Run after r15-rename-rust.py and before crate_cut.py inventory. This adapter
accepts only renamed include-source literals and mechanically renamed function
namespaces. It proves each changed fingerprint against the previous reviewed
source before allowing the owning inventory generator to refresh annotations.
"""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'scripts'))
import sqlite_large_values as slv


def main():
    spec = importlib.util.spec_from_file_location('rename_docs', ROOT / 'scripts/r15-rename-docs.py')
    docs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(docs)
    directories = {old: new for old, new in docs.CRATES.items() if (ROOT / 'crates' / new).is_dir()
                   and not old.startswith('st2-')}
    directories.update({'st2-resource-providers': 'fractal-resource-providers',
                        'st2-resource-wasip2': 'fractal-resource-wasip2'})
    identifiers = {old.replace('-', '_'): new.replace('-', '_') for old, new in directories.items()}
    identifiers['st3'] = 'fractal_daemon'

    def forward(source):
        for old, new in sorted(directories.items(), key=lambda pair: -len(pair[0])):
            source = source.replace('crates/' + old + '/', 'crates/' + new + '/')
            source = source.replace('../' + old + '/', '../' + new + '/')
        for old, new in sorted(identifiers.items(), key=lambda pair: -len(pair[0])):
            source = re.sub(r'\b' + re.escape(old) + r'(?=\s*::)', new, source)
        return source

    path = ROOT / slv.MANIFEST
    manifest = json.loads(path.read_text())
    old = {entry['id']: entry for entry in manifest['sites']}
    baseline = subprocess.check_output(['git', 'merge-base', 'origin/main', 'HEAD'], cwd=ROOT, text=True).strip()
    reviewed = json.loads(subprocess.check_output(['git', 'show', baseline + ':' + slv.MANIFEST], cwd=ROOT, text=True))
    reviewed_by_id = {entry['id']: entry for entry in reviewed['sites']}
    # A merge can retain canonical source markers while taking the incoming
    # manifest with its pre-rename IDs. The first parent is also reviewed source.
    # Recover only explicitly anchored records; the generator still rejects
    # fingerprint changes and never grants an unreviewed exception.
    prior = json.loads(subprocess.check_output(
        ['git', 'show', 'HEAD^1:' + slv.MANIFEST], cwd=ROOT, text=True))
    prior_by_id = {entry['id']: entry for entry in prior['sites']}
    cache = {}
    sites, markers = slv.scan(ROOT, cache)
    by_path = {}
    for marker in markers:
        by_path.setdefault(marker[0], []).append(marker)
    for site in sites:
        source = cache[site.path][0]
        adjacent = [marker for marker in by_path.get(site.path, []) if marker[2] <= site.start
                    and not source[marker[2]:site.start].strip() and marker[3] is not None]
        if not adjacent:
            continue
        identifier = adjacent[-1][3]['id']
        if identifier in old:
            continue
        accepted = reviewed_by_id.get(identifier) or prior_by_id.get(identifier)
        if accepted is not None:
            old[identifier] = dict(accepted)
            manifest['sites'].append(old[identifier])
    changed = 0
    for site in sites:
        if site.id in old or site.kind != 'included-source':
            continue
        source = cache[site.path][0]
        adjacent = [marker for marker in by_path.get(site.path, []) if marker[2] <= site.start
                    and not source[marker[2]:site.start].strip() and marker[3] is not None]
        if not adjacent:
            raise ValueError(f'{site.path}:{site.line}: no reviewed include-source marker')
        identifier = adjacent[-1][3]['id']
        if identifier not in old:
            # A prior owning-generator pass deliberately drops unmatched sites.
            # Recover only an acceptance record present in the reviewed baseline.
            old[identifier] = dict(reviewed_by_id[identifier])
            manifest['sites'].append(old[identifier])
        entry = old[identifier]
        raw = source[site.start:site.end]
        previous = raw
        for before, after in sorted(directories.items(), key=lambda pair: -len(pair[1])):
            previous = previous.replace('/' + after + '/', '/' + before + '/')
        previous_sites, _ = slv.scan_file(site.path, previous)
        if len(previous_sites) != 1 or previous_sites[0].fingerprint != entry['fingerprint'] or forward(previous) != raw:
            raise ValueError(f'{site.path}:{site.line}: include-source changed beyond the rename')
        entry['fingerprint'] = site.fingerprint
        entry['excerpt'] = site.excerpt
        changed += 1
    for flow in manifest.get('flows', []):
        current_path = forward(flow['path'])
        if current_path == flow['path']:
            continue
        previous = subprocess.check_output(['git', 'show', baseline + ':' + flow['path']], cwd=ROOT, text=True)
        original = slv.flow_site(flow['path'], previous, flow['function'])
        proposed = slv.flow_site(current_path, forward(previous), flow['function'])
        actual = slv.flow_site(current_path, (ROOT / current_path).read_text(), flow['function'])
        if original['fingerprint'] != flow['fingerprint'] or proposed['fingerprint'] != actual['fingerprint']:
            raise ValueError(f'{current_path}:{flow["function"]}: flow changed beyond the rename')
        flow['fingerprint'] = actual['fingerprint']
        changed += 1
    path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'Proved {changed} source-only inventory fingerprint transitions')
    subprocess.run([sys.executable, 'scripts/crate_cut.py', 'inventory'], cwd=ROOT, check=True)


if __name__ == '__main__':
    main()
