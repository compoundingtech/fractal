import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { isRequest } from '@fractal/views';
import { autoResolvedRows, closedOverride, needsWords, OVERRIDE_TITLE, overrideAnswer, overrideCopy } from './autoResolved.ts';
import { agentName } from './agentsView.ts';
import { markdownSource, optionalSource } from '../../clients/typescript/fractal-client/markdown-text.ts';

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
  const useStore = () => ({
    data: { attention: [item], agents: [], missions: [] }, actions: {},
    autoResolved: { items: [], issue: '', complete: true }, overridePending: new Set(),
    status: 'online', busy: false,
  });
  const nativeCalls = [];
  const unexpectedNativeCall = name => (...args) => {
    nativeCalls.push({ name, args });
    assert.fail(`Unexpected native call: ${name}`);
  };
  const hookDeps = {
    useStore, useMemo: f => f(), autoResolvedRows, agentName, needsWords, OVERRIDE_TITLE, overrideAnswer, overrideCopy,
    Alert: { alert: unexpectedNativeCall('Alert.alert'), prompt: unexpectedNativeCall('Alert.prompt') },
    showOwnedActionSheet: unexpectedNativeCall('showOwnedActionSheet'),
  };
  const InboxThread = component('./screens/Threads.tsx', 'InboxThread', 'InboxThreadStart', {
    useStore: () => ({ threadClient: enabled ? {} : null, threadActions: new Set(start ? ['thread.start'] : []) }),
    InboxThreadStart: 'InboxThreadStart', T: 'T', threadStartScope: origin => JSON.stringify([origin.sourceId, origin.episode]),
  });
  const dependencies = {
    useStore,
    useAutoResolvedRows: component('./screens/AutoResolved.tsx', 'useAutoResolvedRows', undefined, hookDeps),
    useOverride: component('./screens/AutoResolved.tsx', 'useOverride', undefined, hookDeps),
    closedOverride, OVERRIDE_TITLE, markdownSource, optionalSource,
    useRefresh: () => 'RefreshControl', useMissionsOnFocus() {}, useFocusEffect() {}, useHeaderHeight: () => 0, useCallback: callback => callback, useRef: current => ({ current }), detailRow: () => ({}),
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
    assert.equal('agentId' in entry.props, false, 'the Inbox agent never selects the Start recipient');
    assert.equal(gated.props.pending, undefined, 'a listed row is a fresh Start, never a handed-off attempt');
    assert.equal(gated.type, 'InboxThreadStart');
    assert.equal(gated.props.canStart, true);
    assert.ok(find(screen, 'RequestAnswers'));
    assert.equal(find(screen, 'KeyboardAvoidingView').props.behavior, 'padding');
  }
});

test('all existing Inbox item kinds Start on their canonical source; the StartContext selects the recipient', () => {
  for (const attention_kind of ['person-step', 'agent-request', 'notice', 'message', 'harness-prompt', 'harness-login']) {
    const { entry, gated } = render({ ...ask, attention_kind, actions: [], source_id: 'notice/source' });
    assert.deepEqual(entry.props.origin, { sourceId: 'notice/source', episode: ask.episode });
    assert.equal('agentId' in entry.props, false);
    assert.equal(gated.props.canStart, true);
  }
  const { entry } = render({ ...ask, attention_kind: 'notice', closedElsewhere: true });
  assert.ok(entry, 'cached resolved Inbox items keep the same thread entry');
});

test('an Inbox update retains the same thread component beside its existing controls', () => {
  const { entry } = render({ ...ask, attention_kind: 'notice', update: { about: 'agent/worker', summary: 'Ready' } });
  assert.equal('agentId' in entry.props, false);
  assert.deepEqual(entry.props.origin, { sourceId: ask.source_id, episode: ask.episode });
});

test('a typed notice without an assigned agent still Starts; only the origin and thread.start grant gate it', () => {
  const item = { ...ask, attention_kind: 'notice', conversation_id: undefined };
  const { entry, gated } = render(item);
  assert.equal('agentId' in entry.props, false);
  assert.equal(gated.props.canStart, true, 'no current agent or roster entry is required');
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
