#!/usr/bin/env python3
"""Validate retained E1/E2/E3 artifacts offline; never collect, authorize or fabricate evidence."""
import argparse
from pathlib import Path
import re

import packet
import observer


def file_line(value):
    return isinstance(value, str) and re.fullmatch(r'[^\n]+:[1-9][0-9]*(?:-[1-9][0-9]*)?', value) is not None


def require(condition, message):
    if not condition:
        raise ValueError(message)


class Finding(ValueError):
    pass


def preserved(condition, message):
    if not condition:
        raise Finding(message)


def references(value, subject):
    if isinstance(value, dict):
        return any(references(item, subject) for item in value.values())
    if isinstance(value, list):
        return any(references(item, subject) for item in value)
    return value == subject


def validate_e1(e1, record):
    require(e1['fresh_fixture'] is True and e1['creation_provenance'], 'E1 fresh creation provenance missing')
    require(e1['root'] == record['root'], 'E1 root mismatch')
    require(0 < e1['fixture_created_at_unix_ms'] <= e1['ask_created_at_unix_ms']
            <= record['captured_at_unix_ms'] <= e1['before_first_journey_unix_ms'], 'E1 chronology missing')
    require(e1['ask_count'] == 1 and e1['ask_id'] == record['catalog']['ask_id'], 'E1 unique ask missing')
    require(e1['complete_claim_feed'] is True, 'E1 complete durable feed missing')
    require(e1['unread_bit'] == 'not-supported-for-native-asks', 'E1 unsupported unread must be explicit')
    own = sorted(record['baseline']['claim_ids'])
    require(sorted(e1['seed_claim_ids']) == own and sorted(e1['person_relevant_claim_ids']) == own,
            'E1 durable person-relevant claim IDs differ from seed')
    relevant = sorted(claim['id'] for claim in record['claims']
                      if claim['subject'] == record['catalog']['ask_id']
                      or references(claim, packet.seed.PERSON))
    require(relevant == own, 'E1 captured complete feed contains other person-relevant claims')
    require(e1['claim_feed_provenance'], 'E1 claim feed capture provenance missing')


def validate_e2(e2, record, client):
    before, after = e2['before'], e2['after']
    for phase, receipt in (('before', before), ('after', after)):
        require(receipt['client'] == client and receipt['phase'] == phase, 'E2 wrong client/phase')
        require(receipt['state_check'] == record['baseline'], 'E2 claim and receipt comparison changed')
        require(receipt['catalog_sha256'] == record['catalog_sha256'], 'E2 wrong actual catalog')
        require(receipt['receipt_sha256'] == packet.digest(packet.canonical(record['baseline'])),
                'E2 receipt digest mismatch')
        binding = receipt['grant_binding']
        require(binding['grant_id'] == record['grants'][client]['grant_id']
                and binding['item_id'] == record['catalog']['ask_id']
                and sorted(binding['screens']) == sorted(packet.SCREENS), 'E2 grant binding mismatch')
    require(0 < before['observed_at_unix_ms'] <= after['observed_at_unix_ms'], 'E2 chronology missing')
    return before['observed_at_unix_ms'], after['observed_at_unix_ms']


