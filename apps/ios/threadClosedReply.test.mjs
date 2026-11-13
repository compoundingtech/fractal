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
    threadEditScope: id => { assert.ok(id); return `edit/${id}`; },
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
  store.threadDeliveries = new Map();
  state.reset();
  const start = { cacheKey: 'origin', accepted: false, title: 'Unsent' };
  assert.equal(find(body({ client: {}, start }), 'ThreadComposer').props.start, start,
    'an unsent draft does not look up metadata delivery for a nonexistent server thread');

});

test('the first draft uses the retained composer through its receipt and accepted summary', async () => {
  const calls = [];
  const id = 'thread/new';
  const state = hooks();
  const draftCache = { current: new Map() };
  let accepted = false;
  const start = {
    cacheKey: 'draft-origin', accepted: false, origin: { agentId: 'agent/reviewer', sourceId: 'notice/source', episode: 'episode/exact' },
    title: 'Review', onAccepted: target => { assert.equal(target, id); accepted = true; }, onCancel() {},
  };
  const store = {
    status: 'online', draftCache, draftsReady: true, saveDrafts() {},
    actions: {
      startThread: async (...args) => { calls.push(['start', ...args]); return { how: 'sent' }; },
      replyToThread: async (...args) => { calls.push(['reply', ...args]); return { how: 'sent' }; },
    },
  };
  const composer = compile('ThreadComposer', 'function ThreadComposeField', {
    ...state, useStore: () => store, useRef: current => ({ current }),
    useEffect() {}, useFocusEffect() {}, useCallback: fn => fn, threadDraftKey: key => key,
    threadStartScope: origin => JSON.stringify(origin), readThreadDraft: raw => raw ? JSON.parse(raw) : { text: '' },
    rememberBounded: (cache, key, value) => cache.set(key, value), threadTextIssue: () => undefined,
    Crypto: { randomUUID: () => 'new' }, View: 'View', T: 'T', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  let props = { start, canReply: true, selectionVersion: 0 };
  let field = find(composer(props), 'ThreadComposeField');
  field.props.onChange('First message');
  state.reset();
  field = find(composer(props), 'ThreadComposeField');
  assert.equal(field.props.text, 'First message');
  assert.equal(JSON.parse(draftCache.current.get(start.cacheKey)).text, 'First message');
  field.props.onSend();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(accepted, true);
  assert.deepEqual(calls, [['start', id, start.origin, start.title, 'First message']]);
  state.reset();
  field = find(composer({ ...props, start: { ...start, target: id, accepted: true } }), 'ThreadComposeField');
  assert.equal(field.props.text, '');
  assert.equal(field.props.enabled, true, 'same editable field remains while confirmed history is loading');
  assert.equal(field.props.canSend, false, 'another send waits for the accepted summary');
  assert.equal(field.props.onCancel, undefined);

  const thread = { id, agent_id: 'agent/reviewer', state: 'open', title: 'Review' };
  props = { thread, canReply: true, selectionVersion: 0 };
  state.reset();
  field = find(composer(props), 'ThreadComposeField');
  assert.equal(field.props.enabled, true);
  field.props.onChange('Follow-up');
  state.reset();
  field = find(composer(props), 'ThreadComposeField');
  field.props.onSend();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls[1], ['reply', thread, 'Follow-up', undefined]);
});

test('an unsent inline thread does not request server history or subscribe to a fabricated summary', async () => {
  let read, follow;
  const calls = [];
  const inline = compile('InlineThread', 'function threadTextIssue', {
    useStore: () => ({ feed: { belongsTo: () => true, followThreadSummary: (...args) => calls.push(args) } }),
    useCallback: fn => fn, useEffect: effect => { effect(); },
    useThreadPage: (reader, _refresh, follower) => { read = reader; follow = follower; return { items: [], loading: false }; },
    ThreadDetailContent: 'ThreadDetailContent',
  });
  const start = { cacheKey: 'draft-origin', accepted: false, title: 'Draft' };
  const rendered = inline({ client: { threadGet: (...args) => calls.push(args) }, id: undefined, refresh: 0, start });
  assert.equal(rendered.props.start, start);
  assert.deepEqual(await read(undefined, new AbortController().signal), { value: { items: [], next_cursor: null } });
  assert.equal(follow({}), undefined);
  assert.deepEqual(calls, []);
});

test('an origin-scoped first draft survives navigating away and starts without a preallocated thread ID', () => {
  const origin = { sourceId: 'notice/canonical', episode: 'episode/original' };
  const cacheKey = 'thread-draft/["notice/canonical","episode/original"]';
  const draftCache = { current: new Map([[cacheKey, JSON.stringify({ text: 'Retained first draft' })]]) };
  const client = {};
  const component = compile('InboxThreadStart', 'function InlineThread', {
    useStore: () => ({ status: 'online', threadRefresh: 0, draftCache, draftsReady: true, saveDrafts() {} }),
    useState: initial => [initial, () => {}], threadStartScope: value => JSON.stringify([value.sourceId, value.episode]),
    threadDraftKey: value => `thread-draft/${value}`, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  for (let navigation = 0; navigation < 2; navigation++) {
    const tree = component({ client, origin, agentId: 'agent/canonical', initialTitle: 'Notice', canStart: true, onOpen() {} });
    const inline = find(tree, 'InlineThread');
    assert.equal(inline.props.id, undefined);
    assert.equal(inline.props.start.target, undefined);
    assert.equal(inline.props.start.cacheKey, cacheKey);
    assert.deepEqual(inline.props.start.origin, { ...origin, agentId: 'agent/canonical' });
    assert.deepEqual(find(tree, 'ThreadGroup').props.context, { origin_source: origin.sourceId, origin_episode: origin.episode });
  }
});

test('collapsing the first draft preserves its origin composer for reopening and acceptance', () => {
  const state = hooks();
  const panel = compile('ThreadDetailContent', 'function threadSummaryLine', {
    ...state, EmbeddedThreadHeader: 'EmbeddedThreadHeader', ThreadBody: 'ThreadBody',
    ThreadActionFeedback: 'ThreadActionFeedback', T: 'T', Button: 'Button',
    threadSummaryLine: thread => thread.state,
  });
  const start = { cacheKey: 'origin-draft', title: 'Notice', accepted: false };
  const props = { client: {}, embedded: true, start };
  let tree = panel(props);
  assert.equal(find(tree, 'ThreadBody').props.start, start);
  find(tree, 'EmbeddedThreadHeader').props.toggle();
  state.reset();
  tree = panel(props);
  assert.equal(find(tree, 'ThreadBody'), undefined);
  find(tree, 'EmbeddedThreadHeader').props.toggle();
  state.reset();
  tree = panel(props);
  assert.equal(find(tree, 'ThreadBody').props.start.cacheKey, 'origin-draft');
  const thread = { id: 'thread/accepted', title: 'Notice', state: 'open' };
  state.reset();
  tree = panel({ ...props, thread, start: { ...start, accepted: true, target: thread.id } });
  assert.equal(find(tree, 'ThreadBody').props.thread, thread);
  assert.equal(find(tree, 'ThreadBody').props.start.cacheKey, start.cacheKey);
});

test('a lost Start receipt remains retryable after the source episode and agent disappear on remount', async () => {
  const origin = { sourceId: 'notice/stable-source', episode: 'episode/original', agentId: 'agent/original' };
  const client = {};
  const byClient = { current: new WeakMap() };
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const a = source.indexOf('  async function trackThreadDelivery(');
  const b = source.indexOf('  const actions =', a);
  const { outputText } = ts.transpileModule(source.slice(a, b) + ';exports.track = trackThreadDelivery;', {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const exports = {};
  new Function('exports', 'threadClient', 'deliveriesByClient', 'setThreadDeliveryRevision', 'errorText', outputText)(
    exports, client, byClient, () => {}, String);
  let exactRetries = 0;
  const original = Object.freeze({ origin: { source_id: origin.sourceId, episode: origin.episode }, agent_id: origin.agentId, content: 'Original content', title: 'Original title', signature: 'original-signature', idempotency_key: 'original-key' });
  const observed = [];
  const retry = async () => { exactRetries++; observed.push(original); return { how: 'sent' }; };
  const scope = value => JSON.stringify([value.sourceId, value.episode]);
  await exports.track(scope(origin), 'thread/original', async () => ({ how: 'unconfirmed', retry }), undefined,
    { origin: { ...origin }, title: original.title });
  const ledger = byClient.current.get(client);
  const cache = { current: new Map() };
  const store = {
    threadClient: client, threadActions: new Set(['thread.start']), threadDeliveries: ledger,
    status: 'online', threadRefresh: 0, draftCache: cache, draftsReady: true, saveDrafts() {},
    actions: { startThread: () => { throw new Error('must not construct a current Start'); } },
  };
  const entry = compile('InboxThread', 'type ThreadStartDraft', {
    useStore: () => store, threadStartScope: scope, InboxThreadStart: 'InboxThreadStart', T: 'T',
  });
  // A fresh component instance sees the same valid source, but the new row carries no
  // usable episode or assigned conversation. Its changed title cannot rewrite the retry.
  const props = entry({ sourceId: origin.sourceId, initialTitle: 'Changed title', onOpen() {} }).props;
  assert.deepEqual(props.origin, origin);
  assert.equal(props.agentId, origin.agentId);
  assert.equal(props.initialTitle, original.title);
  assert.equal(props.canStart, false);
  const startPanel = compile('InboxThreadStart', 'function InlineThread', {
    useStore: () => store, useState: initial => [initial, () => {}], threadStartScope: scope,
    threadDraftKey: value => value, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  const start = find(startPanel(props), 'InlineThread').props.start;
  assert.equal(start.target, 'thread/original');
  const composer = compile('ThreadComposer', 'function ThreadComposeField', {
    useStore: () => store, useRef: current => ({ current }), useState: initial => [typeof initial === 'function' ? initial() : initial, () => {}],
    useEffect() {}, useFocusEffect() {}, useCallback: fn => fn, threadDraftKey: value => value, threadStartScope: scope,
    readThreadDraft: () => ({ text: 'Original content' }), threadTextIssue: () => undefined,
    View: 'View', T: 'T', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const tree = composer({ start, canReply: true, selectionVersion: 0 });
  assert.equal(find(tree, 'ThreadComposeField').props.enabled, false);
  find(tree, 'ThreadActionFeedback').props.onRetry();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(exactRetries, 1);
  assert.deepEqual(observed, [original]);
  assert.equal(ledger.size, 0);
});

test('Inbox recovery exposes orphaned first starts from the same ledger without duplicating listed origins', () => {
  const listed = { sourceId: 'notice/listed', episode: 'episode/one', agentId: 'agent/one' };
  const orphan = { sourceId: 'notice/orphan', episode: 'episode/first', agentId: 'agent/original' };
  const nextEpisode = { ...orphan, episode: 'episode/second' };
  const ledger = new Map([
    ['listed', { target: 'thread/listed', start: { origin: listed, title: 'Listed' } }],
    ['orphan-first', { target: 'thread/first', start: { origin: orphan, title: 'Original title' } }],
    ['orphan-second', { target: 'thread/second', start: { origin: nextEpisode, title: 'Second title' } }],
    ['metadata', { target: 'thread/metadata' }],
  ]);
  const component = compile('PendingInboxThreads', 'type ThreadStartDraft', {
    useStore: () => ({ threadClient: {}, threadDeliveries: ledger, data: { attention: [{ source_id: listed.sourceId }] } }),
    threadStartScope: origin => JSON.stringify([origin.sourceId, origin.episode]),
    View: 'View', T: 'T', InboxThread: 'InboxThread',
  });
  const tree = component({ onOpen() {} });
  const collect = node => !node || typeof node !== 'object' ? [] : node.type === 'InboxThread' ? [node]
    : node.children?.flatMap(collect) ?? [];
  const recovered = collect(tree);
  assert.equal(recovered.length, 2);
  assert.deepEqual(recovered.map(node => node.props.sourceId), [orphan.sourceId, orphan.sourceId]);
  assert.deepEqual(recovered.map(node => node.props.pendingScope), [
    JSON.stringify([orphan.sourceId, orphan.episode]), JSON.stringify([nextEpisode.sourceId, nextEpisode.episode]),
  ]);
  assert.ok(recovered.every(node => node.props.origin === undefined), 'retained context never authorizes a new Start');
});
