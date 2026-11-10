import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = ts.createSourceFile('ClientControlNative.tsx', readFileSync(new URL('./ClientControlNative.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const find = node => ts.isPropertyAssignment(node) && node.name.getText(source) === 'result' ? node.initializer : ts.forEachChild(node, find);
const expression = ts.transpileModule(`const result = ${find(source).getText(source)};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
for (const refused of [false, true]) test(`shipped native result uses Expo digest without WebCrypto; refused=${refused}`, async () => {
  const calls = [], bytes = new Uint8Array([1, 2, 3]);
  const Crypto = { CryptoDigestAlgorithm: { SHA256: 'SHA256' }, getRandomBytes: size => new Uint8Array(size), digest: async (algorithm, input) => { calls.push(['digest', algorithm, input]); return bytes.buffer; } };
  const envelope = { command_id: 'command' };
  const signClientResult = async (_binding, _key, _sign, _time, _nonce, sha256) => { assert.deepEqual(await sha256(bytes), bytes); return refused ? undefined : envelope; };
  const result = new Function('Crypto', 'signClientResult', 'controlProfile', 'signWithDeviceKey', 'base64url', 'client', `${expression}; return result;`)(Crypto, signClientResult, { signing: {} }, () => {}, () => 'nonce', { deviceResult: async value => calls.push(['post', value]) });
  await result({}, () => true);
  assert.deepEqual(calls, refused ? [['digest', 'SHA256', bytes]] : [['digest', 'SHA256', bytes], ['post', envelope]]);
  calls.length = 0; await result({}, () => false); assert.deepEqual(calls, []);
});
