import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { isRequest } from '@fractal/views';

// Execute the production JSX entry points without booting React Native. The element
// factory records the tree, including which branch mounted the real AskThread gate.
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
  const navigation = { navigate() {}, goBack() {} };
  const AskThread = component('./screens/Threads.tsx', 'AskThread', 'AskThreadStart', {
    useStore: () => ({ threadClient: enabled ? {} : null, threadActions: new Set(start ? ['thread.start'] : []) }),
    AskThreadStart: 'AskThreadStart',
  });
  const dependencies = {
    useStore: () => ({ data: { attention: [item], agents: [], missions: [] }, actions: {} }),
    useMissionsOnFocus() {}, useFocusEffect() {}, useHeaderHeight: () => 0, useCallback: callback => callback, useRef: current => ({ current }), detailRow: () => ({}),
    cleanMessageText: text => text, blockedLine: () => undefined, theme: {},
    Screen: 'Screen', Banners: 'Banners', ScrollView: 'ScrollView', KeyboardAvoidingView: 'KeyboardAvoidingView', T: 'Text',
    Button: 'Button', Markdown: 'Markdown', RequestQuestion: 'RequestQuestion', RequestAnswers: 'RequestAnswers', isRequest, AskThread,
  };
  const AttentionScreen = component('./screens/Now.tsx', 'AttentionScreen', undefined, dependencies);
  const screen = AttentionScreen({ route: { params: { id: item.id } }, navigation });
  const entry = find(screen, AskThread);
  return { screen, entry, gated: entry ? AskThread(entry.props) : undefined };
}

const ask = {
  id: 'attention/card', attention_kind: 'person-step', source_id: 'step-run/review',
  episode: 'episode/now', requester_id: 'agent/builder', title: 'Review the change',
  detail: 'Approve this?', actions: ['work.done'],
};

test('the active Inbox ask mounts inline start with the exact origin and keeps its answer controls', () => {
  for (const attention_kind of ['person-step', 'agent-request']) {
    const { screen, entry, gated } = render({ ...ask, attention_kind });
    assert.deepEqual(entry.props.origin, { agentId: 'agent/builder', ask: 'step-run/review', episode: 'episode/now' });
    assert.equal(gated.type, 'AskThreadStart');
    assert.equal(gated.props.canStart, true);
    assert.ok(find(screen, 'RequestAnswers'));
    assert.equal(find(screen, 'KeyboardAvoidingView').props.behavior, 'padding');
  }
});

test('the Inbox thread entry respects capability, action grant, and exact ask identity', () => {
  assert.equal(render(ask, false).gated, null);
  assert.equal(render(ask, true, false).gated.props.canStart, false);
  for (const patch of [{ source_id: 'agent/builder' }, { requester_id: undefined }, { episode: undefined }]) {
    assert.equal(render({ ...ask, ...patch }).entry, undefined);
  }
  assert.equal(render({ ...ask, closedElsewhere: true }).gated.props.canStart, false);
});
