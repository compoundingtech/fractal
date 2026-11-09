import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const React = { createElement: (type, props, ...children) => ({ type, props, children: children.flat() }) };
const find = (node, type) => !node || typeof node !== 'object' ? undefined : node.type === type ? node
  : node.children?.map(child => find(child, type)).find(Boolean);

function compile(name, next, dependencies) {
  const source = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  const a = source.indexOf(`function ${name}(`);
  const b = source.indexOf(`\n${next}`, a);
  assert.ok(a >= 0 && b > a);
  const { outputText } = ts.transpileModule(source.slice(a, b) + `\nexports.component = ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  });
  const exports = {};
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exports, React, ...Object.values(dependencies));
  return exports.component;
}

function hooks() {
  const values = [];
  let index = 0;
  return {
    reset: () => { index = 0; },
    useState(initial) {
      const at = index++;
      if (!(at in values)) values[at] = typeof initial === 'function' ? initial() : initial;
      return [values[at], next => { values[at] = typeof next === 'function' ? next(values[at]) : next; }];
    },
  };
}

test('a closed thread keeps its composer and exact-message quote without implicitly reopening', async () => {
  const thread = Object.freeze({ id: 'thread/closed', agent_id: 'agent/reviewer', title: 'Review', state: 'closed', unread: false });
  const message = { id: 'message/old', kind: 'message', actor_id: 'agent/reviewer', accepted_at_unix_ms: 1, content: 'Original reply' };
  const calls = [];
  const draftCache = { current: new Map([['draft', JSON.stringify({ text: 'Follow up', quote: { id: message.id, from: 'Reviewer', text: message.content } })]]) };
  const store = {
    actor: 'person/avery', data: { agents: [] }, threadActions: new Set(['thread.reply', 'thread.reopen']),
    status: 'online', draftCache, draftsReady: true,
    actions: {
      replyToThread: async (...args) => { calls.push(['reply', ...args]); return { how: 'sent' }; },
      editThread: async () => { calls.push(['edit']); return { how: 'sent' }; },
    },
  };
  const state = hooks();
  const body = compile('ThreadBody', 'type Quote', {
    ...state, useStore: () => store, useCallback: fn => fn, useRef: current => ({ current }),
    useThreadPage: () => ({ items: [message] }), useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], agentName: () => 'Reviewer', ago: () => '1m',
    View: 'View', T: 'T', Markdown: 'Markdown', Pressable: 'Pressable', ContextMenu: 'ContextMenu', Button: 'Button',
    ThreadComposer: 'ThreadComposer', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  let rendered = body({ client: {}, thread });
  assert.equal(find(rendered, 'ThreadComposer').props.canReply, true);
  const reply = find(rendered, 'Pressable').props.onLongPress;
  assert.equal(typeof reply, 'function');
  reply();
  state.reset();
  rendered = body({ client: {}, thread });
  const props = find(rendered, 'ThreadComposer').props;
  assert.equal(props.selection.id, message.id);

  const composer = compile('ThreadComposer', 'function ThreadComposeField', {
    useStore: () => store, useRef: current => ({ current }), useState: initial => [typeof initial === 'function' ? initial() : initial, () => {}],
    useEffect() {}, useFocusEffect() {}, useCallback: fn => fn, threadDraftKey: () => 'draft',
    readThreadDraft: raw => JSON.parse(raw), threadTextIssue: () => undefined,
    View: 'View', T: 'T', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const field = find(composer(props), 'ThreadComposeField');
  assert.equal(field.props.enabled, true);
  assert.equal(field.props.canSend, true);
  field.props.onSend();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls, [['reply', thread, 'Follow up', message.id]]);
  assert.equal(thread.state, 'closed');
});
