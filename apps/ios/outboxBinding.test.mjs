import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { outboxProfileMode, pairingDeploymentId, acknowledgeOutboxNotice, restoreBoundAttempt, REPAIR_OUTBOX_NOTICE } from './outboxBinding.ts';
import { createOutboxRecord, markOutboxDispatched, observeOutbox } from '../../clients/typescript/fractal-client/outbox.ts';
import { prepareMessageSendPacket, prepareThreadStartPacket, prepareThreadReplyPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';

const issuer = { issuer: 'host/original', issuerKey: 'authority/original' };
const profile = { profileId: 'profile/avery', pairingIssuer: issuer };
const mode = () => outboxProfileMode(profile, 'member/original');
const clone = value => JSON.parse(JSON.stringify(value));
// The core exports no repository implementation. This test double enforces its CAS contract;
// serialization simulates reopen, not native SQLite durability.
const repository = () => {
  const rows = new Map();
  return {
    scope: mode().scope, reads: 0, conflict: false,
    async commit(record) { if (record.revision !== 0 || rows.has(record.id)) return false; rows.set(record.id, clone(record)); return true; },
    async compareAndSwap(revision, record) {
      if (this.conflict || rows.get(record.id)?.revision !== revision || record.revision !== revision + 1) return false;
      rows.set(record.id, clone(record)); return true;
    },
    async read(id) { this.reads++; return rows.has(id) ? clone(rows.get(id)) : undefined; },
    async list() { return clone([...rows.values()]); },
  };
};
const base = { id: 'action/original', idempotency_key: 'action/original', fence: { snapshot_id: 'snapshot/original', subject_revisions: {} } };
const signature = { signer: 'person/avery', key: 'key/original', chain: ['grant/original'], nonce: 'nonce/original', signed_at_unix_ms: 1790000000000, signature: 'signature/original' };
const packets = [
  prepareMessageSendPacket({ ...base, parameters: { to: 'agent/dotfiles/manager', content: 'image', attachments: [{ blob: 'blob/original', media_type: 'image/png' }], signature } }),
  prepareThreadStartPacket({ ...base, parameters: { agent_id: 'agent/dotfiles/manager', target_id: 'thread/original', content: 'start', title: 'title', origin: { source_id: 'inbox/original', episode: 'episode/original' }, signature } }),
  prepareThreadReplyPacket({ ...base, parameters: { target_id: 'thread/original', content: 'reply', thread_quote_id: 'message/quote', signature } }),
];

test('deployment tuple encoding is injective across delimiters, escapes and Unicode', () => {
  const inputs = [['host/a', 'b:c'], ['host/a:b', 'c'], ['host/a', 'b"c'], ['host/a', 'b\\c'], ['host/é', '雪'], ['host/é', '雪']];
  const encoded = inputs.map(([issuer, issuerKey]) => pairingDeploymentId({ issuer, issuerKey }));
  assert.equal(new Set(encoded).size, inputs.length);
  encoded.forEach((value, index) => assert.deepEqual(JSON.parse(value), inputs[index]));
  assert.deepEqual(mode().scope, { principalId: profile.profileId, memberId: 'member/original', deploymentId: JSON.stringify([issuer.issuer, issuer.issuerKey]) });
});

test('legacy profiles remain unprotected; notice acknowledgement survives reopen without granting binding', () => {
  for (const legacy of [{}, { personRootFingerprint: 'sha256:pin' }, { profileId: 'profile/ada' }, { pairingIssuer: issuer }]) {
    assert.deepEqual(outboxProfileMode(legacy, 'member/original'), { mode: 'unprotected', notice: REPAIR_OUTBOX_NOTICE });
    assert.deepEqual(outboxProfileMode(clone(acknowledgeOutboxNotice(legacy)), 'member/original'), { mode: 'unprotected', notice: undefined });
  }
  assert.equal(outboxProfileMode(profile, '').mode, 'unprotected');
  assert.equal(outboxProfileMode({ ...profile, pairingIssuer: { issuer: 'host/original', issuerKey: '' } }, 'member/original').mode, 'unprotected');
});

for (const packet of packets) test(`${packet.type}: exact packet survives repository reopen and core recovery`, async () => {
  const repo = repository();
  const original = createOutboxRecord('attempt/original', packet);
  assert.equal(await repo.commit(original), true);
  assert.equal(await repo.commit(original), false);
  const prepared = await restoreBoundAttempt(repo, mode(), original.id);
  assert.equal(prepared.state, 'bound');
  assert.deepEqual(prepared.record.packet, packet);
  assert.equal(prepared.record.mayHaveDispatched, false);
  const dispatched = markOutboxDispatched(prepared.record);
  assert.equal(await repo.compareAndSwap(0, dispatched), true);
  const recovered = await restoreBoundAttempt(repo, mode(), original.id);
  assert.equal(recovered.record.state, 'unknown');
  assert.deepEqual(recovered.record.packet, packet);
  assert.equal(markOutboxDispatched(recovered.record), undefined);
  assert.equal(observeOutbox(recovered.record, { kind: 'failure', applied: 'none' }).state, 'unknown');
});

test('each binding mismatch remains unknown without reading or adopting repository records', async () => {
  const repo = repository();
  const original = createOutboxRecord('attempt/original', packets[0]);
  await repo.commit(original);
  for (const changed of [
    outboxProfileMode({ ...profile, profileId: 'profile/ada' }, 'member/original'),
    outboxProfileMode(profile, 'member/other'),
    outboxProfileMode({ ...profile, pairingIssuer: { ...issuer, issuer: 'host/other' } }, 'member/original'),
    outboxProfileMode({ ...profile, pairingIssuer: { ...issuer, issuerKey: 'authority/other' } }, 'member/original'),
    outboxProfileMode({}, 'member/original'),
  ]) assert.equal((await restoreBoundAttempt(repo, changed, original.id)).state, 'unknown');
  assert.equal(repo.reads, 0);
  assert.deepEqual(await repo.read(original.id), original);
});

test('revision conflicts fail closed; missing records grant no packet', async () => {
  const repo = repository();
  await repo.commit(createOutboxRecord('attempt/original', packets[0]));
  await repo.compareAndSwap(0, markOutboxDispatched(await repo.read('attempt/original')));
  repo.conflict = true;
  assert.deepEqual(await restoreBoundAttempt(repo, mode(), 'attempt/original'), { state: 'unknown', reason: 'revision-conflict' });
  assert.deepEqual(await restoreBoundAttempt(repo, mode(), 'attempt/missing'), { state: 'bound', record: undefined });
});

test('repository storage failure propagates rather than reporting protected acceptance', async () => {
  const repo = repository();
  repo.read = async () => { throw new Error('storage unavailable'); };
  await assert.rejects(restoreBoundAttempt(repo, mode(), 'attempt/original'), /storage unavailable/);
});

test('saved root fingerprint pins person root, not host authority; profile save is one secure write', () => {
  const vector = JSON.parse(readFileSync(new URL('../../fixtures/clients/device-pairing-proofs-v1.json', import.meta.url)));
  const fields = vector.proofs[1].body.fields;
  const hash = key => 'sha256:' + createHash('sha256').update(key).digest('base64url');
  assert.equal(hash(fields.key), vector.fingerprint);
  assert.notEqual(fields.key, fields.issuer_key);
  assert.notEqual(hash(fields.issuer_key), vector.fingerprint);
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  assert.match(source, /const pairingIssuer = await verifyPairing/);
  assert.match(source, /profileId: Crypto.randomUUID\(\), .*pairingIssuer/);
  assert.match(source, /SecureStore.setItemAsync\(profileKey, JSON.stringify\(next\)/);
});
