import copy
import json
from pathlib import Path
import tempfile
import unittest

import observer


class ObserverTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.profile = self.home / '.st-instance/client-control-fixture/client-profiles/tui'
        self.profile.mkdir(parents=True)
        self.identity = {'device_key_fingerprint': 'fixture-key-fingerprint',
                         'pairing_id': 'fixture-pairing', 'evidence': 'test-only-identity-readback',
                         'public_key': 'test-public-key', 'device_id': 'test-device',
                         'session_actor': 'test-session-actor', 'enrollment_id': 'test-enrollment',
                         'sanitized_profile_evidence': 'test-only-helper-export'}
        self.policy = {'web': {}, 'ios': {}, 'tui': {
            'identity': ['identity.json#/session'], 'expected_churn': [],
            'raw_omit': ['config/private-profile.json']}}
        (self.profile / 'identity.json').write_text(json.dumps({'session': 'fixture-session'}))

    def capture(self):
        return observer.snapshot(self.profile, 'tui', 'attention/fixture',
                                 self.policy, self.identity, self.home)

    def test_empty_or_wildcard_exclusions_fail_closed(self):
        for exclusions in [[], ['*'], ['identity.json#/*']]:
            self.policy['tui']['identity'] = exclusions
            with self.assertRaisesRegex(ValueError, 'policy-refused'):
                self.capture()

    def test_other_normal_and_symlink_profiles_are_refused(self):
        for profile in [self.home / 'normal', self.home / '.st-instance/other/client-profiles/tui']:
            with self.assertRaisesRegex(ValueError, 'path-refused'):
                observer.isolated_profile(profile, 'tui', self.home)
        target = self.home / 'normal'
        target.mkdir()
        alias = self.profile / 'normal-alias'
        alias.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'path-refused'):
            self.capture()
        self.assertEqual(list(target.iterdir()), [])

    def test_exact_exclusion_does_not_hide_sibling_read_marker(self):
        before = self.capture()
        (self.profile / 'identity.json').write_text(json.dumps({
            'session': 'fixture-session', 'item': 'attention/fixture', 'seen': True}))
        after = self.capture()
        diff = observer.difference(before, after)
        self.assertNotIn('identity.json#/session', diff['changed'])
        self.assertIn('identity.json#/seen', diff['changed'])
        self.assertEqual(diff['status'], 'FINDING')
        self.assertEqual(diff['read_seen_findings'], ['identity.json#/seen'])
        self.assertTrue(diff['identity_preserved'])

    def test_changed_device_key_or_pairing_never_passes(self):
        before = self.capture()
        for key in ['device_key_fingerprint', 'pairing_id', 'public_key',
                    'device_id', 'session_actor', 'enrollment_id']:
            after = copy.deepcopy(before)
            after['identity'][key] = 'replacement'
            with self.subTest(key=key):
                diff = observer.difference(before, after)
                self.assertEqual(diff['status'], 'FINDING')
                self.assertFalse(diff['identity_preserved'])
                self.assertIn('identity/' + key, diff['identity_changes'])

    def test_unknown_binary_changes_require_review_not_pass(self):
        path = self.profile / 'state.bin'
        path.write_bytes(b'\xffbaseline')
        before = self.capture()
        path.write_bytes(b'\xffchanged')
        diff = observer.difference(before, self.capture())
        self.assertEqual(diff['status'], 'FINDING')
        self.assertEqual(diff['changed'], ['state.bin'])
        self.assertTrue(diff['requires_review'])

    def test_policy_and_profile_reset_refused(self):
        before = self.capture()
        for key, value in [('policy_sha256', 'replacement'), ('profile', '/new-profile')]:
            after = copy.deepcopy(before)
            after[key] = value
            with self.assertRaisesRegex(ValueError, 'comparison-refused'):
                observer.difference(before, after)

    def test_private_profile_is_never_opened_and_safe_export_is_compared(self):
        raw_profile = self.profile / 'config/private-profile.json'
        raw_profile.parent.mkdir()
        raw_profile.write_text('INVALID JSON PRIVATE PROFILE NEVER PARSED')
        (self.profile / 'identity.json').write_text(json.dumps({
            'session': {'credential': {'redacted': True, 'sha256': 'test-only-hash'}},
            'cache': {'item': 'attention/fixture', 'closed': True}}))
        before = self.capture()
        self.assertNotIn('config/private-profile.json', before['entries'])
        self.assertEqual(observer.difference(before, before)['status'], 'FINDING')

    def test_expected_churn_is_recorded_not_failure_but_scanned(self):
        self.policy['tui']['expected_churn'] = ['cache.json']
        before = self.capture()
        (self.profile / 'cache.json').write_text(json.dumps({'layout': 'changed'}))
        diff = observer.difference(before, self.capture())
        self.assertEqual(diff['status'], 'DIFF_RECORDED')
        self.assertEqual(diff['expected_churn_changes'], ['cache.json#/layout'])
        self.assertFalse(diff['requires_review'])
        (self.profile / 'cache.json').write_text(json.dumps({
            'item': 'attention/fixture', 'acted': True}))
        self.assertEqual(observer.difference(before, self.capture())['status'], 'FINDING')

    def test_string_encoded_storage_marker_and_identity_changes_are_findings(self):
        before = self.capture()
        (self.profile / 'storage.json').write_text(json.dumps({
            'st3.projection.v1': json.dumps({'item': 'attention/fixture', 'seenAt': 123})}))
        self.assertEqual(observer.difference(before, self.capture())['status'], 'FINDING')
        (self.profile / 'identity.json').write_text(json.dumps({'session': 'replacement'}))
        diff = observer.difference(before, self.capture())
        self.assertFalse(diff['identity_preserved'])
        self.assertEqual(diff['identity_changes'], ['identity.json#/session'])

    def test_unresolved_policy_is_refused_before_baseline(self):
        self.policy['web'] = {'resolved_names': ['gateway/<gw>']}
        with self.assertRaisesRegex(ValueError, 'unresolved placeholder'):
            self.capture()

    def test_expected_churn_must_be_json_at_capture(self):
        self.policy['tui']['expected_churn'] = ['readonly-store.json']
        path = self.profile / 'readonly-store.json'
        path.write_bytes(b'\xffopaque')
        with self.assertRaisesRegex(ValueError, 'read-only JSON export'):
            self.capture()
        path.write_text(json.dumps({'item': 'attention/fixture', 'seen': True}))
        self.assertEqual(observer.difference(self.capture(), self.capture())['status'], 'FINDING')


if __name__ == '__main__':
    unittest.main()
