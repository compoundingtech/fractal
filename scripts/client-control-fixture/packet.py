#!/usr/bin/env python3
"""Source-only common packet: read-only evidence capture and human-only command preparation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess

import seed

HERE = Path(__file__).resolve().parent
CLIENTS = ('ios', 'web', 'tui')
SCREENS = ['NowRoot', 'Attention']


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode('ascii')


def load(path):
    return json.loads(Path(path).read_text())


def manifest():
    value = load(HERE / 'expected-input.json')
    if (value['schema'] != 'client-control-fixture-input/v1'
            or value['instance'] != seed.INSTANCE or value['actor'] != seed.ACTOR
            or value['person'] != seed.PERSON or value['title'] != seed.TITLE
            or value['body'] != seed.BODY or value['screens'] != SCREENS
            or value['clients'] != list(CLIENTS)
            or value['body_sha256'] != digest(seed.BODY.encode('ascii'))
            or any(item is not None for item in value['generated'].values())):
        raise ValueError('immutable-input-refused: manifest differs from fixed fixture')
    seed.validate_text(value['title'])
    seed.validate_text(value['body'])
    return value


def digests():
    manifest()
    files = {name: digest((HERE / name).read_bytes())
             for name in ('expected-input.json', 'packet.py', 'seed.py', 'acceptance.py', 'observer.py',
                          'policy.py', 'storage-inventory.json')}
    return {'input_sha256': files['expected-input.json'],
            'packet_sha256': digest(canonical(files)), 'packet_files': files}


def safe_root(home):
    if not home.is_absolute() or home != Path(os.environ['HOME']):
        raise ValueError('fixture-root-refused: only absolute operator HOME')
    root = seed.instance_root(home)
    # Check every existing ancestor, not only the leaf or run directory.
    for path in (home, *home.parents, root.parent, root,
                 root / '.fractal-migration-completed', root / 'run', root / 'run/fractal.sock'):
        if path.is_symlink():
            raise ValueError('fixture-root-refused: symlink alias')
    if root.resolve() != root:
        raise ValueError('fixture-root-refused: resolved root differs')
    return root


def safe_socket(socket, home):
    root = safe_root(home)
    if socket != root / 'run/fractal.sock':
        raise ValueError('fixture-socket-refused: normal or alternate socket')
    return seed.operator_socket(socket, home)


def fabric_paths(home):
    root = safe_root(home)
    own = root / 'fabric'
    control = own / 'run/control.sock'
    for path in (own, own / 'run', control, own / 'identity.toml', own / 'peers.toml', own / 'config.toml'):
        if path.is_symlink() or path.resolve() != path:
            raise ValueError('fixture-fabric-refused: dedicated home/control symlink alias')
    return own, control


def fabric_commands(home):
    own, control = fabric_paths(home)
    prefix = ('env', f'FABRIC_HOME={own}', 'fabric', '--home', own)
    return {'home': str(own), 'control_socket': str(control),
            'start': command(*prefix, 'up', '--foreground'),
            'status': command(*prefix, 'status', '--json'),
            'stop': command(*prefix, 'down'),
            'scope': 'Separate authenticated Fabric-mesh start/stop scope required; no ambient Fabric.',
            'source_prerequisite': 'Dedicated fixture execution context must land before any invocation.'}


def fabric_evidence(value, home):
    own, control = fabric_paths(home)
    if (value['home'] != str(own) or value['control_socket'] != str(control)
            or not isinstance(value['status']['node_id'], str) or not value['status']['node_id']
            or not value['scope_receipt']):
        raise ValueError('fixture-fabric-refused: dedicated Fabric status and scope evidence required')
    return value


def local_health(value):
    value = value.get('value', value)
    policy = value['network_policy']
    if (value['status'] != 'ready' or not value['node']
            or policy['mode'] != 'local-only'
            or any(policy[name] is not False for name in ('fleet_join', 'replication', 'peer_relay'))):
        raise ValueError('fixture-health-refused: expected ready local-only daemon')
    return value


def validate_grant(grant, record, client, now_ms):
    """Validate saved issuance binding, NOT live authority; see README execution blockers."""
    if client not in CLIENTS:
        raise ValueError('fixture-grant-refused: unknown client')
    from datetime import datetime
    expires = datetime.fromisoformat(grant['expires_at'].replace('Z', '+00:00'))
    if (grant['state'] != 'active' or grant['actor_agent'] != seed.ACTOR
            or grant['run_id'] != record['catalog']['run_id']
            or grant['item_id'] != record['catalog']['ask_id']
            or sorted(grant['screens']) != sorted(SCREENS)
            or len(grant['screens']) != len(SCREENS)
            or sorted(grant['verbs']) != ['phone.inspect', 'phone.navigate']
            or expires.timestamp() * 1000 <= now_ms
            or not all(isinstance(grant[key], str) and grant[key]
                       for key in ('grant_id', 'device', 'session'))):
        raise ValueError('fixture-grant-refused: exact run/item/screens/verbs/expiry binding required')
    saved = record.get('grants', {}).get(client)
    if saved is not None and saved != grant:
        raise ValueError('fixture-grant-refused: issuance differs from frozen run record')
    return {'client': client, 'grant_id': grant['grant_id'],
            'item_id': grant['item_id'], 'screens': grant['screens'],
            'binding': 'saved-issuance-only', 'live_authority': 'not-verified'}


def service_evidence(value, stopped=False):
    fixture = value['fixture']
    normal = value['normal']
    if fixture['manager'] != 'systemd-user' or normal['manager'] != fixture['manager']:
        raise ValueError('fixture-service-refused: Linux systemd user evidence required')
    expected = {f'st3-instance-{seed.INSTANCE}.service',
                f'st3-instance-{seed.INSTANCE}-replication.service'}
    if ({item['name'] for item in fixture['services']} != expected
            or {item['name'] for item in normal['services']} != {'st3.service', 'st3-replication.service'}):
        raise ValueError('fixture-service-refused: exact fixture and normal unit names required')
    daemon = next(item for item in fixture['services']
                  if item['name'] == f'st3-instance-{seed.INSTANCE}.service')
    if stopped:
        if any(item['installed'] or item['running'] for item in fixture['services']):
            raise ValueError('fixture-service-refused: fixture services still installed or running')
    elif daemon['installed'] is not True or daemon['running'] is not True:
        raise ValueError('fixture-service-refused: fixture daemon must be service-managed and running')
    return value


def read_services(program):
    if os.environ.get('FRACTAL_INSTANCE') or os.environ.get('ST_INSTANCE'):
        raise ValueError('fixture-service-refused: use clean operator shell for normal-unit evidence')
    def status(*args):
        return json.loads(subprocess.run([program, *args, '--json', 'service', 'status'],
                                        text=True, capture_output=True, check=True).stdout)
    return {'fixture': status('--instance', seed.INSTANCE), 'normal': status()}


def actual_record(health, receipt, claims, items, grants, services, fabric, policy, home, socket, now_ms):
    health = local_health(health)
    fabric = fabric_evidence(fabric, home)
    if set(policy) != set(CLIENTS) or any(not policy[client].get('identity') for client in CLIENTS):
        raise ValueError('fixture-policy-refused: concrete per-client identity policy required before journeys')
    check = seed.no_read_mutation(items, claims, receipt['ask_id'])
    if check != receipt:
        raise ValueError('fixture-record-refused: seed receipt differs from current state')
    if set(grants) != set(CLIENTS):
        raise ValueError('fixture-record-refused: actual issuance resource for each client required')
    run_ids = {grant['run_id'] for grant in grants.values()}
    if len(run_ids) != 1 or not next(iter(run_ids)):
        raise ValueError('fixture-record-refused: grants must bind one actual run')
    catalog = {
        'node_id': health['node'], 'run_id': next(iter(run_ids)),
        'fabric_node_id': fabric['status']['node_id'],
        'person_id': seed.PERSON, 'actor_id': seed.ACTOR,
        'ask_id': check['ask_id'], 'attention_item_id': check['attention_id'],
        'navigation_item_id': check['ask_id'],
        'claim_ids': sorted(claim['id'] for claim in claims),
        'claim_subjects': sorted({claim['subject'] for claim in claims}),
        'grant_ids': sorted(grant['grant_id'] for grant in grants.values()),
        'device_ids': sorted(grant['device'] for grant in grants.values()),
        'session_ids': sorted(grant['session'] for grant in grants.values()),
    }
    for field in ('grant_ids', 'device_ids', 'session_ids'):
        if len(set(catalog[field])) != len(CLIENTS):
            raise ValueError('fixture-record-refused: each client requires distinct actual identity')
    record = {'schema': 'client-control-fixture-actual/v1', **digests(),
              'captured_at_unix_ms': now_ms, 'instance': seed.INSTANCE,
              'root': str(safe_root(home)), 'operator_socket': str(socket),
              'health': health, 'catalog': catalog, 'catalog_sha256': digest(canonical(catalog)),
              'baseline': check, 'claims': claims, 'attention': items, 'grants': grants,
              'services': service_evidence(services),
              'fabric': fabric,
              'observer_policy': policy, 'observer_policy_sha256': digest(canonical(policy)),
              'unread_bit': 'not-supported-for-native-asks',
              'grant_check_scope': 'verifies_binding_only; daemon enforces live authority'}
    for client in CLIENTS:
        validate_grant(grants[client], record, client, now_ms)
    return record


def validate_record(record, socket, home):
    if (record['schema'] != 'client-control-fixture-actual/v1'
            or record['instance'] != seed.INSTANCE
            or record['root'] != str(safe_root(home))
            or record['operator_socket'] != str(socket)
            or record['catalog_sha256'] != digest(canonical(record['catalog']))
            or any(record[key] != value for key, value in digests().items())):
        raise ValueError('fixture-record-refused: changed input, packet, path or catalog')
    expected = actual_record(record['health'], record['baseline'], record['claims'],
                             record['attention'], record['grants'], record['services'],
                             record['fabric'], record['observer_policy'], home, socket, record['captured_at_unix_ms'])
    if record != expected:
        raise ValueError('fixture-record-refused: evidence and catalog disagree')
    return record


def command(*args):
    return shlex.join(str(arg) for arg in args)


def preparation(home):
    root = safe_root(home)
    script = HERE / 'seed.py'
    return {**digests(), 'root': str(root), 'operator_socket': str(root / 'run/fractal.sock'),
            'authorization': 'separate authenticated consent required before ANY effects',
            'human_only_order': [
                'Review the packet, exclusions, prerequisites and authenticated consent in README.md.',
                fabric_commands(home),
                command('fractal', '--instance', seed.INSTANCE, 'setup', '--person', seed.INSTANCE),
                'Choose service-managed setup; save exact fixture and normal service status evidence.',
                command('python3', HERE / 'packet.py', 'services'),
                'Save local-only GET /v1/health and exact path evidence BEFORE seed; refuse normal/socket aliases.',
                command('python3', HERE / 'packet.py', '--socket', root / 'run/fractal.sock', 'health'),
                'Only after local-only evidence and separate seed authorization:',
                command('python3', script, '--socket', root / 'run/fractal.sock', 'seed',
                        '--idempotency-key', 'client-control-fixture-0001'),
                'Save seed JSON externally; do not alter expected-input.json.',
                'Record actual issuance resources and enrollment evidence for all three clients.',
                'Capture actual run record before any journey; daemon retains live grant authorization authority.'
            ]}


def teardown(record, socket, home):
    validate_record(record, socket, home)
    service_evidence(record['services'])
    # This prepares commands only. It neither revokes nor stops anything.
    commands = []
    for grant in record['grants'].values():
        commands.append(command('curl', '--fail-with-body', '--silent', '--show-error',
                                '--unix-socket', socket, '-H', 'Content-Type: application/json',
                                '-X', 'POST', '--data', json.dumps({'grant_id': grant['grant_id']}),
                                'http://localhost/v1/client/control-grants/revoke'))
    for device in record['catalog']['device_ids']:
        commands.append(command('curl', '--fail-with-body', '--silent', '--show-error',
                                '--unix-socket', socket, '-H', 'Content-Type: application/json',
                                '-X', 'POST', '--data', json.dumps({'device': device}),
                                'http://localhost/v1/client/test-clients/revoke'))
    return {'human_only': True, 'requires_new_authenticated_teardown_consent': True,
            'commands': commands,
            'managed_service_stop_only': command('fractal', '--instance', seed.INSTANCE, 'service', 'uninstall'),
            'dedicated_fabric_stop_after_fixture_service': fabric_commands(home)['stop'],
            'service_readback_before_and_after': command('python3', HERE / 'packet.py', 'services'),
            'normal_units_must_remain_identical': record['services']['normal'],
            'stop_gate': 'Only if fixture service status proves daemon is service-managed; unmanaged daemon stop has no shipped standalone CLI here.',
            'excluded': ['normal root', 'normal store', 'normal socket', 'normal sessions',
                         'normal grants', 'normal enrollments', 'fixture state deletion'],
            'api_gate': 'Grant/enrollment APIs depend on unlanded canonical source; do not execute until admitted.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', type=Path)
    parser.add_argument('--fractal', default='fractal')
    sub = parser.add_subparsers(dest='action', required=True)
    sub.add_parser('prepare')
    sub.add_parser('digests')
    sub.add_parser('health')
    services = sub.add_parser('services')
    services.add_argument('--record', type=Path, help='Compare normal units to frozen run record')
    services.add_argument('--expect-stopped', action='store_true')
    capture = sub.add_parser('capture')
    capture.add_argument('--seed-receipt', type=Path, required=True)
    capture.add_argument('--grants', type=Path, required=True, help='Object ios/web/tui: actual issuance resources')
    capture.add_argument('--service-evidence', type=Path, required=True)
    capture.add_argument('--fabric-evidence', type=Path, required=True)
    capture.add_argument('--observer-policy', type=Path, required=True)
    check = sub.add_parser('check')
    check.add_argument('--record', type=Path, required=True)
    check.add_argument('--client', choices=CLIENTS, required=True)
    check.add_argument('--phase', choices=('before', 'after'), required=True)
    stop = sub.add_parser('teardown-plan')
    stop.add_argument('--record', type=Path, required=True)
    args = parser.parse_args()
    home = Path(os.environ['HOME'])
    if args.action == 'digests':
        result = digests()
    elif args.action == 'prepare':
        result = preparation(home)
    elif args.action == 'services':
        safe_root(home)
        result = service_evidence(read_services(args.fractal), args.expect_stopped)
        if args.record:
            record = load(args.record)
            validate_record(record, Path(record['operator_socket']), home)
            if result['normal'] != record['services']['normal']:
                raise ValueError('normal-service-refused: normal unit state changed')
    else:
        if args.socket is None:
            parser.error('--socket is required for evidence and teardown commands')
        socket = safe_socket(args.socket, home)
        if args.action == 'teardown-plan':
            result = teardown(load(args.record), socket, home)
        else:
            fixture = seed.Fixture(args.fractal, socket)
            health = local_health(json.loads(subprocess.run(
                ['curl', '--fail-with-body', '--silent', '--show-error', '--unix-socket', str(socket),
                 'http://localhost/v1/health'], text=True, capture_output=True, check=True).stdout))
            if args.action == 'health':
                result = {**digests(), 'root': str(safe_root(home)), 'operator_socket': str(socket),
                          'health': health, 'effects': 'none; read-only evidence'}
            else:
                import time
                now_ms = int(time.time() * 1000)
                items, claims = fixture.attention(), fixture.claims()
                if args.action == 'capture':
                    result = actual_record(health, load(args.seed_receipt), claims, items,
                                           load(args.grants), load(args.service_evidence),
                                           load(args.fabric_evidence), load(args.observer_policy),
                                           home, socket, now_ms)
                else:
                    record = validate_record(load(args.record), socket, home)
                    if health['node'] != record['catalog']['node_id']:
                        raise ValueError('fixture-record-refused: node identity changed')
                    current = seed.no_read_mutation(items, claims, record['catalog']['ask_id'])
                    if current != record['baseline']:
                        raise ValueError('fixture-no-read-mutation-refused: current state differs')
                    binding = validate_grant(record['grants'][args.client], record, args.client, now_ms)
                    result = {'client': args.client, 'phase': args.phase, 'state_check': current,
                              'observed_at_unix_ms': now_ms,
                              'catalog_sha256': record['catalog_sha256'],
                              'receipt_sha256': digest(canonical(current)),
                              'grant_binding': binding, 'verifies_binding_only': True,
                              'authorization': 'Separate authenticated consent and daemon command authorization remain required.'}
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
