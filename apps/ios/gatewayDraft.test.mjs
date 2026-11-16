import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
import { normalizeGatewayUrl } from './gatewayUrl.ts';

const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
const start = source.indexOf('    async saveUrl(');
const callback = stripTypeScriptTypes('({' + source.slice(start, source.indexOf('    async approveRenewal(', start)) + '})');
function setup({ proof = false, url = 'https://old.example', saved = null } = {}) {
  const writes = [], errors = [], selected = [];
  let clears = 0;
  const save = new Function('proof', 'setError', 'normalizeGatewayUrl', 'url', 'clearCachedProjection', 'SecureStore', 'PROFILE_KEY', 'AsyncStorage', 'URL_KEY', 'setUrl', 'urlDraft', 'return ' + callback + '.saveUrl;')(
    proof, error => errors.push(error), normalizeGatewayUrl, url, async () => { clears++; },
    { getItemAsync: async () => saved, setItemAsync: async (...args) => writes.push(['secure', ...args]), WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'device-only' }, 'profile',
    { setItem: async (...args) => writes.push(['preferences', ...args]) }, 'url', value => selected.push(value), 'https://stale-global-draft.example',
  );
  return { save, writes, errors, selected, clears: () => clears };
}

test('saving uses the explicit current local draft and preserves the enrolled profile', async () => {
  const profile = { url: 'https://old.example', credential: 'fixture', personRootFingerprint: 'fixture-pin' };
  const h = setup({ saved: JSON.stringify(profile) });
  await h.save(' http://192.168.1.123:8793/ ');
  assert.equal(h.clears(), 1);
  assert.deepEqual(h.selected, ['http://192.168.1.123:8793']);
  assert.deepEqual(JSON.parse(h.writes[0][2]), { ...profile, url: 'http://192.168.1.123:8793' });
  assert.deepEqual(h.writes[1], ['preferences', 'url', 'http://192.168.1.123:8793']);
  assert.deepEqual(h.errors, ['']);
});

test('invalid local input and a proof session do not change persisted gateway state', async () => {
  const invalid = setup(); await invalid.save('htp://192.168.1.123:8793');
  assert.equal(invalid.writes.length, 0); assert.equal(invalid.clears(), 0); assert.equal(invalid.selected.length, 0);
  assert.match(invalid.errors[0], /Enter the paired gateway/);
  const proof = setup({ proof: true }); await proof.save('http://192.168.1.123:8793');
  assert.equal(proof.writes.length, 0); assert.equal(proof.clears(), 0);
  assert.match(proof.errors[0], /Close the fabric proof/);
});

// Execute the actual hook with controlled commit boundaries; these ports do not initialize RN.
function editor(initial, retain) {
  const cells = []; let cursor = 0, layout = [], effects = [];
  const changed = (before, after) => !before || after.some((value, i) => !Object.is(value, before[i]));
  const useState = initial => {
    const index = cursor++; cells[index] ??= { value: initial };
    return [cells[index].value, value => { cells[index].value = value; }];
  };
  const useRef = initial => { const index = cursor++; cells[index] ??= { current: initial }; return cells[index]; };
  const effect = queue => (run, deps) => {
    const index = cursor++; const prior = cells[index];
    if (changed(prior?.deps, deps)) queue.push(() => {
      prior?.cleanup?.(); cells[index] = { deps, cleanup: run() };
    });
  };
  const useCallback = (callback, deps) => {
    const index = cursor++; const prior = cells[index];
    if (changed(prior?.deps, deps)) cells[index] = { deps, callback };
    return cells[index].callback;
  };
  const hook = stripTypeScriptTypes(readFileSync(new URL('./gatewayDraft.ts', import.meta.url), 'utf8'))
    .replace(/^import .*?;\n/, '').replace('export function', 'function');
  const useDraft = new Function('useState', 'useRef', 'useEffect', 'useLayoutEffect', 'useCallback', hook + '\nreturn useGatewayDraft;')(
    useState, useRef, effect(effects), effect(layout), useCallback,
  );
  const render = saved => {
    cursor = 0; const result = useDraft(saved, retain);
    for (const commit of layout.splice(0)) commit();
    for (const commit of effects.splice(0)) commit();
    return result;
  };
  return { draft: render(initial), render, unmount() { for (const cell of cells) cell?.cleanup?.(); } };
}

test('a same-turn Save reads the latest edit before the next React render', async () => {
  const h = editor('https://old.example', () => {}), save = setup();
  h.draft.change('https://first.example'); h.draft.change('https://latest.example');
  assert.equal(h.draft.value, 'https://old.example', 'no rerender has committed');
  await save.save(h.draft.read());
  assert.deepEqual(save.selected, ['https://latest.example']);
});

test('unmount retains unfinished local edits without per-keystroke provider writes', () => {
  const retained = [], h = editor('https://old.example', value => retained.push(value));
  h.draft.change('https://unfinished.example'); assert.deepEqual(retained, []);
  h.unmount(); assert.deepEqual(retained, ['https://unfinished.example']);
});

test('an external profile replacement resets the field and never restores the previous dirty edit', () => {
  const retained = [], h = editor('https://old.example', value => retained.push(value));
  h.draft.change('https://unfinished.example');
  const replacing = h.render('https://new-profile.example');
  assert.equal(replacing.read(), 'https://new-profile.example', 'layout commit resets current value');
  assert.equal(h.render('https://new-profile.example').value, 'https://new-profile.example');
  h.unmount(); assert.deepEqual(retained, []);
});
