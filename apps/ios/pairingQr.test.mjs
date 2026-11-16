import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parsePairingQr } from './pairingQr.ts';

const now = 1_800_000_000_000;
const payload = { _tag: 'fractal.pairing', version: 1, gateway: 'https://EXAMPLE.com:443/fractal/', id: 'pairing/0123456789abcdef01234567', code: 'ABCD2345', expires_at_unix_ms: now + 300_000 };
const parse = patch => parsePairingQr(JSON.stringify({ ...payload, ...patch }), now);

test('valid compact payload normalizes the full gateway URL', () => {
  assert.deepEqual(parse({}), { ok: true, gateway: 'https://example.com/fractal', id: payload.id, code: payload.code, expiresAtUnixMs: payload.expires_at_unix_ms });
  assert.equal(parse({ gateway: 'http://100.64.0.1:4102/base' }).ok, true);
});

test('rejects extra fields, especially claimed fingerprints, and missing fields', () => {
  for (const field of ['extra', 'person_root_fingerprint', 'fingerprint']) assert.deepEqual(parse({ [field]: 'not trusted' }), { ok: false, reason: 'malformed' });
  for (const field of Object.keys(payload)) {
    const value = { ...payload }; delete value[field];
    assert.equal(parsePairingQr(JSON.stringify(value), now).ok, false);
  }
});

test('rejects wrong tag, version, id, code and expiry type', () => {
  for (const patch of [
    { _tag: 'other' }, { version: 2 }, { version: '1' },
    { id: 'pairing/123' }, { id: 'pairing/0123456789ABCDEF01234567' },
    { code: 'ABCD0123' }, { code: 'abcd2345' }, { code: 'ABCD23456' },
    { expires_at_unix_ms: '1800000300000' }, { expires_at_unix_ms: 1.5 },
    { expires_at_unix_ms: Number.MAX_SAFE_INTEGER + 1 },
  ]) assert.deepEqual(parse(patch), { ok: false, reason: 'malformed' });
});

test('rejects query, fragment, credentials and unsupported transport without echoing input', () => {
  for (const gateway of ['https://example.com/?code=secret', 'https://example.com/?', 'https://example.com/#', 'https://example.com/#fragment', 'https://secret@example.com', 'http://public.example.com', 'not a URL', ' https://example.com']) {
    assert.deepEqual(parse({ gateway }), { ok: false, reason: 'gateway' });
  }
});

test('expires at the exact deadline', () => {
  for (const expires_at_unix_ms of [now - 1, now]) assert.deepEqual(parse({ expires_at_unix_ms }), { ok: false, reason: 'expired' });
  assert.equal(parse({ expires_at_unix_ms: now + 1 }).ok, true);
});

test('rejects malformed JSON, non-objects and oversized input', () => {
  for (const raw of ['', '{', 'null', '[]', '1', '"text"', ' '.repeat(4097), JSON.stringify({ ...payload, gateway: `https://example.com/${'a'.repeat(2048)}` })]) {
    assert.deepEqual(parsePairingQr(raw, now), { ok: false, reason: 'malformed' });
  }
});

test('rejects surrounding whitespace and trailing line terminators', () => {
  const raw = JSON.stringify(payload);
  for (const whitespace of [' ', '\n', '\r', '\r\n', '\t', '\u2028', '\u2029']) {
    assert.deepEqual(parsePairingQr(raw + whitespace, now), { ok: false, reason: 'malformed' });
    assert.deepEqual(parsePairingQr(whitespace + raw, now), { ok: false, reason: 'malformed' });
  }
});
