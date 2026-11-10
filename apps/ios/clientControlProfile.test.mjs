import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const file = ts.createSourceFile('store.tsx', readFileSync(new URL('./store.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const find = (node, predicate) => predicate(node) ? node : ts.forEachChild(node, child => find(child, predicate));
const pairing = find(file, node => ts.isFunctionDeclaration(node) && node.name?.text === 'completePairing');
const declarations = ['testControl', 'signs', 'next'].map(name => find(pairing, node => ts.isVariableDeclaration(node) && node.name.getText(file) === name).getText(file));
const build = new Function('Constants', 'result', 'made', 'pin', 'member', 'pairDraft', 'old', 'gateway', 'routing', ts.transpileModule(declarations.map(value => `const ${value};`).join('\n') + '\nreturn { next, signs };', { compilerOptions: { target: ts.ScriptTarget.ESNext } }).outputText);
for (const flag of [undefined, false, true]) for (const messageScope of [false, true]) test(`pairing key retention: testMode=${flag} messages=${messageScope}`, () => {
  const session = { device_id: 'device/fixture', session_actor: 'client-session/fixture', person_id: 'person/fixture', credential: 'fixture-bearer', scopes: messageScope ? ['control.messages'] : ['read.projections'], device_key_chain: [] };
  const made = { key: 'p256:fixture', handle: 'fixture-handle', storage: 'secure-enclave' };
  const { next, signs } = build({ expoConfig: { extra: { testClientMode: flag } } }, { value: session }, made, 'pin', undefined, null, undefined, 'https://fixture.invalid', {});
  assert.equal(signs, messageScope || flag === true);
  assert.equal(!!next.signing, messageScope || flag === true);
  assert.deepEqual(next.controlIdentity, flag === true ? { device_id: session.device_id, session_actor: session.session_actor } : undefined);
  assert.equal(next.credential, session.credential);
});
