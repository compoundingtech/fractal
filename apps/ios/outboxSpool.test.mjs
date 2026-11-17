import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createMemoryOutboxRepository } from '../../clients/typescript/fractal-client/testSupport.ts';
import { acceptProtectedOutboxIntent, readOutboxAttachment, requireProtectedOutboxStorage, openProtectedOutboxStorage, OutboxStorageProtectionError } from './outboxSpool.ts';

const directory = '/private/app/Documents/SQLite/fractal-outbox';
const scope = { principalId: 'profile/avery', deploymentId: '["host/original","key/original"]', memberId: 'member/original' };
const sha256 = async bytes => new Uint8Array(createHash('sha256').update(bytes).digest());
const image = (text = 'original bytes') => ({ key: text, base64: Buffer.from(text).toString('base64'), bytes: Buffer.byteLength(text), mediaType: 'image/png', name: 'private.png' });
const unavailable = error => error instanceof OutboxStorageProtectionError && error.mode === 'unprotected-storage' && error.keepDraft === true;

// Fake bridge exercises JS policy only. Actual Swift file flags/sync require the later build.
const setup = () => {
  const events = [], files = new Map();
  const protection = { databaseDirectory: directory, spoolDirectory: directory + '/spool', databaseExcludedFromBackup: true, spoolExcludedFromBackup: true };
  const bridge = {
    async prepareStorage() { events.push('read-back'); return { ...protection }; },
    async writeAttachment(base64, contentHash) {
      events.push('write');
      const reference = 'spool/' + contentHash.slice(7);
      if (files.has(reference)) assert.equal(files.get(reference), base64);
      else files.set(reference, base64);
      return { reference, contentHash, bytes: Buffer.from(base64, 'base64').length };
    },
    async readAttachment(reference) { events.push('read'); return files.get(reference); },
  };
  const repository = createMemoryOutboxRepository(scope);
  const commit = repository.commit;
  repository.commit = async record => { events.push('commit'); return commit(record); };
  return { bridge, repository, protection, events, files };
};

test('repository open requires both backup exclusion read-backs before opening SQLite', async () => {
  const t = setup();
  assert.equal(await openProtectedOutboxStorage(t.bridge, async proof => { t.events.push('open'); assert.equal(proof.databaseDirectory, directory); return 'opened'; }), 'opened');
  assert.deepEqual(t.events, ['read-back', 'open']);
  for (const bridge of [undefined, null, { ...t.bridge, prepareStorage: async () => { throw new Error('native unavailable'); } }]) {
    await assert.rejects(openProtectedOutboxStorage(bridge, async () => assert.fail('must not open')), unavailable);
  }
  for (const field of ['databaseExcludedFromBackup', 'spoolExcludedFromBackup']) {
    const broken = { ...t.bridge, prepareStorage: async () => ({ ...t.protection, [field]: false }) };
    await assert.rejects(openProtectedOutboxStorage(broken, async () => assert.fail('must not open')), unavailable);
  }
});

