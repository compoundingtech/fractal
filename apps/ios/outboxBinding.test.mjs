import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { outboxProfileMode, acknowledgeOutboxNotice, restoreBoundAttempt, restoreBoundEntries, requireOutboxAuthorization, OutboxBindingUnavailableError, REPAIR_OUTBOX_NOTICE } from './outboxBinding.ts';
import { createOutboxRecord, prepareOutboxRecord, markOutboxDispatched, observeOutbox, outboxDeploymentId } from '../../clients/typescript/fractal-client/outbox.ts';
import { prepareMessageSendPacket, prepareThreadStartPacket, prepareThreadReplyPacket, expectedPreparedPacketIdentities } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { createMemoryOutboxRepository } from '../../clients/typescript/fractal-client/testSupport.ts';

const issuer = { issuer: 'host/original', issuerKey: 'authority/original' };
const profile = { profileId: 'profile/avery', pairingIssuer: issuer };
const capabilities = { issuer: issuer.issuer, issuer_key: issuer.issuerKey };
const mode = () => outboxProfileMode(profile, 'member/original', capabilities);
const clone = value => JSON.parse(JSON.stringify(value));
// The shared reference repository models CAS, not native SQLite durability.
const repository = () => createMemoryOutboxRepository(mode().scope);
const base = { id: 'action/original', idempotency_key: 'action/original', fence: { snapshot_id: 'snapshot/original', subject_revisions: {} } };
const signature = { signer: 'person/avery', key: 'key/original', chain: ['grant/original'], nonce: 'nonce/original', signed_at_unix_ms: 1790000000000, signature: 'signature/original' };
const packets = [
  prepareMessageSendPacket({ ...base, parameters: { to: 'agent/dotfiles/manager', content: 'image', attachments: [{ blob: 'blob/original', media_type: 'image/png' }], signature } }),
  prepareThreadStartPacket({ ...base, parameters: { agent_id: 'agent/dotfiles/manager', target_id: 'thread/original', content: 'start', title: 'title', origin: { source_id: 'inbox/original', episode: 'episode/original' }, signature } }),
  prepareThreadReplyPacket({ ...base, parameters: { target_id: 'thread/original', content: 'reply', thread_quote_id: 'message/quote', signature } }),
];
// References stand for future private iOS spool entries; no attachment bytes are stored here.
const intentFor = packet => ({
  text: packet.parameters.content,
  attachments: packet.type === 'message.send'
    ? [{ contentHash: 'sha256:original', mediaType: 'image/png', reference: 'spool/original' }] : [],
});
const identitiesFor = packet => expectedPreparedPacketIdentities(packet, 'person/avery',
  async bytes => new Uint8Array(createHash('sha256').update(bytes).digest()));

test('deployment tuple encoding is injective across delimiters, escapes and Unicode', () => {
  const inputs = [['host/a', 'b:c'], ['host/a:b', 'c'], ['host/a', 'b"c'], ['host/a', 'b\\c'], ['host/é', '雪'], ['host/é', '雪']];
  const encoded = inputs.map(([issuer, issuerKey]) => outboxDeploymentId(issuer, issuerKey));
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
  const intent = intentFor(packet);
  const original = createOutboxRecord('attempt/original', intent);
  assert.equal(await repo.commit(original), true);
  assert.equal(await repo.commit(original), false);
  const preparing = await restoreBoundAttempt(repo, mode(), original.id);
  assert.equal(preparing.state, 'bound');
  assert.equal(preparing.record.phase, 'preparing');
  assert.deepEqual(preparing.record.intent, intent);
  assert.equal('packet' in preparing.record, false);
  assert.equal(markOutboxDispatched(preparing.record), undefined);
  const identities = await identitiesFor(packet);
  const prepared = prepareOutboxRecord(preparing.record, packet, identities);
  assert.equal(await repo.compareAndSwap(0, prepared), true);
  const reopened = await restoreBoundAttempt(repo, mode(), original.id);
  assert.equal(reopened.record.phase, 'ready');
  assert.deepEqual(reopened.record.intent, intent);
  assert.deepEqual(reopened.record.packet, packet);
  assert.deepEqual(reopened.record.expectedIdentities, identities);
  assert.equal(await repo.compareAndSwap(1, { ...reopened.record, revision: 2, intent: { ...intent, text: 'changed' } }), false);
  const dispatched = markOutboxDispatched(reopened.record);
  assert.equal(await repo.compareAndSwap(1, dispatched), true);
  const recovered = await restoreBoundAttempt(repo, mode(), original.id);
  assert.equal(recovered.record.state, 'unknown');
  assert.deepEqual(recovered.record.packet, packet);
  assert.equal(markOutboxDispatched(recovered.record), undefined);
  assert.equal(observeOutbox(recovered.record, { kind: 'failure', applied: 'none' }).state, 'unknown');
  assert.equal(observeOutbox(recovered.record, { kind: 'observed', identity: 'message/not-original', source: 'timeline' }).state, 'unknown');
  const delivered = observeOutbox(recovered.record, { kind: 'observed', identity: identities[0], source: 'timeline' });
  assert.equal(delivered.state, 'delivered');
  assert.equal(await repo.compareAndSwap(recovered.record.revision, delivered), true);
  assert.deepEqual(delivered.intent, intent);
  assert.deepEqual(delivered.packet, packet);
});