def primary_identity(proof, record, client):
    """Consume authoritative daemon-verified receipt evidence, not generic custom claims."""
    import base64
    require(proof['store_readback_provenance'] and proof['trusted_receipt_provenance'],
            'E3 authoritative store/verified receipt provenance missing')
    before, after = proof['enrollment_before'], proof['enrollment_after']
    preserved(before == after and before['id'] and before['kind'] == 'custom.client.test-enrolled',
            'E3 enrollment record or ID changed')
    fields = before['body']['fields']
    grant = record['grants'][client]
    require(fields['device'] == grant['device'] and fields['session'] == grant['session']
            and fields['instance'] == packet.seed.INSTANCE and fields['pairingSubject'],
            'E3 enrollment does not bind actual fixture device/session')
    public = fields['publicKey']
    require(isinstance(public, str) and public.startswith('p256:'), 'E3 P-256 public key missing')
    encoded = public.removeprefix('p256:')
    point = base64.b64decode(encoded + '=' * (-len(encoded) % 4), altchars=b'-_', validate=True)
    require(len(point) == 65 and point[0] == 4, 'E3 expected uncompressed 65-byte SEC1 point')
    fingerprint = packet.digest(point)
    preserved(proof['enrolled_fingerprint'] == fingerprint, 'E3 enrollment fingerprint mismatch')
    commands = proof['journey_command_ids']
    require(commands and len(set(commands)) == len(commands), 'E3 actual journey command catalog missing')
    observed = []
    for claim in proof['verified_receipts']:
        require(claim['kind'] == 'custom.client.command-receipt' and claim['id'],
                'E3 expected actual daemon command receipt')
        receipt = claim['body']['fields']
        signature, result = receipt['verifiedResultSignature'], receipt['signedResult']
        preserved(signature and result and signature['key'] == public and signature['signature'],
                'E3 result not verified with the one enrolled key')
        for camel, snake, expected in (
                ('device', 'device', grant['device']), ('session', 'session', grant['session']),
                ('grantId', 'grant_id', grant['grant_id']), ('runId', 'run_id', grant['run_id'])):
            require(receipt[camel] == result[snake] == expected, 'E3 signed result identity/run binding mismatch')
        require(receipt['commandId'] == result['command_id']
                and receipt['commandKind'] == result['command_kind']
                and receipt['commandKind'] in ('phone.inspect', 'phone.navigate'),
                'E3 receipt command binding mismatch')
        observed.append(receipt['commandId'])
    require(sorted(observed) == sorted(commands), 'E3 incomplete or duplicated signed journey receipts')
    return fingerprint, fields['pairingSubject']


