import assert from 'node:assert/strict';
import { test } from 'node:test';
import { scanOutcome } from './pairingScan.ts';

const scanned = { ok: true, gateway: 'https://example.com/base', id: 'pairing/0123456789abcdef01234567', code: 'ABCD2345', expiresAtUnixMs: 1_800_000_300_000 };
const candidate = { gateway: scanned.gateway, id: scanned.id, code: scanned.code };

test('scan cancellation is unchanged, including with an existing gateway', () => {
  assert.deepEqual(scanOutcome(null, 'https://saved.example'), { kind: 'unchanged' });
});

test('same normalized full gateway and first pairing prefill only candidate fields', () => {
  for (const gateway of ['', 'https://EXAMPLE.com:443/base///']) {
    assert.deepEqual(scanOutcome(scanned, gateway), { kind: 'prefill', candidate });
  }
  assert.deepEqual(Object.keys(scanOutcome(scanned, '').candidate), ['gateway', 'id', 'code']);
});

test('host, base path, scheme, port and Fabric changes all require confirmation', () => {
  for (const gateway of ['https://other.example/base', 'https://example.com/other', 'https://example.com', 'https://example.com:444/base', 'http://example.com/base', 'fabric:member']) {
    assert.deepEqual(scanOutcome(scanned, gateway), { kind: 'confirm-gateway', candidate });
  }
});

test('invalid scans leave form untouched and report only generic issues', () => {
  for (const reason of ['malformed', 'expired', 'gateway']) {
    const outcome = scanOutcome({ ok: false, reason }, 'https://saved.example');
    assert.equal(outcome.kind, 'issue');
    assert.equal('candidate' in outcome, false);
    assert.equal(outcome.text.includes(scanned.code), false);
    assert.equal(outcome.text.includes(scanned.id), false);
  }
});
