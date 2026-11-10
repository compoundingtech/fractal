#!/usr/bin/env python3
"""Source-only preparation: invoke seed only under separately authorized fixture setup."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess

INSTANCE = 'client-control-fixture'
PERSON = f'person/{INSTANCE}'
ACTOR = f'agent/{INSTANCE}/producer'
TITLE = 'Client control fixture check'
BODY = 'client-control fixture 0001'
MIGRATION = 'arn:lmig:smalltalk:2026-10-16-state-dirs-fractal\n'


def validate_text(text):
    if (not 1 <= len(text) <= 60 or any(not 32 <= ord(c) <= 126 for c in text)
            or text != text.strip() or '  ' in text
            or any(c in text for c in '*_#[]`>|')):
        raise ValueError('fixture-text-refused: use one plain ASCII paragraph of at most 60 characters')
    try:
        json.loads(text)
    except json.JSONDecodeError:
        return text
    raise ValueError('fixture-text-refused: JSON text is not a fixture paragraph')


def instance_root(home):
    # Match fractal_client::product_dirs::instance_root and its completed migration receipt.
    migrated = home / '.fractal-instance' / INSTANCE
    receipt = migrated / '.fractal-migration-completed'
    if receipt.exists() and receipt.read_bytes() == MIGRATION.encode('ascii'):
        return migrated
    return home / '.st-instance' / INSTANCE


def operator_socket(socket, home):
    root = instance_root(home)
    expected = root / 'run/fractal.sock'
    if not home.is_absolute() or socket != expected:
        raise ValueError('local-only-fixture-path-refused: expected own fixture operator socket')
    for path in [root.parent, root, root / 'run', expected]:
        if path.is_symlink():
            raise ValueError('local-only-fixture-path-refused: symlink alias')
    if socket.parent.resolve() / socket.name != root.resolve() / 'run/fractal.sock':
        raise ValueError('local-only-fixture-path-refused: resolved socket escapes fixture root')
    if not stat.S_ISSOCK(socket.stat().st_mode):
        raise ValueError('local-only-fixture-path-refused: operator socket must already exist')
    return socket


class Fixture:
    def __init__(self, program, socket):
        self.program, self.socket = program, socket

    def cli(self, *args, request=None):
        # Revalidate before every invocation; never discover/fall back to a normal socket.
        socket = operator_socket(self.socket, Path(os.environ['HOME']))
        result = subprocess.run([self.program, '--instance', INSTANCE, '--endpoint',
                                 f'unix:{socket}', '--json', *args],
                                input=None if request is None else json.dumps(request),
                                text=True, capture_output=True, check=True)
        return json.loads(result.stdout)

    def attention(self):
        envelope = self.cli('alerts', 'ls', '--as', PERSON, '--limit', '200')
        page = envelope['value']
        if page['page']['has_more'] or page['page'].get('omitted_items'):
            raise ValueError('fixture-inbox-refused: incomplete attention projection')
        return page['items']

    def claims(self):
        claims, cursor = [], None
        while True:
            args = ['claims', 'ls', '--limit', '500']
            if cursor is not None:
                args += ['--after-index', str(cursor)]
            page = self.cli(*args)
            claims.extend(page['claims'])
            next_cursor = page.get('next_cursor')
            if next_cursor is None:
                return claims
            if cursor is not None and next_cursor <= cursor:
                raise ValueError('fixture-claims-refused: nonadvancing cursor')
            cursor = next_cursor


def assert_no_read_positions(claims):
    # Native asks have no read-position model; refuse durable position records.
    for claim in claims:
        kind = claim['kind'].lower().replace('_', '-').replace('.', '-')
        if 'read' in kind and 'position' in kind:
            raise ValueError(f'fixture-read-position-refused: {claim["id"]}')


def no_read_mutation(items, claims, ask, title=TITLE, body=BODY):
    validate_text(title)
    validate_text(body)
    if len(items) != 1:
        raise ValueError('fixture-inbox-refused: expected exactly one attention item')
    item = items[0]
    if (item['source_id'] != ask or item['person_id'] != PERSON
            or item['attention_kind'] != 'person-step' or item['state'] != 'open'
            or item['presentation']['kind'] != 'request' or item['title'] != title
            or item['request']['question'] != body):
        raise ValueError('fixture-ask-refused: expected own open native request')
    ask_claims = [claim for claim in claims if claim['subject'] == ask]
    requested = [claim for claim in ask_claims if claim['kind'] == 'work.person-asked']
    if len(requested) != 1:
        raise ValueError('fixture-ask-refused: missing unique ask claim')
    fields = requested[0]['body']['fields']
    if (requested[0]['actor'] != ACTOR or fields['person'] != PERSON
            or fields['title'] != title or fields['request']['question'] != body):
        raise ValueError('fixture-ask-refused: source claim does not bind seed')
    forbidden = re.compile(r'read|acknowledg|answer|clos|person-done|person-cancel|completed')
    assert_no_read_positions(claims)
    for claim in claims:
        if claim['subject'] == ask and forbidden.search(claim['kind']):
            raise ValueError(f'fixture-no-read-mutation-refused: {claim["id"]}')
    return {'check': 'no-read-mutation', 'ask_id': ask, 'person': PERSON,
            'attention_id': item['id'], 'title': title, 'body': body,
            'body_sha256': hashlib.sha256(body.encode('ascii')).hexdigest(),
            'claim_ids': sorted(claim['id'] for claim in ask_claims),
            'read_position_claim_ids': [], 'state': 'open', 'answered': False,
            'surfaces': ['current-attention-projection', 'complete-durable-claims'],
            'unread_bit': 'not-supported-for-native-asks'}


def seed(fixture, key, title, body):
    validate_text(title)
    validate_text(body)
    if body != BODY:
        raise ValueError('fixture-text-refused: seed body must match the fixed fixture')
    if fixture.attention():
        raise ValueError('fixture-inbox-refused: seed requires an empty fixture inbox')
    assert_no_read_positions(fixture.claims())
    request = {'version': 1, 'type': 'feedback', 'question': body,
               'why_person': 'Isolated client rendering fixture'}
    created = fixture.cli('work', 'ask', '--for', PERSON, '--as', ACTOR,
                          '--title', title, '--request', '-', '--new-run', 'client-control-fixture',
                          '--idempotency-key', key, request=request)
    return no_read_mutation(fixture.attention(), fixture.claims(), created['subject'], title, body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fractal', default='fractal')
    parser.add_argument('--socket', type=Path, required=True)
    commands = parser.add_subparsers(dest='command', required=True)
    create = commands.add_parser('seed', help='Requires separate setup authorization; creates one ask')
    create.add_argument('--idempotency-key', required=True)
    create.add_argument('--title', default=TITLE)
    create.add_argument('--body', default=BODY)
    check = commands.add_parser('check', help='Read-only before/after journey invariant')
    check.add_argument('--ask', required=True)
    check.add_argument('--baseline', type=Path, help='Compare with saved before-check JSON')
    args = parser.parse_args()
    fixture = Fixture(args.fractal, args.socket)
    if args.command == 'seed':
        receipt = seed(fixture, args.idempotency_key, args.title, args.body)
    else:
        baseline = json.loads(args.baseline.read_text()) if args.baseline else None
        title, body = (baseline['title'], baseline['body']) if baseline else (TITLE, BODY)
        receipt = no_read_mutation(fixture.attention(), fixture.claims(), args.ask, title, body)
        if baseline is not None and receipt != baseline:
            raise ValueError('fixture-no-read-mutation-refused: journey changed seed evidence')
    print(json.dumps(receipt, sort_keys=True))


if __name__ == '__main__':
    main()
