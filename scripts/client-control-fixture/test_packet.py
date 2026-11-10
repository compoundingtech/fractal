"""Source-only tests. All identifiers below are synthetic test values, never runtime records."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location('fixture_packet', Path(__file__).with_name('packet.py'))
packet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packet)
seed = packet.seed


def evidence():
    ask = 'step-run/synthetic-test/ask'
    items = [{'id': 'attention/synthetic-test', 'source_id': ask, 'person_id': seed.PERSON,
              'attention_kind': 'person-step', 'state': 'open', 'title': seed.TITLE,
              'presentation': {'kind': 'request'}, 'request': {'question': seed.BODY}}]
    claims = [{'subject': ask, 'id': 'claim/synthetic-request', 'kind': 'work.person-asked',
               'actor': seed.ACTOR, 'body': {'fields': {'person': seed.PERSON,
               'title': seed.TITLE, 'request': {'question': seed.BODY}}}}]
    grants = {client: {'grant_id': f'synthetic-grant-{client}', 'run_id': 'run/synthetic',
                      'device': f'synthetic-device-{client}', 'session': f'synthetic-session-{client}',
                      'actor_agent': seed.ACTOR, 'verbs': ['phone.inspect', 'phone.navigate'],
                      'screens': ['NowRoot', 'Attention'], 'item_id': ask, 'state': 'active',
                      'expires_at': '2099-01-01T00:00:00Z'} for client in packet.CLIENTS}
    health = {'node': 'synthetic-node', 'status': 'ready', 'network_policy': {
        'mode': 'local-only', 'fleet_join': False, 'replication': False, 'peer_relay': False}}
    services = {'fixture': {'manager': 'systemd-user', 'services': [
        {'name': f'st3-instance-{seed.INSTANCE}.service', 'installed': True,
         'running': True, 'state': 'active'},
        {'name': f'st3-instance-{seed.INSTANCE}-replication.service', 'installed': False,
         'running': False, 'state': 'not-found'}]},
        'normal': {'manager': 'systemd-user', 'services': [
            {'name': 'st3.service', 'installed': True, 'running': True, 'state': 'active'},
            {'name': 'st3-replication.service', 'installed': True, 'running': True, 'state': 'active'}]}}
    return ask, items, claims, grants, health, services


class PacketTests(unittest.TestCase):
    def test_manifest_is_fixed_input_not_actual_ids(self):
        raw = (packet.HERE / 'expected-input.json').read_text()
        pairs = []
        value = json.loads(raw, object_pairs_hook=lambda items: pairs.extend(items) or dict(items))
        self.assertEqual(value, packet.manifest())
        # A separate duplicate-key decoder guards the immutable JSON contract.
        def unique(items):
            self.assertEqual(len(items), len(dict(items)))
            return dict(items)
        json.loads(raw, object_pairs_hook=unique)
        self.assertTrue(all(value is None for value in value['generated'].values()))
        self.assertEqual(set(value['generated']), set(value['generated_schema']) - {'navigation_item_id'})
        self.assertEqual(value['body_sha256'], packet.digest(seed.BODY.encode('ascii')))
        self.assertEqual(value['screens'], ['NowRoot', 'Attention'])
        self.assertEqual(value['unread_bit'], 'not-supported-for-native-asks')
        self.assertEqual(len(packet.digests()['input_sha256']), 64)

    def test_text_restrictions(self):
        for value in (seed.TITLE, seed.BODY, 'x' * 60):
            self.assertEqual(seed.validate_text(value), value)
        for value in ('', 'x' * 61, ' leading', 'trailing ', 'two  spaces', '\n', '{}',
                      '123', 'true', 'nonascii é', *('a' + char for char in '*_#[]`>|')):
            with self.subTest(value=value), self.assertRaises(ValueError):
                seed.validate_text(value)

    def test_health_refuses_network_enabled_or_unready(self):
        health = evidence()[4]
        self.assertEqual(packet.local_health(health), health)
        for field in ('fleet_join', 'replication', 'peer_relay', 'mode'):
            bad = copy.deepcopy(health)
            bad['network_policy'][field] = 'fleet-capable' if field == 'mode' else True
            with self.subTest(field=field), self.assertRaises(ValueError):
                packet.local_health(bad)
        bad = copy.deepcopy(health)
        bad['status'] = 'starting'
        with self.assertRaises(ValueError):
            packet.local_health(bad)

    def test_every_client_binding_requires_exact_item_and_both_screens(self):
        ask, _, _, grants, _, _ = evidence()
        record = {'catalog': {'run_id': 'run/synthetic', 'ask_id': ask}}
        for client in packet.CLIENTS:
            valid = packet.validate_grant(grants[client], record, client, 0)
            self.assertEqual(valid['live_authority'], 'not-verified')
            for field, values in {
                'screens': [['NowRoot'], ['Attention'], ['NowRoot', 'Attention', 'Settings'],
                            ['NowRoot', 'NowRoot']],
                'item_id': ['attention/synthetic-test', 'another-ask', None],
                'run_id': ['another-run'], 'actor_agent': ['agent/normal'],
                'state': ['revoked', 'expired'], 'verbs': [['phone.inspect'], ['phone.navigate', 'phone.answer']],
                'expires_at': ['1970-01-01T00:00:00Z'], 'grant_id': [''], 'device': [''], 'session': ['']
            }.items():
                for value in values:
                    bad = copy.deepcopy(grants[client])
                    bad[field] = value
                    with self.subTest(client=client, field=field, value=value), self.assertRaises(ValueError):
                        packet.validate_grant(bad, record, client, 0)

    def test_service_management_refuses_wrong_or_unmanaged_units(self):
        services = evidence()[5]
        self.assertEqual(packet.service_evidence(services), services)
        for field, value in [('installed', False), ('running', False), ('name', 'st3.service')]:
            bad = copy.deepcopy(services)
            bad['fixture']['services'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                packet.service_evidence(bad)
        with self.assertRaises(ValueError):
            packet.service_evidence(services, stopped=True)
        stopped = copy.deepcopy(services)
        for item in stopped['fixture']['services']:
            item.update(installed=False, running=False, state='not-found')
        self.assertEqual(packet.service_evidence(stopped, stopped=True), stopped)

    def test_catalog_is_actual_separate_and_tamper_evident(self):
        ask, items, claims, grants, health, services = evidence()
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve()) as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            path = seed.instance_root(home) / 'run/fractal.sock'
            fabric_home, control = packet.fabric_paths(home)
            fabric = {'home': str(fabric_home), 'control_socket': str(control),
                      'status': {'node_id': 'synthetic-fabric-node'}, 'scope_receipt': 'synthetic-test-scope'}
            raw_input = (packet.HERE / 'expected-input.json').read_bytes()
            policy = {client: {'identity': ['synthetic.json#/identity'], 'expected_churn': [], 'raw_omit': []}
                      for client in packet.CLIENTS}
            record = packet.actual_record(health, seed.no_read_mutation(items, claims, ask),
                                          claims, items, grants, services, fabric, policy, home, path, 0)
            self.assertEqual(packet.validate_record(record, path, home), record)
            self.assertEqual(record['catalog']['navigation_item_id'], ask)
            self.assertNotEqual(record['catalog']['attention_item_id'], ask)
            self.assertEqual(record['catalog']['claim_subjects'], [ask])
            self.assertEqual(record['catalog_sha256'], packet.digest(packet.canonical(record['catalog'])))
            self.assertEqual((packet.HERE / 'expected-input.json').read_bytes(), raw_input)
            for field in ('ask_id', 'attention_item_id', 'run_id', 'node_id'):
                changed = copy.deepcopy(record)
                changed['catalog'][field] = 'synthetic-tampered'
                with self.subTest(field=field), self.assertRaises(ValueError):
                    packet.validate_record(changed, path, home)
            changed = copy.deepcopy(record)
            changed['grants']['ios']['item_id'] = 'synthetic-other-ask'
            with self.assertRaises(ValueError):
                packet.validate_record(changed, path, home)
            plan = packet.teardown(record, path, home)
            self.assertEqual(len(plan['commands']), 6)
            self.assertEqual(plan['managed_service_stop_only'],
                             'fractal --instance client-control-fixture service uninstall')
            self.assertTrue(plan['requires_new_authenticated_teardown_consent'])
            self.assertNotIn('uninstall --yes', json.dumps(plan))
            with self.assertRaises(ValueError):
                packet.teardown(record, home / 'run/fractal.sock', home)

    def test_dedicated_fabric_paths_commands_and_refusals(self):
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve()) as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            own, control = packet.fabric_paths(home)
            plan = packet.fabric_commands(home)
            self.assertIn(f'FABRIC_HOME={own}', plan['start'])
            self.assertIn(f'--home {own} up --foreground', plan['start'])
            self.assertTrue(plan['stop'].endswith(f'--home {own} down'))
            valid = {'home': str(own), 'control_socket': str(control),
                     'status': {'node_id': 'synthetic-fabric-node'}, 'scope_receipt': 'synthetic-test-scope'}
            self.assertEqual(packet.fabric_evidence(valid, home), valid)
            for key, value in [('home', str(home / '.fabric')),
                               ('control_socket', str(home / '.fabric/run/control.sock'))]:
                invalid = copy.deepcopy(valid)
                invalid[key] = value
                with self.subTest(key=key), self.assertRaises(ValueError):
                    packet.fabric_evidence(invalid, home)
            own.parent.mkdir(parents=True)
            own.symlink_to(home)
            with self.assertRaises(ValueError):
                packet.fabric_paths(home)

    def test_socket_normal_client_alternate_and_symlink_refusals(self):
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve()) as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            own = seed.instance_root(home) / 'run/fractal.sock'
            own.parent.mkdir(parents=True)
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(own))
                self.assertEqual(packet.safe_socket(own, home), own)
                for candidate in (home / 'run/fractal.sock', own.with_name('fractal-client.sock'),
                                  home / '.st-instance/other/run/fractal.sock', home / 'outside.sock'):
                    with self.subTest(path=candidate), self.assertRaises(ValueError):
                        packet.safe_socket(candidate, home)
            own.unlink()
            own.symlink_to(home / 'normal.sock')
            with self.assertRaises(ValueError):
                packet.safe_socket(own, home)
            own.unlink()
            own.parent.rmdir()
            (own.parent).symlink_to(home)
            with self.assertRaises(ValueError):
                packet.safe_root(home)


if __name__ == '__main__':
    unittest.main()