test('mismatched directory/read-back fails closed without file writes or intent commit', async () => {
  for (const change of [{ databaseDirectory: '/another/root' }, { spoolDirectory: '/another/spool' }, { databaseExcludedFromBackup: false }, { spoolExcludedFromBackup: false }]) {
    const t = setup(); Object.assign(t.protection, change);
    await assert.rejects(acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/original', 'draft', [image()]), unavailable);
    assert.deepEqual(t.events, ['read-back']);
    assert.deepEqual(await t.repository.list(), []);
  }
});

test('all attachment bytes are acknowledged before immutable intent commit; no bytes enter JSON', async () => {
  const t = setup(), images = [image('first bytes'), image('second bytes')];
  const expected = images.map(item => ({ ...item }));
  const accepting = acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/original', 'captured draft', images);
  images[0].base64 = image('later edit').base64; images.pop();
  const record = await accepting;
  assert.deepEqual(t.events, ['read-back', 'write', 'write', 'commit']);
  assert.equal(record.phase, 'preparing');
  assert.equal(record.mayHaveDispatched, false);
  assert.equal(Object.isFrozen(record.intent), true);
  assert.equal(Object.isFrozen(record.intent.attachments), true);
  for (let index = 0; index < expected.length; index++) {
    const hash = 'sha256:' + createHash('sha256').update(Buffer.from(expected[index].base64, 'base64')).digest('hex');
    assert.deepEqual(record.intent.attachments[index], { contentHash: hash, reference: 'spool/' + hash.slice(7), mediaType: 'image/png', name: 'private.png' });
    assert.equal(JSON.stringify(record).includes(expected[index].base64), false);
    assert.deepEqual(await readOutboxAttachment(t.bridge, directory, sha256, record.intent.attachments[index].reference), new Uint8Array(Buffer.from(expected[index].base64, 'base64')));
  }
});

test('same content reuses a content-hash file; no delete API or automatic orphan cleanup', async () => {
  const t = setup();
  await acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/first', 'first', [image()]);
  await acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/second', 'second', [image()]);
  assert.equal(t.files.size, 1);
  t.repository.commit = async () => { throw new Error('disk full'); };
  await assert.rejects(acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/failed', 'keep draft', [image('orphan retained')]), unavailable);
  assert.equal(t.files.size, 2);
  assert.equal(await t.repository.read('attempt/failed'), undefined);
});

test('partial write, wrong descriptor, commit refusal and storage failure retain the caller draft', async () => {
  for (const failure of ['write', 'descriptor', 'commit', 'throw']) {
    const t = setup(), draft = { text: 'draft kept', images: [image()] }, before = structuredClone(draft);
    if (failure === 'write') t.bridge.writeAttachment = async () => { throw new Error('write failure'); };
    if (failure === 'descriptor') t.bridge.writeAttachment = async () => ({ reference: 'spool/wrong', contentHash: 'sha256:wrong', bytes: 0 });
    if (failure === 'commit') t.repository.commit = async () => false;
    if (failure === 'throw') t.repository.commit = async () => { throw new Error('transaction failure'); };
    await assert.rejects(acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/original', draft.text, draft.images), unavailable);
    assert.deepEqual(draft, before);
    assert.equal(await t.repository.read('attempt/original'), undefined);
  }
});

test('text-only acceptance still requires protected storage and commits no attachment reference', async () => {
  const t = setup();
  const record = await acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/text', 'text');
  assert.deepEqual(record.intent, { text: 'text', attachments: [] });
  assert.deepEqual(t.events, ['read-back', 'commit']);
});

test('read rejects traversal, wrong content and revoked exclusion', async () => {
  const t = setup();
  const record = await acceptProtectedOutboxIntent(t.repository, t.bridge, directory, sha256, 'attempt/original', 'text', [image()]);
  await assert.rejects(readOutboxAttachment(t.bridge, directory, sha256, '../private'), unavailable);
  const reference = record.intent.attachments[0].reference;
  t.files.set(reference, Buffer.from('tampered').toString('base64'));
  await assert.rejects(readOutboxAttachment(t.bridge, directory, sha256, reference), unavailable);
  t.protection.spoolExcludedFromBackup = false;
  await assert.rejects(requireProtectedOutboxStorage(t.bridge, directory), unavailable);
});

test('authored native module registers backup read-back and immutable spool without generated edits', () => {
  const root = new URL('./modules/fractal-outbox-files/', import.meta.url);
  const config = JSON.parse(readFileSync(new URL('expo-module.config.json', root)));
  assert.deepEqual(config.apple.modules, ['FractalOutboxFilesModule']);
  const swift = readFileSync(new URL('ios/FractalOutboxFilesModule.swift', root), 'utf8');
  assert.match(swift, /values.isExcludedFromBackup = true/);
  assert.match(swift, /readback.resourceValues\(forKeys: \[.isExcludedFromBackupKey\]\).isExcludedFromBackup == true/);
  assert.match(swift, /Darwin.link\(source, target\)/);
  assert.match(swift, /handle.synchronize\(\)/);
  assert.doesNotMatch(swift, /AsyncFunction\("delete/);
  // Static source assertions are NOT native compilation, flag readback or crash proof.
});
