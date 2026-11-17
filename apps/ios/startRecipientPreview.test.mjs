// The Start recipient preview, rendered from the generated ThreadStartRecipient exactly as the
// StartContext returns it. Fixtures follow the generated DTO; no field is synthesized.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { agentName } from './agentsView.ts';

const React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat() }) };
const source = readFileSync(new URL('./screens/StartRecipientPreview.tsx', import.meta.url), 'utf8');
const from = source.indexOf('export function StartRecipientPreview(');
assert.ok(from >= 0);
const { outputText } = ts.transpileModule(`${source.slice(from)}\nexports.value = StartRecipientPreview;`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
});
const exports = {};
let agents = [];
new Function('exports', 'React', 'View', 'T', 'theme', 'agentName', 'useStore', outputText)(
  exports, React, 'View', 'T', { waiting: 'waiting' }, agentName, () => ({ data: { agents } }));
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
    'To agent/reviewer',
  ]);
});

test('a configured system/typed-notice recipient needs no roster entry or label', () => {
  assert.deepEqual(lines({ agent_id: 'agent/unlisted-system-recipient', basis: { _tag: 'configured' } }), [
    'To agent/unlisted-system-recipient',
  ]);
});

test('a retired-origin fallback names the original source agent separately and leaves history unchanged', () => {
  assert.deepEqual(lines(fallback('agent/chief-of-staff', 'agent/reviewer')), [
    'New discussion with agent/chief-of-staff',
    'Original source agent · agent/reviewer',
    'This agent is retired.',
    "The original request's history and participants are unchanged.",
  ]);
});

test('successive stale-recipient refreshes show the exact new ID and explicit fresh-send warning without retargeting a send', () => {
  for (const agentId of ['agent/A', 'agent/B', 'agent/C']) {
    const recipient = fallback(agentId, 'agent/original-source');
    const tree = StartRecipientPreview({ recipient, changed: true });
    const rendered = lines(recipient, true);
    assert.ok(rendered.includes(`New discussion with ${agentId}`));
    assert.ok(rendered.includes('Original source agent · agent/original-source'));
    assert.equal(rendered[0], 'Recipient changed. Review before sending again.');
    assert.equal(tree.props.accessibilityLiveRegion, 'polite', 'the refreshed recipient is announced');
    assert.equal(findAll(tree, node => typeof node.props.onPress === 'function').length, 0,
      'the presenter never submits, retries, or reads');
  }
  assert.equal(StartRecipientPreview({ recipient: fallback('agent/A', 'agent/o') }).props.accessibilityLiveRegion, undefined);
});

test('existing exact-ID metadata supplies agentName for display only, while accessibility retains canonical identity', () => {
  const recipient = fallback('agent/harbor.image-sorter', 'agent/original-source');
  agents = [
    { id: 'agent/harbor.image-sorter', name: 'harbor.image-sorter', host_id: 'host/harbor' },
    { id: 'agent/another', name: 'wrong-recipient' },
  ];
  try {
    const tree = StartRecipientPreview({ recipient });
    const texts = findAll(tree, node => node.type === 'T');
    assert.equal(text(texts[0]), 'New discussion with Image Sorter');
    assert.equal(tree.props.accessibilityLabel, "New discussion with Image Sorter (agent/harbor.image-sorter) Original source agent · agent/original-source. This agent is retired. The original request's history and participants are unchanged.");
    assert.equal(recipient.agent_id, 'agent/harbor.image-sorter', 'display metadata cannot rewrite routing');
    assert.deepEqual(lines(recipient), [
      'New discussion with Image Sorter', 'Original source agent · agent/original-source',
      'This agent is retired.', "The original request's history and participants are unchanged.",
    ]);
    assert.deepEqual(lines({ agent_id: 'agent/unlisted', basis: { _tag: 'configured' } }), ['To agent/unlisted'],
      'a missing exact match stays the truthful ID, not another roster entry');
  } finally {
    agents = [];
  }
});
