#!/usr/bin/env python3
"""Read-only E3 snapshots of isolated client exports/copies; never reset a profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

from seed import instance_root

CLIENTS = ('web', 'ios', 'tui')


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def isolated_profile(profile, client, home):
    root = instance_root(home)
    allowed = root / 'client-profiles' / client
    if client not in CLIENTS or not profile.is_absolute() or '..' in profile.parts:
        raise ValueError('fixture-observer-path-refused')
    if not profile.is_relative_to(allowed):
        raise ValueError('fixture-observer-path-refused: not the declared fixture client profile')
    for path in [root.parent, root, *profile.relative_to(root).parents]:
        candidate = path if path.is_absolute() else root / path
        if candidate.is_symlink():
            raise ValueError('fixture-observer-path-refused: symlink parent')
    if profile.is_symlink() or not profile.is_dir():
        raise ValueError('fixture-observer-path-refused: profile must already exist without aliases')
    return profile


def flatten(value, pointer=''):
    # StorageState and AsyncStorage exports encode JSON payloads as strings.
    if isinstance(value, str):
        try:
            decoded = json.loads(value)
        except ValueError:
            decoded = None
        if isinstance(decoded, (dict, list)):
            yield from flatten(decoded, pointer + '/$json')
            return
    if isinstance(value, dict) and value:
        for key, child in sorted(value.items()):
            escaped = key.replace('~', '~0').replace('/', '~1')
            yield from flatten(child, f'{pointer}/{escaped}')
    elif isinstance(value, list) and value:
        for index, child in enumerate(value):
            yield from flatten(child, f'{pointer}/{index}')
    else:
        yield pointer, value


def snapshot(profile, client, item_id, policy, identity, home):
    isolated_profile(profile, client, home)
    classes = policy.get(client, {})
    exclusions = classes.get('identity', []) if isinstance(classes, dict) else []
    churn = classes.get('expected_churn', []) if isinstance(classes, dict) else []
    omitted = classes.get('raw_omit', []) if isinstance(classes, dict) else []
    if (not exclusions or any(not isinstance(keys, list) for keys in (exclusions, churn, omitted))
            or any(not isinstance(key, str) or not key or '*' in key
                   or '..' in Path(key).parts for key in exclusions + churn + omitted)):
        raise ValueError('fixture-observer-policy-refused: explicit named client classes required')
    if client == 'tui' and (not omitted or not identity.get('sanitized_profile_evidence')):
        raise ValueError('fixture-observer-policy-refused: omit raw private profile; require helper export')
    if (not isinstance(identity.get('device_key_fingerprint'), str)
            or not identity['device_key_fingerprint'] or not identity.get('pairing_id')
            or not identity.get('evidence')):
        raise ValueError('fixture-observer-identity-refused: actual fingerprint, pairing and provenance required')
    entries, markers, opaque = {}, [], []
    for path in sorted(profile.rglob('*')):
        if path.is_symlink():
            raise ValueError('fixture-observer-path-refused: symlink in profile')
        if not path.is_file():
            continue
        relative = path.relative_to(profile).as_posix()
        if relative in omitted:
            # Never open the private profile. Its safe helper export is captured separately.
            continue
        raw = path.read_bytes()
        try:
            value = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            entries[relative] = hashlib.sha256(raw).hexdigest()
            opaque.append(relative)
            continue
        contains_item = item_id in json.dumps(value)
        for pointer, leaf in flatten(value):
            selector = f'{relative}#{pointer}'
            entries[selector] = digest(leaf)
            words = re.sub(r'([a-z])([A-Z])', r'\1 \2',
                           pointer + (' ' + leaf if isinstance(leaf, str) else ''))
            tokens = re.split(r'[^a-z]+', words.lower())
            if (contains_item and any(token in tokens for token in ('read', 'seen', 'acted', 'closed'))
                    and leaf not in (None, False, 0, '', 'unread', 'unseen')):
                markers.append(selector)
    return {'version': 1, 'client': client, 'profile': str(profile), 'item_id': item_id,
            'identity': identity, 'exclusions': sorted(exclusions), 'policy_sha256': digest(policy),
            'expected_churn': sorted(churn), 'raw_omit': sorted(omitted),
            'entries': entries, 'read_seen_markers': sorted(markers), 'opaque_files': opaque}


def difference(before, after):
    for key in ('version', 'client', 'profile', 'item_id', 'exclusions',
                'policy_sha256', 'expected_churn', 'raw_omit'):
        if before[key] != after[key]:
            raise ValueError(f'fixture-observer-comparison-refused: changed {key}')
    identity_keys = ['device_key_fingerprint', 'pairing_id']
    if before['client'] == 'tui':
        identity_keys.extend(('public_key', 'device_id', 'session_actor', 'enrollment_id'))
    identity_changes = []
    for key in identity_keys:
        if not before['identity'].get(key) or not after['identity'].get(key):
            raise ValueError(f'fixture-observer-identity-refused: missing {key}')
        if before['identity'][key] != after['identity'][key]:
            identity_changes.append('identity/' + key)
    exclusions = set(before['exclusions'])
    def classified(key, names):
        return any(key == name or key.startswith(name + '/') or key.startswith(name + '#')
                   for name in names)
    if not exclusions or any(not any(classified(key, [name])
                                    for key in before['entries'].keys() | after['entries'].keys())
                             for name in exclusions):
        raise ValueError('fixture-observer-policy-refused: exclusion did not name captured state')
    changed = sorted(key for key in before['entries'].keys() | after['entries'].keys()
                     if not classified(key, exclusions)
                     and before['entries'].get(key) != after['entries'].get(key))
    identity_changes.extend(key for key in before['entries'].keys() | after['entries'].keys()
                            if classified(key, exclusions)
                            and before['entries'].get(key) != after['entries'].get(key))
    identity_changes.sort()
    expected = [key for key in changed if classified(key, before['expected_churn'])]
    unexplained = [key for key in changed if key not in expected]
    unscanned = sorted(set(changed + identity_changes)
                       & (set(before['opaque_files']) | set(after['opaque_files'])))
    # An already-present marker also defeats the initially-unseen assertion.
    findings = sorted(set(after['read_seen_markers']) | set(before['read_seen_markers']))
    return {'version': 1, 'client': before['client'], 'item_id': before['item_id'],
            'identity_preserved': not identity_changes,
            'device_key_fingerprint': before['identity']['device_key_fingerprint'],
            'pairing_id': before['identity']['pairing_id'], 'exclusions': sorted(exclusions),
            'changed': changed, 'expected_churn_changes': expected,
            'identity_changes': identity_changes,
            'unexplained_changes': unexplained, 'read_seen_findings': findings,
            'unscanned_changes': unscanned,
            'status': 'FINDING' if findings or unexplained or identity_changes or unscanned else 'DIFF_RECORDED',
            'requires_review': bool(unexplained or findings or identity_changes or unscanned)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    capture = commands.add_parser('snapshot')
    capture.add_argument('--client', choices=CLIENTS, required=True)
    capture.add_argument('--profile', type=Path, required=True)
    capture.add_argument('--item-id', required=True)
    capture.add_argument('--policy', type=Path, required=True)
    capture.add_argument('--identity', type=Path, required=True,
                         help='Actual client identity readback JSON with capture provenance')
    compare = commands.add_parser('diff')
    compare.add_argument('--before', type=Path, required=True)
    compare.add_argument('--after', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'snapshot':
        result = snapshot(args.profile, args.client, args.item_id, json.loads(args.policy.read_text()),
                          json.loads(args.identity.read_text()), Path(os.environ['HOME']))
    else:
        result = difference(json.loads(args.before.read_text()), json.loads(args.after.read_text()))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
