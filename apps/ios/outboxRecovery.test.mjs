import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat(Infinity).filter(Boolean) }) };
const source = readFileSync(new URL('./outboxRecovery.tsx', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
const all = node => !node || typeof node !== 'object' ? [] : [node, ...(node.children ?? []).flatMap(all)];
const render = (rows, issue = '') => {
  const events = [], exports = {};
  const store = { outboxIssue: issue, retryOutbox: () => events.push('storage-retry'), outbox: {
    attempts: { subscribe: () => () => {}, getSnapshot: () => rows.map(draft => ({ draft })) },
    checkStatus: async id => events.push(['check', id]), discard: async id => events.push(['discard', id]),
    retry: async (id, childId) => events.push(['retry-child', id, childId]),
  } };
  const modules = { react: { useSyncExternalStore: (_subscribe, snapshot) => snapshot() },
    'expo-crypto': { randomUUID: () => 'fresh-child' },
    'react-native': { View: 'View', Alert: { alert: (_title, message, buttons) => { events.push(['warning', message]); buttons.find(button => button.text === 'Discard').onPress(); } } },
    'expo-clipboard': { setStringAsync: async text => events.push(['copy', text]) }, './ui': { Button: 'Button', T: 'T' }, './store': { useStore: () => store } };
  new Function('require', 'exports', 'React', outputText)(name => { assert.ok(name in modules, name); return modules[name]; }, exports, React);
  const tree = exports.OutboxRecovery({ chat: 'agent/dotfiles/manager', onOpen: (...args) => events.push(['open', ...args]) });
  const buttons = all(tree).filter(node => node.type === 'Button');
  return { events, buttons, labels: buttons.map(node => node.props.label), text: JSON.stringify(tree) };
};
const row = { id: 'action/original', message: { kind: 'chat', to: 'agent/dotfiles/manager' }, text: 'recovery', images: 1, member: 'member/original', state: 'unknown', status: 'Delivery unknown', canCheck: true };

test('unknown recovery has status/open/copy/discard only; all actions are non-dispatching', async () => {
  const r = render([row]);
  assert.deepEqual(r.labels, ['Check status', 'Open conversation', 'Copy message', 'Discard recovery']);
  for (const button of r.buttons) await button.props.onPress();
  assert.ok(r.events.some(event => event[0] === 'check'));
  assert.ok(r.events.some(event => event[0] === 'copy'));
  assert.ok(r.events.some(event => event[0] === 'open'));
  assert.ok(r.events.some(event => event[0] === 'discard'));
  assert.match(r.events.find(event => event[0] === 'warning')[1], /does not cancel.*prove it was not applied/);
  assert.doesNotMatch(source, /sendAsNew|\.send\(|\.post\(|\.prepare\(/);
});

test('corrupt recovery exposes status and discard only, never Copy/Open/Check/Retry', () => {
  const r = render([{ id: 'action/damaged', state: 'corrupt', images: 0, member: 'member/original', status: 'storage damaged; delivery unknown', canCheck: false }]);
  assert.deepEqual(r.labels, ['Discard recovery']);
  assert.match(r.text, /storage damaged; delivery unknown/);
});

test('unavailable storage offers storage retry later, not a message retry', () => {
  const r = render([], 'Storage unavailable, retry later.');
  assert.deepEqual(r.labels, ['Retry storage later']);
  r.buttons[0].props.onPress(); assert.deepEqual(r.events, ['storage-retry']);
});

test('only proven not-applied recovery offers Retry as a fresh child attempt', async () => {
  const r = render([{ ...row, state: 'not-applied', status: 'Not applied' }]);
  const retry = r.buttons.find(button => button.props.label === 'Retry as new attempt');
  assert.ok(retry);
  await retry.props.onPress();
  assert.deepEqual(r.events, [['retry-child', row.id, 'action/ios-fresh-child']]);
  for (const state of ['unknown', 'pending', 'delivered', 'legacy', 'corrupt']) {
    assert.equal(render([{ ...row, state }]).labels.includes('Retry as new attempt'), false);
  }
});
