"""Synthetic copied-profile policy tests; never read a real client profile or key."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location('fixture_policy', Path(__file__).with_name('policy.py'))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


def bindings_for(home):
    inventory = policy.packet.load(policy.INVENTORY)
    root = policy.packet.safe_root(home)
    result = {'inventory_sha256': policy.packet.digest(policy.INVENTORY.read_bytes()), 'clients': {}}
    for client in policy.packet.CLIENTS:
        profile = root / 'client-profiles' / client
        profile.mkdir(parents=True)
        (profile / 'synthetic-safe-export.json').write_text(json.dumps({'identity': {'value': 'synthetic-redacted'}}))
        source = inventory[client]
        row = {'profile': str(profile), 'raw_omit': [], 'bindings': [
            {'class': 'identity', 'name': source['identity'][0],
             'selector': 'synthetic-safe-export.json#/identity', 'capture_provenance': 'synthetic-test-only'}],
            'unobserved': [{'name': name, 'reason': 'synthetic-test-only absence',
                            'capture_provenance': 'synthetic-test-only', 'source_evidence': ['synthetic.rs:1']}
                           for name in source['identity'][1:]]}
        if client == 'tui':
            (profile / 'synthetic-private-profile').write_bytes(b'not JSON and MUST NOT be opened by policy')
            row.update(raw_omit=['synthetic-private-profile'], sanitized_profile_evidence='synthetic-test-only')
        result['clients'][client] = row
    return result


class PolicyTests(unittest.TestCase):
    def test_bindings_use_existing_concrete_safe_selectors(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            bindings = bindings_for(home)
            result = policy.compile_policy(bindings, home)
            self.assertEqual(result['web']['identity'], ['synthetic-safe-export.json#/identity'])
            self.assertEqual(result['tui']['raw_omit'], ['synthetic-private-profile'])
            self.assertEqual(result['ios']['expected_churn'], [])

    def test_unowned_glob_missing_and_whole_file_identity_selectors_refused(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            original = bindings_for(home)
            for field, value in [('name', 'not-in-source-inventory'),
                                 ('selector', 'synthetic-safe-export.json#/*'),
                                 ('selector', 'missing.json#/identity'),
                                 ('selector', 'synthetic-safe-export.json'),
                                 ('selector', '../normal.json#/identity'), ('capture_provenance', '')]:
                bindings = copy.deepcopy(original)
                bindings['clients']['web']['bindings'][0][field] = value
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    policy.compile_policy(bindings, home)

    def test_private_profile_must_be_omitted_and_source_inventory_pinned(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'HOME': directory}):
            home = Path(directory)
            original = bindings_for(home)
            for change in ('inventory', 'omission', 'safe-export', 'unobserved'):
                bindings = copy.deepcopy(original)
                if change == 'inventory':
                    bindings['inventory_sha256'] = 'changed'
                elif change == 'omission':
                    bindings['clients']['tui']['raw_omit'] = []
                elif change == 'safe-export':
                    bindings['clients']['tui']['sanitized_profile_evidence'] = ''
                else:
                    bindings['clients']['ios']['unobserved'] = []
                with self.subTest(change=change), self.assertRaises(ValueError):
                    policy.compile_policy(bindings, home)


if __name__ == '__main__':
    unittest.main()
