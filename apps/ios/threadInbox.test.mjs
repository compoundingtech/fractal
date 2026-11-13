import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { isRequest } from '@fractal/views';

// Execute the production JSX entry points without booting React Native. The element
// factory records the tree, including which branch mounted the real InboxThread gate.
const React = { createElement: (type, props, ...children) => ({ type, props, children }) };
function component(file, name, next, dependencies) {
  const source = readFileSync(new URL(file, import.meta.url), 'utf8');
  const start = source.indexOf(`export function ${name}(`);
  const end = next ? source.indexOf(`\nfunction ${next}(`, start) : source.length;
  assert.ok(start >= 0 && end > start);
  const { outputText } = ts.transpileModule(source.slice(start, end), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  });
  const exports = {};
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exports, React, ...Object.values(dependencies));
  return exports[name];
}

const find = (element, type) => !element || typeof element !== 'object' ? undefined
  : element.type === type ? element
  : element.children?.map(child => find(child, type)).find(Boolean);

function render(item, enabled = true, start = true) {
  const navigated = [];
  const navigation = { navigate: (...args) => navigated.push(args), goBack() {} };
  const InboxThread = component('./screens/Threads.tsx', 'InboxThread', 'InboxThreadStart', {
    useStore: () => ({ threadClient: enabled ? {} : null, threadActions: new Set(start ? ['thread.start'] : []) }),
    InboxThreadStart: 'InboxThreadStart', T: 'T', threadStartScope: origin => JSON.stringify([origin.sourceId, origin.episode]),
  });
  const dependencies = {
    useStore: () => ({ data: { attention: [item], agents: [], missions: [] }, actions: {} }),
    useMissionsOnFocus() {}, useFocusEffect() {}, useHeaderHeight: () => 0, useCallback: callback => callback, useRef: current => ({ current }), detailRow: () => ({}),
    spaced: text => text, attentionKindLabel: kind => kind, attentionActionLabel: action => action, cleanMessageText: text => text, blockedLine: () => undefined, theme: {},
    Screen: 'Screen', Banners: 'Banners', ScrollView: 'ScrollView', KeyboardAvoidingView: 'KeyboardAvoidingView', T: 'Text',
    AlertBody: 'AlertBody', Button: 'Button', Markdown: 'Markdown', RequestQuestion: 'RequestQuestion', RequestAnswers: 'RequestAnswers', ThreadViewport: 'ThreadViewport', isRequest, InboxThread,
  };
  const AttentionScreen = component('./screens/Now.tsx', 'AttentionScreen', undefined, dependencies);
  const screen = AttentionScreen({ route: { params: { id: item.id } }, navigation });
  const entry = find(screen, InboxThread);
  return { screen, entry, navigated, gated: entry ? InboxThread(entry.props) : undefined };
}

const ask = {
  id: 'attention/card', attention_kind: 'person-step', source_id: 'step-run/review',
  conversation_id: 'agent/authoritative', episode: 'episode/now', requester_id: 'agent/builder', title: 'Review the change',
  detail: 'Approve this?', actions: ['work.done'],
};

test('the active Inbox ask mounts inline start with the exact origin and keeps its answer controls', () => {
  for (const attention_kind of ['person-step', 'agent-request']) {
    const { screen, entry, gated } = render({ ...ask, attention_kind });
    assert.deepEqual(entry.props.origin, { sourceId: 'step-run/review', episode: 'episode/now' });
    assert.equal(entry.props.agentId, 'agent/authoritative');
    assert.equal(gated.type, 'InboxThreadStart');
    assert.equal(gated.props.canStart, true);
    assert.ok(find(screen, 'RequestAnswers'));
    assert.equal(find(screen, 'KeyboardAvoidingView').props.behavior, 'padding');
  }
});

test('all existing Inbox item kinds use their canonical source and server-resolved agent', () => {
  for (const attention_kind of ['person-step', 'agent-request', 'notice', 'message', 'harness-prompt', 'harness-login']) {
    const { entry, gated } = render({ ...ask, attention_kind, actions: [], source_id: 'notice/source' });
    assert.deepEqual(entry.props.origin, { sourceId: 'notice/source', episode: ask.episode });
    assert.equal(entry.props.agentId, 'agent/authoritative');
    assert.equal(gated.props.canStart, true);
  }
  const { entry } = render({ ...ask, attention_kind: 'notice', closedElsewhere: true });
  assert.ok(entry, 'cached resolved Inbox items keep the same thread entry');
});

test('an Inbox update retains the same thread component beside its existing controls', () => {
  const { entry } = render({ ...ask, attention_kind: 'notice', update: { about: 'agent/worker', summary: 'Ready' } });
  assert.equal(entry.props.agentId, 'agent/authoritative');
  assert.deepEqual(entry.props.origin, { sourceId: ask.source_id, episode: ask.episode });
});

test('missing canonical agent disables starts without guessing requester responsibility', () => {
  const item = { ...ask, conversation_id: undefined };
  const { entry, gated } = render(item);
  assert.equal(entry.props.agentId, undefined);
  assert.equal(gated.props.canStart, false);
  assert.equal(render(ask, false).gated, null);
  assert.equal(render(ask, true, false).gated.props.canStart, false);
  assert.ok(render({ ...ask, episode: undefined }).entry);
  assert.equal(render({ ...ask, episode: undefined }).gated.type, 'T');
});


test('agent navigation uses canonical responsibility while request attribution stays with its publisher', () => {
  const { screen, navigated } = render(ask);
  const button = node => !node || typeof node !== 'object' ? undefined : node.props?.label === 'Open agent conversation' ? node
    : node.children?.map(button).find(Boolean);
  button(screen).props.onPress();
  assert.deepEqual(navigated, [['Conversation', { target: 'agent/authoritative', title: undefined }]]);
  assert.equal(find(screen, 'RequestAnswers').props.from, 'builder');
});
