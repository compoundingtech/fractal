// The Start recipient preview, rendered from the generated ThreadStartRecipient exactly as the
// StartContext returns it. Fixtures follow the generated DTO; no field is synthesized.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat() }) };
const source = readFileSync(new URL('./screens/StartRecipientPreview.tsx', import.meta.url), 'utf8');
const from = source.indexOf('export function StartRecipientPreview(');
assert.ok(from >= 0);
const { outputText } = ts.transpileModule(`${source.slice(from)}\nexports.value = StartRecipientPreview;`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
});
const exports = {};
new Function('exports', 'React', 'View', 'T', 'theme', outputText)(exports, React, 'View', 'T', { waiting: 'waiting' });
const StartRecipientPreview = exports.value;
const findAll = (node, match) => !node || typeof node !== 'object' ? [] : [
  ...(match(node) ? [node] : []), ...(node.children ?? []).flatMap(child => findAll(child, match)),
];
const text = node => !node || typeof node !== 'object' ? String(node ?? '') : node.children.map(text).join('');
const lines = (recipient, changed = false) => findAll(StartRecipientPreview({ recipient, changed }), node => node.type === 'T').map(text);

/** @type {(agent_id: string, original_agent_id: string) => import('../../clients/typescript/fractal-client').ThreadStartRecipient} */
const fallback = (agent_id, original_agent_id) => Object.freeze({
  agent_id, basis: { _tag: 'fallback', original_agent_id, retirement: { claim_id: `claim/retired/${original_agent_id}`, kind: 'agent.retired', state: 'retired' } },
});

test('an original recipient is named by its exact agent ID', () => {
  assert.deepEqual(lines({ agent_id: 'agent/reviewer', basis: { _tag: 'original' } }), [
    'To agent/reviewer', 'Recipient · agent/reviewer · basis original', 'The original recipient.',
  ]);
});

test('a configured system/typed-notice recipient needs no roster entry or label', () => {
  assert.deepEqual(lines({ agent_id: 'agent/unlisted-system-recipient', basis: { _tag: 'configured' } }), [
    'To agent/unlisted-system-recipient', 'Recipient · agent/unlisted-system-recipient · basis configured', 'The configured recipient for this source.',
  ]);
});

test('a retired-origin fallback names the original source agent separately and leaves history unchanged', () => {
  assert.deepEqual(lines(fallback('agent/chief-of-staff', 'agent/reviewer')), [
    'New discussion with agent/chief-of-staff',
    'Recipient · agent/chief-of-staff · basis fallback',
    'Original source agent · agent/reviewer',
    'agent/reviewer is retired. This Start opens a new discussion with agent/chief-of-staff before the first message is sent.',
    "The original request's history and participants are unchanged.",
  ]);
});

test('successive stale-recipient refreshes show the exact new ID and explicit fresh-send warning without retargeting a send', () => {
  for (const agentId of ['agent/A', 'agent/B', 'agent/C']) {
    const recipient = fallback(agentId, 'agent/original-source');
    const tree = StartRecipientPreview({ recipient, changed: true });
    const rendered = lines(recipient, true);
    assert.ok(rendered.includes(`Recipient · ${agentId} · basis fallback`));
    assert.ok(rendered.includes('Original source agent · agent/original-source'));
    assert.match(rendered[0], /Recipient refreshed.*fresh send.*previous send is not retargeted/);
    assert.equal(tree.props.accessibilityLiveRegion, 'polite', 'the refreshed recipient is announced');
    assert.equal(findAll(tree, node => typeof node.props.onPress === 'function').length, 0,
      'the presenter never submits, retries, or reads');
  }
  assert.equal(StartRecipientPreview({ recipient: fallback('agent/A', 'agent/o') }).props.accessibilityLiveRegion, undefined);
});