test('each binding mismatch remains unknown without reading or adopting repository records', async () => {
  const repo = repository();
  const original = createOutboxRecord('attempt/original', intentFor(packets[0]));
  await repo.commit(original);
  const read = repo.read;
  repo.read = async () => assert.fail('a mismatched binding must not read records');
  for (const changed of [
    outboxProfileMode({ ...profile, profileId: 'profile/ada' }, 'member/original'),
    outboxProfileMode(profile, 'member/other'),
    outboxProfileMode({ ...profile, pairingIssuer: { ...issuer, issuer: 'host/other' } }, 'member/original'),
    outboxProfileMode({ ...profile, pairingIssuer: { ...issuer, issuerKey: 'authority/other' } }, 'member/original'),
    outboxProfileMode({}, 'member/original'),
  ]) assert.equal((await restoreBoundAttempt(repo, changed, original.id)).state, 'unknown');
  assert.deepEqual(await read(original.id), original);
});

test('revision conflicts fail closed; missing records grant no packet', async () => {
  const repo = repository();
  const original = createOutboxRecord('attempt/original', intentFor(packets[0]));
  await repo.commit(original);
  const ready = prepareOutboxRecord(original, packets[0], await identitiesFor(packets[0]));
  await repo.compareAndSwap(0, ready);
  await repo.compareAndSwap(1, markOutboxDispatched(ready));
  repo.compareAndSwap = async () => false;
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

test('missing, partial or mismatched live issuer blocks protected recovery and dispatch without retargeting visible entries', async () => {
  const repo = repository();
  const original = createOutboxRecord('attempt/original', intentFor(packets[0]));
  await repo.commit(original);
  const before = clone(profile);
  for (const caps of [undefined, null, {}, { issuer: issuer.issuer }, { issuer_key: issuer.issuerKey },
    { ...capabilities, issuer: 'host/other' }, { ...capabilities, issuer_key: 'authority/other' }]) {
    const blocked = outboxProfileMode(profile, 'member/original', caps);
    assert.equal(blocked.authorized, false);
    assert.equal(blocked.notice, REPAIR_OUTBOX_NOTICE);
    assert.deepEqual(blocked.scope, mode().scope);
    assert.throws(() => requireOutboxAuthorization(blocked, repo.scope), OutboxBindingUnavailableError);
    assert.equal((await restoreBoundAttempt(repo, blocked, original.id)).state, 'unknown');
    assert.deepEqual(await restoreBoundEntries(repo, blocked), [{ _tag: 'record', record: original }]);
    assert.deepEqual(await repo.read(original.id), original);
    assert.deepEqual(profile, before);
    assert.equal(outboxProfileMode(acknowledgeOutboxNotice(profile), 'member/original', caps).notice, undefined);
  }
  requireOutboxAuthorization(mode(), repo.scope);
  assert.throws(() => requireOutboxAuthorization(mode(), { ...repo.scope, memberId: 'member/other' }), OutboxBindingUnavailableError);
  repo.list = async () => [{ _tag: 'corrupt', id: 'attempt/damaged', revision: 3 }];
  const [damage] = await restoreBoundEntries(repo, outboxProfileMode(profile, 'member/original', {}));
  assert.equal(damage.state, 'unknown');
  assert.deepEqual(damage.actions, ['discard']);
});

test('store binds issuer comparison to live client/profile and current capabilities epoch; notice metadata never rewrites pairing', () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  assert.match(source, /if \(!current\(\) \|\| profileKey !== signingKeyRef.current\) return/);
  assert.match(source, /liveOutboxClient.current === client/);
  assert.match(source, /capsClient.current === client/);
  assert.match(source, /requireOutboxAuthorization\(outboxProfileMode/);
  assert.match(source, /AsyncStorage.setItem\(key, 'seen'\)/);
  assert.match(source, /Alert.alert\('Re-pair this phone', mode.notice\)/);
  const native = readFileSync(new URL('./outboxSqliteNative.ts', import.meta.url), 'utf8');
  assert.match(native, /commit: record => \{ authorize\(scope\)/);
  assert.match(native, /compareAndSwap: \(revision, record\) => \{ authorize\(scope\)/);
});
