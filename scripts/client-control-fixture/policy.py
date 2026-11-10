#!/usr/bin/env python3
"""Bind source-inventoried names to actual captured selectors; never collect client secrets."""
import argparse
import json
from pathlib import Path

import observer
import packet


INVENTORY = Path(__file__).with_name('storage-inventory.json')


def compile_policy(bindings, home):
    inventory = packet.load(INVENTORY)
    if bindings['inventory_sha256'] != packet.digest(INVENTORY.read_bytes()):
        raise ValueError('fixture-policy-refused: named source inventory digest mismatch')
    output = {}
    for client in packet.CLIENTS:
        source, row = inventory[client], bindings['clients'][client]
        profile = Path(row['profile'])
        observer.isolated_profile(profile, client, home)
        omitted = row.get('raw_omit', [])
        if client == 'tui' and (not omitted or not row.get('sanitized_profile_evidence')):
            raise ValueError('fixture-policy-refused: actual private profile omission and safe helper export required')
        if client != 'tui' and omitted:
            raise ValueError('fixture-policy-refused: no inventoried raw omissions for this client')
        for relative in omitted:
            path = profile / relative
            if (not relative or Path(relative).is_absolute() or '..' in Path(relative).parts
                    or path.is_symlink() or path.resolve() != path or not path.is_file()):
                raise ValueError('fixture-policy-refused: raw omission must name actual isolated private profile')
        # Enumerate only safe captured data. Never open an omitted private profile.
        present = set()
        for path in sorted(profile.rglob('*')):
            if path.is_symlink():
                raise ValueError('fixture-policy-refused: profile symlink')
            if not path.is_file():
                continue
            relative = path.relative_to(profile).as_posix()
            if relative in omitted:
                continue
            try:
                parsed = json.loads(path.read_bytes())
            except (ValueError, UnicodeDecodeError):
                present.add(relative)
            else:
                present.update(f'{relative}#{pointer}' for pointer, _ in observer.flatten(parsed))
        classes = {'identity': [], 'expected_churn': [], 'raw_omit': omitted}
        accounted = set()
        for binding in row['bindings']:
            classification, name, selector = binding['class'], binding['name'], binding['selector']
            if (classification not in ('identity', 'expected_churn')
                    or name not in source[classification] or not binding['capture_provenance']
                    or not isinstance(selector, str) or not selector or '*' in selector
                    or '..' in Path(selector.split('#', 1)[0]).parts
                    or Path(selector.split('#', 1)[0]).is_absolute()):
                raise ValueError('fixture-policy-refused: unowned name or unsafe/unproven selector')
            if classification == 'identity' and '#' not in selector:
                raise ValueError('fixture-policy-refused: identity must name a captured JSON field, not a whole file')
            if not any(key == selector or key.startswith(selector + '/') for key in present):
                raise ValueError('fixture-policy-refused: selector absent from actual safe capture')
            classes[classification].append(selector)
            accounted.add(name)
        for limitation in row.get('unobserved', []):
            if (limitation['name'] not in source['identity'] or not limitation['reason']
                    or not limitation['capture_provenance'] or not limitation['source_evidence']):
                raise ValueError('fixture-policy-refused: absent identity key needs explicit source/capture limitation')
            accounted.add(limitation['name'])
        if not set(source['identity']).issubset(accounted) or not classes['identity']:
            raise ValueError('fixture-policy-refused: every named identity key needs actual binding or declared limitation')
        for classification in classes:
            classes[classification] = sorted(set(classes[classification]))
        if set(classes['identity']) & set(classes['expected_churn']):
            raise ValueError('fixture-policy-refused: selector has conflicting classes')
        output[client] = classes
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bindings', type=Path, required=True)
    args = parser.parse_args()
    import os
    print(json.dumps(compile_policy(packet.load(args.bindings), Path(os.environ['HOME'])),
                     sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
