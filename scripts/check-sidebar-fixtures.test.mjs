import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { behaviorSchema, checkEnvelope, checkFixture, checkFixtures, checkGolden, checkSchema, scrub } from './check-sidebar-fixtures.mjs';

const fixtures = JSON.parse(readFileSync(new URL('../fixtures/clients/sidebar/behavior-v1.json', import.meta.url), 'utf8'));
const mutate = change => { const value = structuredClone(fixtures); change(value); return value; };
const folder = '00000000-0000-7000-8000-00000000000a';

test('real sidebar fixtures pass the CLI and the shared schema/public policy', () => {
  assert.equal(checkFixtures(), 2);
  const result = spawnSync(process.execPath, [new URL('./check-sidebar-fixtures.mjs', import.meta.url).pathname], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Sidebar fixture gate passed/);
  assert.doesNotThrow(() => checkFixture(fixtures));
});

test('generated sidebar buffers and stable hit identities are scrubbed', () => {
  const golden = JSON.parse(readFileSync(new URL('../fixtures/clients/sidebar/tui-v1.snapshot', import.meta.url), 'utf8'));
  assert.doesNotThrow(() => checkGolden(golden));
  for (const change of [
    value => { value.cases[0].selected = 'subject:person/private'; },
    value => { value.cases[0].hits[0][2] = 'subject:agent/private'; },
    value => { value.cases[0].contexts[0][1] = 'subject:host/private'; },
    value => { value.cases[0].unknown = true; },
    value => { value.cases[0].cells[0][0] = 0; },
    value => { value.cases[0].cells[0][1][1] = 'person/private'; },
  ]) {
    const value = structuredClone(golden); change(value);
    assert.throws(() => checkGolden(value));
  }
  for (const text of ['/home/private/project', 'node.internal', 'Bearer private-value', 'person/private']) {
    const value = structuredClone(golden);
    value.cases[0].cells = [...text].map(symbol => [1, [symbol, '']]);
    value.cases[0].cells.push([616 - text.length, [' ', '']]);
    assert.throws(() => checkGolden(value), text);
  }
});

const forbidden = {
  'POSIX home': ['', 'home', 'private-user', 'project'].join('/'), 'macOS home': ['', 'Users', 'private-user', 'project'].join('/'),
  'Windows home': 'C:\\Users\\private-user\\project', 'tilde home': '~/private/project',
  'file URL': 'file:///private/project', 'private hostname': 'private-node.local',
  'private domain': 'service.example.internal', 'tailnet domain': 'node.example.ts.net',
  'loopback name': 'localhost', 'private IPv4': '10.23.45.67', 'loopback IPv4': '127.0.0.1',
  'CGNAT IPv4': '100.64.0.1:1234', 'link-local IPv4': '169.254.1.2',
  'private IPv6': 'fd00::1234', 'IPv6 loopback': '[::1]:1234',
  'host URL': 'https://private-node.example.test', 'tracker ARN': 'arn:private:ticket:123',
  'tracker URN': 'urn:private:ticket:123', 'tracker reference': 'PRIVATE-123',
  'session identity': 'session/private-session', 'agent URI': 'agent://private-agent',
  'credential key': 'authorization', 'API key field': 'api_key',
  'bearer': 'Bearer synthetic-test-credential', 'token': 'ghp_' + 'x'.repeat(32),
  'cloud key': 'AKIA' + 'X'.repeat(16), 'chat token': 'xoxb-synthetic-test',
  'JWT': 'eyJtest.eyJtest.synthetic', 'basic credential': 'Basic c3ludGhldGljOnRlc3Q=',
  'private key': '-----BEGIN PRIVATE KEY-----', 'password assignment': 'password=synthetic-test-value',
};
for (const [name, text] of Object.entries(forbidden)) {
  test(`rejects ${name} in decoded values, keys and escaped JSON`, () => {
    assert.throws(() => scrub({ nested: [{ text }] }));
    assert.throws(() => scrub({ nested: { [text]: 'synthetic' } }));
    const escaped = [...text].map(char => `\\u${char.charCodeAt(0).toString(16).padStart(4, '0')}`).join('');
    assert.throws(() => scrub(JSON.parse(`{"nested":[{"text":"${escaped}"}]}`)));
    assert.throws(() => scrub(JSON.parse(`{"${escaped}":"synthetic"}`)));
  });
}

