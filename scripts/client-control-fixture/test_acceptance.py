"""Synthetic offline evidence tests, not real device observations or runtime IDs."""
import copy
import base64
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location('fixture_acceptance', Path(__file__).with_name('acceptance.py'))
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)
packet = acceptance.packet


def synthetic(root):
    record = {'root': str(root), 'captured_at_unix_ms': 4, 'catalog_sha256': 'synthetic-catalog',
              'catalog': {'ask_id': 'synthetic-ask'},
              'baseline': {'claim_ids': ['synthetic-claim'], 'check': 'no-read-mutation'},
              'claims': [{'id': 'synthetic-claim', 'subject': 'synthetic-ask'}],
              'grants': {client: {'grant_id': f'synthetic-{client}', 'device': f'synthetic-device-{client}',
                                  'session': f'synthetic-session-{client}', 'run_id': 'synthetic-run'}
                         for client in packet.CLIENTS}}
    record['observer_policy'] = {client: {'identity': ['synthetic-test-identity-key'],
                                         'expected_churn': [], 'raw_omit': []}
                                 for client in packet.CLIENTS}
    record['observer_policy_sha256'] = packet.digest(packet.canonical(record['observer_policy']))
    evidence = {'catalog_sha256': record['catalog_sha256'], 'e1': {
        'fresh_fixture': True, 'creation_provenance': 'synthetic-test-only', 'root': str(root),
        'fixture_created_at_unix_ms': 1, 'ask_created_at_unix_ms': 2,
        'before_first_journey_unix_ms': 5, 'ask_count': 1, 'ask_id': 'synthetic-ask',
        'complete_claim_feed': True, 'unread_bit': 'not-supported-for-native-asks',
        'seed_claim_ids': ['synthetic-claim'], 'person_relevant_claim_ids': ['synthetic-claim'],
        'claim_feed_provenance': 'synthetic-test-only'}, 'clients': {}}
    for client in packet.CLIENTS:
        profile = str(root / 'client-profiles' / client)
        # Synthetic encoding-shape data only; tests do not exercise cryptographic verification.
        point = b'\x04' + b'\x01' * 64
        public = 'p256:' + base64.urlsafe_b64encode(point).decode().rstrip('=')
        fingerprint = packet.digest(point)
        grant = record['grants'][client]
        enrollment = {'id': f'synthetic-enrollment-{client}', 'kind': 'custom.client.test-enrolled',
                      'body': {'fields': {'device': grant['device'], 'session': grant['session'],
                               'instance': packet.seed.INSTANCE, 'pairingSubject': f'synthetic-pair-{client}',
                               'publicKey': public}}}
        result = {'command_id': f'synthetic-command-{client}', 'command_kind': 'phone.inspect',
                  'device': grant['device'], 'session': grant['session'],
                  'grant_id': grant['grant_id'], 'run_id': grant['run_id']}
        receipt = {'id': f'synthetic-receipt-{client}', 'kind': 'custom.client.command-receipt',
                   'body': {'fields': {'commandId': result['command_id'], 'commandKind': 'phone.inspect',
                            'device': grant['device'], 'session': grant['session'],
                            'grantId': grant['grant_id'], 'runId': grant['run_id'],
                            'verifiedResultSignature': {'key': public, 'signature': 'synthetic-not-a-signature'},
                            'signedResult': result}}}
        primary = {'store_readback_provenance': 'synthetic-test-only',
                   'trusted_receipt_provenance': 'synthetic-test-only',
                   'enrollment_before': enrollment, 'enrollment_after': copy.deepcopy(enrollment),
                   'enrolled_fingerprint': fingerprint, 'journey_command_ids': [result['command_id']],
                   'verified_receipts': [receipt]}
        capture = {'version': 1, 'client': client, 'profile': profile, 'item_id': 'synthetic-ask',
                   'identity': {'device_key_fingerprint': fingerprint,
                                'pairing_id': f'synthetic-pair-{client}', 'evidence': 'synthetic-test-only',
                                'public_key': public, 'device_id': grant['device'],
                                'session_actor': grant['session'], 'enrollment_id': enrollment['id'],
                                'sanitized_profile_evidence': 'synthetic-safe-export-provenance'},
                   'exclusions': ['synthetic-test-identity-key'],
                   'policy_sha256': record['observer_policy_sha256'],
                   'expected_churn': [], 'raw_omit': [], 'opaque_files': [],
                   'entries': {'synthetic-test-identity-key': 'synthetic-hash'}, 'read_seen_markers': []}
        diff = acceptance.observer.difference(capture, capture)
        e2 = {}
        for phase, observed in [('before', 10), ('after', 20)]:
            e2[phase] = {'client': client, 'phase': phase, 'state_check': record['baseline'],
                         'catalog_sha256': record['catalog_sha256'],
                         'receipt_sha256': packet.digest(packet.canonical(record['baseline'])),
                         'observed_at_unix_ms': observed,
                         'grant_binding': {'grant_id': f'synthetic-{client}', 'item_id': 'synthetic-ask',
                                           'screens': ['NowRoot', 'Attention']}}
        evidence['clients'][client] = {'e2': e2, 'e3': {
            'profile': profile, 'fresh_for_fixture': True, 'never_reset': True,
            'primary_identity': primary,
            'secondary_identity': {'mode': 'declared-unavailable' if client == 'ios' else 'readonly-local',
                                   'provenance': 'synthetic-test-only', 'source_evidence': ['synthetic.rs:1']},
            'pairing_limitation': {'reason': 'synthetic-test-only no-local-pairing-id',
                                   'source_evidence': ['synthetic.rs:2']},
            'reused_enrolled_session': True, 'freshness_provenance': 'synthetic-test-only',
            'enrollment_provenance': 'synthetic-test-only', 'capture_provenance': 'synthetic-test-only',
            'profile_created_at_unix_ms': 1, 'enrolled_at_unix_ms': 3, 'baseline_at_unix_ms': 4,
            'after_at_unix_ms': 21, 'baseline': copy.deepcopy(capture), 'after': copy.deepcopy(capture),
            'diff': copy.deepcopy(diff)}}
    return record, evidence