def validate_e3(e3, record, client, journey_times):
    profile = Path(record['root']) / 'client-profiles' / client
    require(e3['profile'] == str(profile) and profile.resolve() == profile,
            'E3 normal/alternate/aliased client profile refused')
    require(e3['fresh_for_fixture'] is True and e3['never_reset'] is True
            and e3['reused_enrolled_session'] is True, 'E3 fresh fixture profile/key reuse evidence missing')
    require(e3['freshness_provenance'] and e3['enrollment_provenance'] and e3['capture_provenance'],
            'E3 actual creation/enrollment/capture provenance missing')
    require(0 < e3['profile_created_at_unix_ms'] < e3['enrolled_at_unix_ms']
            <= e3['baseline_at_unix_ms'] <= journey_times[0]
            <= journey_times[1] <= e3['after_at_unix_ms'], 'E3 chronology invalid or reset profile')
    baseline, after, diff = e3['baseline'], e3['after'], e3['diff']
    fingerprint, pairing = primary_identity(e3['primary_identity'], record, client)
    secondary = e3['secondary_identity']
    require(secondary['mode'] == ('declared-unavailable' if client == 'ios' else 'readonly-local')
            and secondary['provenance'] and secondary['source_evidence']
            and all(file_line(item) for item in secondary['source_evidence']),
            'E3 secondary identity readback or iOS declared limitation missing')
    pairing_limitation = e3['pairing_limitation']
    require(pairing_limitation['reason'] and pairing_limitation['source_evidence']
            and all(file_line(item) for item in pairing_limitation['source_evidence']),
            'E3 pairing ID exists only in fixture store; declared local limitation required')
    for capture in (baseline, after):
        require(capture['version'] == 1 and capture['client'] == client
                and capture['profile'] == str(profile)
                and capture['item_id'] == record['catalog']['ask_id'], 'E3 observer capture mismatch')
        require(capture['identity']['evidence'] and capture['identity']['device_key_fingerprint']
                and capture['identity']['pairing_id'], 'E3 identity capture provenance missing')
        preserved(capture['identity']['device_key_fingerprint'] == fingerprint
                and capture['identity']['pairing_id'] == pairing,
                'E3 local/store identity correlation mismatch')
        if client == 'tui':
            enrollment = e3['primary_identity']['enrollment_before']
            fields = enrollment['body']['fields']
            preserved(capture['identity']['public_key'] == fields['publicKey']
                      and capture['identity']['device_id'] == fields['device']
                      and capture['identity']['session_actor'] == fields['session']
                      and capture['identity']['enrollment_id'] == enrollment['id'],
                      'E3 TUI public identity differs from authoritative enrolled identity')
            require(capture['identity']['sanitized_profile_evidence'],
                    'E3 TUI safe helper export provenance missing')
        require(capture['exclusions'] and all(isinstance(key, str) and key and '*' not in key
                                            for key in capture['exclusions']), 'E3 named exclusion policy missing')
        require(capture['policy_sha256'] == record['observer_policy_sha256'],
                'E3 policy digest differs from frozen actual run policy')
        require(capture['exclusions'] == record['observer_policy'][client]['identity']
                and capture['expected_churn'] == record['observer_policy'][client]['expected_churn']
                and capture['raw_omit'] == record['observer_policy'][client]['raw_omit'],
                'E3 policy classes differ from frozen actual run policy')
    preserved(baseline['identity']['device_key_fingerprint'] == after['identity']['device_key_fingerprint']
            and baseline['identity']['pairing_id'] == after['identity']['pairing_id'],
            'E3 identity changed or re-paired')
    require(baseline['exclusions'] == after['exclusions']
            and baseline['policy_sha256'] == after['policy_sha256'], 'E3 exclusion policy changed')
    require(diff['version'] == 1 and diff['client'] == client
            and diff['item_id'] == record['catalog']['ask_id']
            and diff['device_key_fingerprint'] == baseline['identity']['device_key_fingerprint']
            and diff['pairing_id'] == baseline['identity']['pairing_id']
            and diff['exclusions'] == baseline['exclusions'], 'E3 diff identity/binding mismatch')
    require(diff == observer.difference(baseline, after), 'E3 saved diff does not match raw captures')
    if e3.get('declared_limitation'):
        limitation = e3['declared_limitation']
        require(limitation['reason'] and limitation['source_evidence']
                and all(file_line(item) for item in limitation['source_evidence']),
                'E3 no-local-state limitation requires file:line evidence')
    if diff['status'] == 'FINDING':
        return 'FINDING'
    require(diff['status'] == 'DIFF_RECORDED' and not diff['requires_review'], 'E3 diff result missing')
    return 'EVIDENCE_COMPLETE'


def validate(evidence, record):
    """Missing or inconsistent evidence keeps rows OPEN, never manufactures a pass."""
    output = {'acceptance': 'initially unseen and unmutated',
              'unread_bit': 'not-supported-for-native-asks', 'rows': {},
              'scope': 'retained-artifact validation only; not consent or independent runtime observation'}
    try:
        require(evidence['catalog_sha256'] == record['catalog_sha256'], 'wrong catalog')
        validate_e1(evidence['e1'], record)
        initial = evidence['e1']['before_first_journey_unix_ms']
        e1_error = None
    except (KeyError, TypeError, ValueError) as error:
        e1_error = str(error)
    for client in packet.CLIENTS:
        try:
            require(e1_error is None, f'E1: {e1_error}')
            row = evidence['clients'][client]
            times = validate_e2(row['e2'], record, client)
            require(initial <= times[0], 'E1 first-journey evidence follows journey')
            status = validate_e3(row['e3'], record, client, times)
            result = {'status': status, 'summary': 'E2 claim and receipt comparison unchanged, and E3 diff recorded'}
            if status == 'FINDING':
                result['report_to'] = 'Fractal Product Lead'
            output['rows'][client] = result
        except Finding as error:
            output['rows'][client] = {'status': 'FINDING', 'reason': str(error),
                                      'report_to': 'Fractal Product Lead'}
        except (KeyError, TypeError, ValueError) as error:
            output['rows'][client] = {'status': 'OPEN', 'reason': str(error)}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    record = packet.load(args.record)
    import os
    packet.validate_record(record, Path(record['operator_socket']), Path(os.environ['HOME']))
    import json
    print(json.dumps(validate(packet.load(args.evidence), record), sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
