import assert from 'node:assert/strict';
import test from 'node:test';
import { identityEnv } from './fractal-env.mjs';

// arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
const read = env => identityEnv('FRACTAL_AGENT', 'ST_AGENT', env);
test('identity migration selects preferred, legacy, equal, and explicit empty values', () => {
  assert.equal(read({}), undefined);
  assert.equal(read({ ST_AGENT: 'legacy' }), 'legacy');
  assert.equal(read({ FRACTAL_AGENT: 'preferred' }), 'preferred');
  assert.equal(read({ FRACTAL_AGENT: 'same', ST_AGENT: 'same' }), 'same');
  assert.equal(read({ FRACTAL_AGENT: '', ST_AGENT: 'legacy' }), '');
});
test('identity conflict refuses both values and reports names only', () => {
  assert.throws(() => read({ FRACTAL_AGENT: 'private-preferred', ST_AGENT: 'private-legacy' }), {
    message: 'Conflicting identity environment variables: FRACTAL_AGENT and ST_AGENT',
  });
});
test('nonempty preferred identity conflicts with an explicitly empty legacy identity', () => {
  assert.throws(() => read({ FRACTAL_AGENT: 'private-preferred', ST_AGENT: '' }), {
    message: 'Conflicting identity environment variables: FRACTAL_AGENT and ST_AGENT',
  });
});