test('schema-aware allowlist refuses identities that do not use synthetic namespaces', () => {
  for (const change of [
    v => { v.cases[0].initial.arrangement.owner = ['person', 'private-person'].join('/'); },
    v => { v.cases[0].initial.arrangement.owner = 'Private Person'; },
    v => { v.cases[0].catalog = ['agent/private-agent']; },
    v => { v.cases[0].catalog = ['machine/private-node']; },
    v => { v.cases[0].initial.arrangement.id = ['arrangement', 'person', 'private-person', folder].join('/'); },
    v => { v.cases[0].initial.arrangement.body.name.value = 'Private Person'; },
    v => { v.cases[0].initial.arrangement.revision = 'private-revision'; },
    v => { v.cases[0].name = 'private-person'; },
    v => { v.cases[0].initial.arrangement.body.placements['agent/private-agent'] = v.cases[1].initial.arrangement.body.placements['agent/test/one']; },
    v => { v.cases[0].initial.arrangement.resolved.folders['machine/private-node'] = null; },
    v => { v.cases[1].checkpoints[0].tree.nodes['subject:agent/private-agent'] = v.cases[1].checkpoints[0].tree.nodes[`folder:${folder}`]; },
  ]) assert.throws(() => checkFixture(mutate(change)));
  assert.doesNotThrow(() => checkFixture(mutate(v => {
    v.cases[0].name = 'synthetic-case-1'; v.cases[0].catalog = ['agent/test/resource-1', 'host/test-1'];
    v.cases[0].initial.arrangement.body.name.value = 'Synthetic Folder 1';
  })));
});

test('all used schema constraints and unknown future keywords fail closed', () => {
  for (const change of [
    v => { v.schemaVersion = 2; }, v => { delete v.schemaVersion; }, v => { v.cases[0].unknown = true; },
    v => { v.cases[0].events.push(v.cases[0].events[0]); }, v => { v.cases[0].checkpoints = []; },
    v => { v.cases[0].events[0].type = 'Unknown'; }, v => { v.cases[0].name = ''; },
    v => { Object.values(v.cases[1].checkpoints[0].rollup)[0].needsYou = -1; },
    v => { v.cases[0].catalog = {}; }, v => { v.cases[0].initial.journal = [{}]; },
  ]) assert.throws(() => checkEnvelope(mutate(change)));
  assert.throws(() => checkSchema({ ...behaviorSchema, pattern: 'unsupported' }));
  assert.throws(() => checkSchema({ ...behaviorSchema, properties: { future: { maxLength: 1 } } }));
});

test('recursive JSON/snapshot discovery, empty sets, parse errors and symlinks are gated', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sidebar-fixture-test-'));
  try {
    assert.throws(() => checkFixtures(directory));
    mkdirSync(join(directory, 'snapshots'));
    const path = join(directory, 'snapshots', 'seed.snapshot.json');
    writeFileSync(path, JSON.stringify(fixtures));
    assert.equal(checkFixtures(directory), 1);
    writeFileSync(path, JSON.stringify(mutate(v => { v.cases[0].catalog = ['host/private-node']; })));
    assert.throws(() => checkFixtures(directory));
    writeFileSync(path, '{\"token\":\"synthetic-invalid-json\"');
    assert.throws(() => checkFixtures(directory), error => error.message === 'invalid sidebar fixture JSON');
    writeFileSync(path, JSON.stringify(fixtures));
    const snapshot = join(directory, 'seed.snap');
    writeFileSync(snapshot, JSON.stringify(fixtures)); assert.equal(checkFixtures(directory), 2);
    writeFileSync(join(directory, 'hidden.txt'), 'synthetic');
    assert.throws(() => checkFixtures(directory)); rmSync(join(directory, 'hidden.txt'));
    symlinkSync(path, join(directory, 'linked.json')); assert.throws(() => checkFixtures(directory));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