class AcceptanceTests(unittest.TestCase):
    def test_missing_artifacts_keep_every_row_open(self):
        record, _ = synthetic(Path('/synthetic-fixture'))
        result = acceptance.validate({}, record)
        self.assertEqual({row['status'] for row in result['rows'].values()}, {'OPEN'})
        self.assertEqual(result['unread_bit'], 'not-supported-for-native-asks')

    def test_complete_synthetic_artifacts_are_not_runtime_authorization(self):
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve()) as directory:
            record, evidence = synthetic(Path(directory))
            result = acceptance.validate(evidence, record)
            self.assertEqual({row['status'] for row in result['rows'].values()}, {'EVIDENCE_COMPLETE'})
            self.assertIn('not consent', result['scope'])
            self.assertNotIn('PASS', str(result))

    def test_creation_provenance_claim_scope_and_chronology_required(self):
        record, evidence = synthetic(Path('/synthetic-fixture'))
        for field, value in [('fresh_fixture', False), ('person_relevant_claim_ids', ['other-claim']),
                             ('ask_count', 2), ('unread_bit', 'PASS'), ('complete_claim_feed', False),
                             ('ask_created_at_unix_ms', 9), ('creation_provenance', '')]:
            changed = copy.deepcopy(evidence)
            changed['e1'][field] = value
            with self.subTest(field=field):
                self.assertEqual({row['status'] for row in acceptance.validate(changed, record)['rows'].values()},
                                 {'OPEN'})

    def test_each_client_needs_own_before_after_receipts(self):
        record, evidence = synthetic(Path('/synthetic-fixture'))
        for client in packet.CLIENTS:
            changed = copy.deepcopy(evidence)
            changed['clients'][client]['e2']['after']['state_check'] = {'changed': True}
            result = acceptance.validate(changed, record)
            self.assertEqual(result['rows'][client]['status'], 'OPEN')
            self.assertEqual(sum(row['status'] == 'EVIDENCE_COMPLETE' for row in result['rows'].values()), 2)

    def test_marker_is_finding_and_unknown_change_requires_review(self):
        record, evidence = synthetic(Path('/synthetic-fixture'))
        for source in ('baseline', 'after'):
            changed = copy.deepcopy(evidence)
            e3 = changed['clients']['web']['e3']
            e3[source]['read_seen_markers'] = ['synthetic-item-seen-marker']
            e3['diff'] = acceptance.observer.difference(e3['baseline'], e3['after'])
            row = acceptance.validate(changed, record)['rows']['web']
            self.assertEqual(row['status'], 'FINDING')
            self.assertEqual(row['report_to'], 'Fractal Product Lead')
        e3 = evidence['clients']['ios']['e3']
        e3['after']['entries']['synthetic-other-file'] = 'synthetic-changed-hash'
        e3['diff'] = acceptance.observer.difference(e3['baseline'], e3['after'])
        self.assertEqual(acceptance.validate(evidence, record)['rows']['ios']['status'], 'FINDING')

    def test_profile_reset_identity_changes_unsafe_profiles_and_missing_policy_refused(self):
        record, evidence = synthetic(Path('/synthetic-fixture'))
        for field, value in [('never_reset', False), ('reused_enrolled_session', False),
                             ('profile', '/normal/profile'), ('fresh_for_fixture', False),
                             ('enrolled_at_unix_ms', 0)]:
            changed = copy.deepcopy(evidence)
            changed['clients']['tui']['e3'][field] = value
            with self.subTest(field=field):
                self.assertEqual(acceptance.validate(changed, record)['rows']['tui']['status'], 'OPEN')
        for field, value in [('pairing_id', 'changed'), ('device_key_fingerprint', 'changed')]:
            changed = copy.deepcopy(evidence)
            changed['clients']['ios']['e3']['after']['identity'][field] = value
            self.assertEqual(acceptance.validate(changed, record)['rows']['ios']['status'], 'FINDING')
        for exclusions in ([], ['*'], ['identity/*']):
            changed = copy.deepcopy(evidence)
            changed['clients']['web']['e3']['baseline']['exclusions'] = exclusions
            self.assertEqual(acceptance.validate(changed, record)['rows']['web']['status'], 'OPEN')

    def test_declared_limitation_requires_file_line_evidence(self):
        record, evidence = synthetic(Path('/synthetic-fixture'))
        limitation = {'reason': 'synthetic test-only declared limitation', 'source_evidence': ['missing-line']}
        evidence['clients']['tui']['e3']['declared_limitation'] = limitation
        self.assertEqual(acceptance.validate(evidence, record)['rows']['tui']['status'], 'OPEN')
        limitation['source_evidence'] = ['synthetic-test.rs:42']
        self.assertEqual(acceptance.validate(evidence, record)['rows']['tui']['status'], 'EVIDENCE_COMPLETE')

    def test_primary_identity_requires_authoritative_receipts_and_stable_enrollment(self):
        record, original = synthetic(Path('/synthetic-fixture'))
        for field in ('store_readback_provenance', 'trusted_receipt_provenance'):
            evidence = copy.deepcopy(original)
            evidence['clients']['web']['e3']['primary_identity'][field] = ''
            self.assertEqual(acceptance.validate(evidence, record)['rows']['web']['status'], 'OPEN')
        evidence = copy.deepcopy(original)
        evidence['clients']['web']['e3']['primary_identity']['enrollment_after']['id'] = 'changed'
        self.assertEqual(acceptance.validate(evidence, record)['rows']['web']['status'], 'FINDING')
        evidence = copy.deepcopy(original)
        proof = evidence['clients']['web']['e3']['primary_identity']
        proof['verified_receipts'][0]['body']['fields']['verifiedResultSignature']['key'] = 'another-key'
        self.assertEqual(acceptance.validate(evidence, record)['rows']['web']['status'], 'FINDING')
        evidence = copy.deepcopy(original)
        evidence['clients']['web']['e3']['primary_identity']['verified_receipts'] = []
        self.assertEqual(acceptance.validate(evidence, record)['rows']['web']['status'], 'OPEN')

    def test_policy_digest_and_secondary_identity_limitations_are_required(self):
        record, original = synthetic(Path('/synthetic-fixture'))
        evidence = copy.deepcopy(original)
        evidence['clients']['web']['e3']['after']['policy_sha256'] = 'changed'
        self.assertEqual(acceptance.validate(evidence, record)['rows']['web']['status'], 'OPEN')
        evidence = copy.deepcopy(original)
        evidence['clients']['ios']['e3']['secondary_identity']['mode'] = 'readonly-local'
        self.assertEqual(acceptance.validate(evidence, record)['rows']['ios']['status'], 'OPEN')
        evidence = copy.deepcopy(original)
        evidence['clients']['tui']['e3']['pairing_limitation']['source_evidence'] = []
        self.assertEqual(acceptance.validate(evidence, record)['rows']['tui']['status'], 'OPEN')


if __name__ == '__main__':
    unittest.main()
