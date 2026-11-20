import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { ThreadEdits } from './threadEdits.ts';
import { LocalDrafts } from './draftsStorage.ts';
import { savedThreadStarts, startRecoveryKey } from './threadStartRecovery.ts';
import { ClientError, FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { threadClock, threadExactTime, threadRelativeTime } from './threadClock.ts';
import { anchorPresentation, reminderFromSummary } from './unreadVisit.ts';
import { pendingUnreadAPI } from './unreadActions.ts';
import { nativeVisitPorts } from './unreadVisitTestPorts.mjs';
import { unreadClient as bindUnreadClient, unreadSupported } from './unreadClient.ts';
import { ConversationAttempts } from './conversationAttempts.ts';

const React = { createElement: (type, props, ...children) => ({ type, props, children: children.flat() }) };
const find = (node, type) => findBy(node, candidate => candidate.type === type);
const findBy = (node, match) => !node || typeof node !== 'object' ? undefined : match(node) ? node
  : node.children?.map(child => findBy(child, match)).find(Boolean);
const button = (node, label) => findBy(node, candidate => candidate.type === 'Button' && candidate.props.label === label);
// The store's unread service; bodies bind it per thread. Tests needing behavior pass their own.
const unreadClient = { forThread: () => ({ state: 'ready', actions: {} }), listMarked: async () => ({ kind: 'empty' }) };
let lineEvidence, draftReader;
// Mirrors store.tsx: each Start attempt's delivery is keyed by its origin and new thread ID.
const attemptScope = (origin, thread) => JSON.stringify([origin.sourceId, origin.episode, thread]);
class ReadyStartContext {
  constructor(_client, origin) {
    this.value = { kind: 'ready', changed: false, context: {
      kind: 'thread-start-context', source_id: origin.sourceId, episode: origin.episode,
      start_recipient: { agent_id: 'agent/one', basis: { _tag: 'configured' } },
    } };
  }
  subscribe = () => () => {};
  getSnapshot = () => this.value;
  async load() {}
  close() {}
}

function compile(name, next, dependencies, file = './screens/Threads.tsx') {
  const source = readFileSync(new URL(file, import.meta.url), 'utf8');
  const a = source.search(new RegExp(`function ${name}(?=[(<])`));
  const b = next ? source.indexOf(`\n${next}`, a) : source.length;
  assert.ok(a >= 0 && b > a);
  const { outputText } = ts.transpileModule(source.slice(a, b) + `\nexports.component = ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  });
  const exports = {};
  // Components rendered as children stay inspectable without mounting a second hook scope.
  dependencies = { ThreadReadIssue: 'ThreadReadIssue', Text: 'Text', RequestHistory: 'RequestHistory', threadClock, threadExactTime, threadRelativeTime,
    ConversationComposer: props => compile('ConversationComposer', undefined, { View: 'View', Field: 'Field', Button: 'Button' }, './conversationComposer.tsx')(props),
    ConversationFeedback: props => compile('ConversationFeedback', undefined, { View: dependencies.View ?? 'View', T: dependencies.T ?? 'T', Button: dependencies.Button ?? 'Button' }, './conversationPresentation.tsx')(props),
    ConversationMessage: props => compile('ConversationMessage', undefined, { View: dependencies.View ?? 'View', T: dependencies.T ?? 'T', Markdown: dependencies.Markdown ?? 'Markdown' }, './conversationPresentation.tsx')(props),
    HistoryStart: 'HistoryStart', StartRecipientPreview: 'StartRecipientPreview', ThreadStartContextReader: ReadyStartContext,
    Clipboard: { setStringAsync: async () => {} }, useMemo: create => create(), useEffect() {}, useFocusEffect() {}, useCallback: callback => callback,
    threadStartAttemptScope: attemptScope,
    threadStartSlot: thread => `start/${thread}`,
    threadStartScope: origin => JSON.stringify([origin.sourceId, origin.episode]),
    reminderFromSummary, anchorPresentation, pendingUnreadAPI, useUnreadVisit: nativeVisitPorts().useUnreadVisit,
    savedThreadStarts, startRecoveryKey,
    useUnreadVisitEntry: nativeVisitPorts().useUnreadVisitEntry,
    renderedLineEvidence: (...args) => (lineEvidence ??= compile('renderedLineEvidence', 'function threadTextIssue', {}))(...args),
    readThreadDraft: (...args) => (draftReader ??= compile('readThreadDraft', 'function ThreadComposer', {}))(...args),
    useSyncExternalStore: (_subscribe, getSnapshot) => getSnapshot(), ...dependencies };
  if (dependencies.useStore) {
    const useStore = dependencies.useStore;
    const ownership = { draftsOwner: { current: 'person/one' }, draftGeneration: { current: 0 } };
    dependencies.useStore = () => ({ ...ownership, ...useStore() });
  }
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

test('the composer exposes Message thread exactly once when empty and after typing', () => {
  const compose = compile('ThreadComposeField', undefined, { View: 'View', Field: 'Field', Button: 'Button' });
  for (const text of ['', 'A reply']) {
    const tree = compose({ text, onChange() {}, onSend() {}, enabled: true, canSend: false });
    const field = find(tree, 'Field');
    // RN 0.86.3 RCTUITextView appends the placeholder only when its text is empty.
    const accessibleName = [field.props.accessibilityLabel, !field.props.value.length ? field.props.placeholder : undefined].filter(Boolean).join(' ');
    assert.equal(accessibleName, 'Message thread');
    assert.equal(field.props.accessibilityLabel, text.length ? 'Message thread' : undefined);
    assert.equal(tree.props.accessibilityLabel, undefined, 'the wrapper does not repeat the field name');
  }
});

test('a closed thread keeps its composer and exact-message quote without implicitly reopening', async () => {
  const thread = Object.freeze({ id: 'thread/closed', agent_id: 'agent/reviewer', title: 'Review', state: 'closed', unread: false });
  const message = { id: 'message/old', kind: 'message', actor_id: 'agent/reviewer', accepted_at_unix_ms: 1, content: 'Original reply' };
  const calls = [];
  const draftCache = { current: new Map([['draft', JSON.stringify({ text: 'Follow up', quote: { id: message.id, from: 'Reviewer', text: message.content } })]]) };
  const store = {
    actor: 'person/avery', data: { agents: [] }, threadActions: new Set(['thread.reply', 'thread.reopen']), unreadClient,
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

const summaryLine = compile('threadSummaryLine', '/** One thread row', {});
const threadRow = compile('ThreadRow', 'export function InboxThread', {
  useTextAppearance: () => ({ color: color => color }), useTitleLines: () => 1, Pressable: 'Pressable', View: 'View', T: 'T', theme: { accent: 'accent' },
});

test('VoiceOver reads thread rows, the Threads header and thread actions by name, role and state', () => {
  const pressable = compile('Button', '\n/** A text input', {
    useTextAppearance: () => ({ system: false, palette: {} }), Pressable: 'Pressable', T: 'T', styles: {}, theme: { accent: 'accent' },
  }, './ui.tsx');
  const control = pressable({ label: 'Reopen thread', onPress() {}, disabled: true });
  assert.equal(control.props.accessibilityRole, 'button');
  assert.equal(control.props.accessibilityLabel, 'Reopen thread');
  assert.deepEqual(control.props.accessibilityState, { disabled: true });
  assert.equal(control.props.accessibilityHint, undefined, 'existing callers retain their default semantics');
  const contextual = pressable({ label: 'Send', accessibilityLabel: 'Send reply to Review', hint: 'Sends to Review', onPress() {} });
  assert.equal(contextual.props.accessibilityLabel, 'Send reply to Review');
  assert.equal(contextual.props.accessibilityHint, 'Sends to Review');
  assert.deepEqual(contextual.children[0].children, ['Send'], 'spoken context does not change the visible title');

  // A row shows an unread dot, the title and Closed; count, state and unread are spoken only.
  const thread = { id: 'thread/one', title: 'Review', state: 'closed', unread: true, message_count: 2 };
  const header = compile('ContextThreadHeader', 'function ExpandedContextThread', {
    useStore: () => ({}), threadSummaryLine: summaryLine, ThreadRow: 'ThreadRow',
  })({ thread, open: false, toggle() {} });
  assert.deepEqual({ ...header.props, toggle: undefined }, {
    title: 'Review', label: 'Review, 2 messages, closed, unread', unread: true, reminder: false, trailing: 'Closed', expanded: false, toggle: undefined,
  });
  const row = threadRow(header.props);
  assert.equal(row.props.accessibilityRole, 'button');
  assert.equal(row.props.accessibilityLabel, 'Review, 2 messages, closed, unread');
  assert.deepEqual(row.props.accessibilityState, { expanded: false });
  assert.deepEqual(row.props.accessibilityValue, { text: 'Collapsed' });
  assert.equal(row.props.accessibilityHint, 'Shows Review');
  assert.equal(row.props.style.minHeight, 44);
  const [dot, title, reminder, trailing] = row.children;
  assert.equal(reminder, null, 'natural unread alone does not display a reminder');
  assert.deepEqual([dot.props.style.width, dot.props.style.backgroundColor], [8, 'accent']);
  assert.deepEqual(title.children, ['Review'], 'no disclosure glyph, count, state words or age');
  assert.equal(title.props.style.flex, 1);
  assert.deepEqual(trailing.children, ['Closed']);
  const openRow = threadRow({ title: 'Review', label: 'Review, 1 message, open', expanded: true, toggle() {} });
  assert.equal(openRow.children[0].props.style.backgroundColor, 'transparent', 'read rows keep the 8pt spacer');
  assert.equal(openRow.children[3], null, 'open rows show no trailing state');
  assert.deepEqual(openRow.props.accessibilityValue, { text: 'Expanded' });
  assert.equal(openRow.props.accessibilityHint, 'Hides Review');

  const state = hooks();
  const page = { items: [thread], snapshots: new Map(), loading: false, live: false, reload() {}, replace() {} };
  const list = compile('ContextThreadList', 'function ContextThreadHeader', {
    ...state, useStore: () => ({ threadRefresh: 0 }), useMemo: compute => compute(), useCallback: fn => fn, useEffect() {}, useFocusEffect() {},
    useRef: current => ({ current }), useWindowDimensions: () => ({ height: 800 }), useTitleLines: () => 1, useThreadViewports: () => [], THREAD_WINDOW_SIZE: 20,
    useThreadPage: () => page, useTextAppearance: () => ({ system: false, palette: {} }), theme: {},
    View: 'View', Pressable: 'Pressable', T: 'T', ScrollView: 'ScrollView', ThreadViewport: 'ThreadViewport', Button: 'Button',
    ExpandedContextThread: 'ExpandedContextThread', ContextThreadHeader: 'ContextThreadHeader', useMarkedUnreadPage: () => ({ items: [] }),
  });
  const render = () => { state.reset(); return list({ context: { agent_id: 'agent/reviewer' }, client: {}, openThread() {}, title: 'Threads' }); };
  let group = find(render(), 'Pressable');
  assert.equal(group.props.accessibilityRole, 'button');
  assert.equal(group.props.accessibilityLabel, 'Threads, 1');
  assert.deepEqual(group.props.accessibilityState, { expanded: false });
  assert.deepEqual(group.props.accessibilityValue, { text: 'Collapsed' });
  assert.equal(group.props.accessibilityHint, 'Shows the threads');
  assert.equal(group.props.style.minHeight, 44);
  assert.deepEqual(group.children[0].children[1].children, [' · ', '1'], 'count, not the first thread title');
  assert.deepEqual(group.children[1].children, ['Show']);
  group.props.onPress();
  let shown = render();
  group = find(shown, 'Pressable');
  assert.deepEqual(group.props.accessibilityState, { expanded: true });
  assert.deepEqual(group.props.accessibilityValue, { text: 'Expanded' });
  assert.equal(group.props.accessibilityHint, 'Hides the threads');
  assert.deepEqual(group.children[1].children, ['Hide']);
  assert.equal(button(shown, 'Refresh threads').props.disabled, false, 'an HTTP-only list keeps manual refresh');
  page.live = true;
  page.more = () => {};
  shown = render();
  assert.equal(button(shown, 'Refresh threads'), undefined, 'a followed list refreshes itself');
  assert.ok(button(shown, 'More threads'));
  assert.equal(find(shown, 'Pressable').props.accessibilityLabel, 'Threads, 1 or more');
  page.issue = 'Updates paused';
  assert.equal(button(render(), 'Retry').props.accessibilityLabel, 'Retry loading threads');

  const body = compile('ThreadBody', 'type Quote', {
    ...hooks(), useCallback: fn => fn, useRef: current => ({ current }), threadEditScope: id => `edit/${id}`,
    useStore: () => ({ actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {}, unreadClient,
      threadActions: new Set(['thread.reply', 'thread.rename', 'thread.close', 'thread.reopen']) }),
    useThreadPage: () => ({ items: [], loading: true }), useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], View: 'View', T: 'T', Button: 'Button',
    ThreadActionFeedback: 'ThreadActionFeedback', ThreadComposer: 'ThreadComposer',
  });
  const open = body({ client: {}, thread: { ...thread, state: 'open', unread: false }, onOpen() {} });
  assert.equal(button(open, 'Rename').props.disabled, false);
  assert.equal(button(open, 'Close thread').props.disabled, false);
  assert.ok(button(open, 'Open thread'));
  assert.equal(button(open, 'Rename').props.hint, 'Renames Review');
  assert.equal(button(open, 'Close thread').props.hint, 'Closes Review');
  assert.equal(button(open, 'Open thread').props.hint, 'Opens Review');
  assert.equal(find(open, 'ThreadComposer').props.contextualTitle, 'Review');
  assert.equal(button(open, 'Refresh history').props.disabled, true, 'refresh announces dimmed while history loads');
  assert.equal(button(open, 'Reopen thread'), undefined);
});

test('a closed thread explains that replying keeps it closed, and a missing reply grant explains the absent composer', () => {
  // Relative labels apply to messages and lifecycle events, independent of their calendar day.
  const at = (day, hour, minute) => new Date(2026, 10, day, hour, minute).getTime();
  let now = at(9, 13, 0);
  const event = (id, kind, accepted) => ({ id, kind, actor_id: 'agent/reviewer', accepted_at_unix_ms: accepted, content: id });
  const items = [ // newest first, as the timeline is served; the Reopen is clock-skewed before the Close
    event('event/closed', 'thread.closed', at(9, 12, 59)), event('event/reopened', 'thread.reopened', at(9, 12, 58)),
    event('message/latest-day', 'message', at(9, 12, 57)), event('message/earlier-day', 'message', at(7, 10, 0)),
  ];
  items[2].thread_quote_id = items[3].id;
  const store = { actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {}, unreadClient, threadActions: new Set(['thread.reply', 'thread.reopen']) };
  const page = { items, loading: false, live: true };
  const state = hooks();
  const body = compile('ThreadBody', 'type Quote', {
    ...state, useCallback: fn => fn, useRef: current => ({ current }), threadEditScope: id => `edit/${id}`,
    useSyncExternalStore: (subscribe, getSnapshot) => {
      assert.equal(subscribe, threadClock.subscribe);
      assert.equal(getSnapshot, threadClock.getSnapshot);
      return now;
    },
    useStore: () => store, useThreadPage: () => page, useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], agentName: () => 'Reviewer', ago: () => '2d', eventLine: event => event.kind,
    View: 'View', T: 'T', Button: 'Button', Markdown: 'Markdown', Pressable: 'Pressable', ContextMenu: 'ContextMenu',
    ThreadComposer: 'ThreadComposer', ThreadActionFeedback: 'ThreadActionFeedback',
    theme: {},
  });
  const thread = { id: 'thread/closed', title: 'Review', state: 'closed', unread: false };
  const render = () => { state.reset(); return body({ client: {}, thread }); };
  const hint = tree => findBy(tree, node => node.props?.accessibilityRole === 'text' && /^Closed(?:,|$)/.test(node.props.accessibilityLabel));

  let tree = render();
  assert.equal(hint(tree).props.accessibilityLabel, 'Closed, replying keeps it closed');
  assert.deepEqual(find(hint(tree), 'T').children, ['Closed · replying keeps it closed']);
  assert.equal(button(tree, 'Reopen thread').props.disabled, false, 'Reopen sits with the hint');
  assert.equal(find(tree, 'ThreadComposer').props.canReply, true, 'the closed composer stays');
  assert.equal(findBy(tree, node => node.children?.includes("Replies aren't available on this connection.")), undefined);
  assert.equal(button(tree, 'Refresh history'), undefined, 'a followed timeline needs no manual refresh');

  const timelineRows = rendered => rendered.children.filter(child => child?.type === 'ContextMenu' || child?.props?.testID?.startsWith('thread-event:'));
  const rows = timelineRows(tree);
  const time = row => find(row, 'Text').children[1];
  const spoken = row => (row.type === 'ContextMenu' ? row.children[0] : row).props.accessibilityLabel;
  assert.deepEqual(rows.map(time), ['2d ago', '3m ago', '2m ago', '1m ago']);
  assert.equal(rows.length, items.length, 'no day heading is inserted between timeline rows');
  assert.equal(tree.children.some(child => child?.type === 'T' && child.props.dim && child.children.length === 1), false, 'calendar-day headings are absent');
  for (const [index, row] of rows.entries()) {
    const message = row.type === 'ContextMenu' ? row.children[0] : row;
    const event = [...items].reverse()[index];
    assert.ok(message.props.accessibilityLabel.includes(threadExactTime(event.accepted_at_unix_ms)), 'each row retains its exact localized date and time');
    assert.equal(message.props.testID, `${event.kind === 'message' ? 'thread-message' : 'thread-event'}:${event.id}`);
    const ageText = find(message, 'Text');
    assert.equal(ageText.props.accessibilityElementsHidden, true);
    assert.equal(ageText.props.importantForAccessibility, 'no');
    if (row.type === 'ContextMenu') assert.equal(row.props.title, threadExactTime(event.accepted_at_unix_ms));
    if (event.thread_quote_id) assert.ok(message.props.accessibilityLabel.includes('Reply to agent/reviewer · message/earlier-day. message/latest-day'), 'grouping retains the quoted sender and original text');
  }
  now += 60_000;
  const updated = timelineRows(render());
  assert.deepEqual(updated.map(time), ['2d ago', '4m ago', '3m ago', '2m ago'], 'a new clock snapshot updates every relative row');
  assert.deepEqual(updated.map(spoken), rows.map(spoken), 'message and lifecycle labels remain stable while their visible relative ages change');
  page.items = [event('message/today', 'message', now)];
  tree = render();
  assert.equal(time(find(tree, 'ContextMenu')), '0s ago');
  const beforeTick = spoken(find(tree, 'ContextMenu'));
  now += 1_000;
  tree = render();
  assert.equal(time(find(tree, 'ContextMenu')), '1s ago', 'a second tick refreshes the youngest message');
  assert.equal(spoken(find(tree, 'ContextMenu')), beforeTick, 'the second tick does not change the accessible label');

  store.threadActions = new Set(['thread.reopen']);
  page.live = false;
  tree = render();
  assert.equal(hint(tree).props.accessibilityLabel, 'Closed');
  assert.equal(find(tree, 'ThreadComposer'), undefined);
  assert.ok(findBy(tree, node => node.type === 'T' && node.children?.includes("Replies aren't available on this connection.")));
  assert.ok(button(tree, 'Refresh history'), 'an HTTP-only timeline keeps manual refresh');
  page.issue = 'Temporary timeline read failure';
  let retries = 0;
  page.reload = () => { retries++; page.issue = undefined; };
  const retry = button(render(), 'Retry');
  assert.ok(retry, 'timeline errors have an in-place Retry control');
  assert.equal(retry.props.accessibilityLabel, 'Retry loading Review');
  retry.props.onPress();
  assert.equal(retries, 1);
  assert.equal(button(render(), 'Retry'), undefined, 'successful recovery clears the error control');
});

test('the standalone thread names its navigation bar, follows renames and drops the duplicate title line', () => {
  const titles = [];
  const navigation = { setOptions: options => titles.push(options.title) };
  let project = thread => thread;
  const content = compile('ThreadDetailContent', 'function threadSummaryLine', {
    ...hooks(), useStore: () => ({ threadEdits: { project: thread => project(thread) } }), useNavigation: () => navigation,
    useEffect: effect => effect(), threadSummaryLine: summaryLine,
    ThreadRow: 'ThreadRow', ThreadBody: 'ThreadBody', T: 'T', Button: 'Button',
  });
  const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false, message_count: 1 };
  const tree = content({ client: {}, thread, loading: false, reload() {} });
  assert.deepEqual(tree.children[0], { type: 'T', props: { dim: true }, children: ['1 message · open'] });
  assert.equal(find(tree, 'ThreadRow'), undefined);
  assert.deepEqual(titles, ['Review']);
  project = summary => ({ ...summary, title: 'Renamed' });
  content({ client: {}, thread, loading: false, reload() {} });
  assert.deepEqual(titles, ['Review', 'Renamed']);
  const embedded = find(content({ client: {}, thread, loading: false, reload() {}, embedded: true }), 'ThreadRow');
  assert.equal(embedded.props.label, 'Renamed, 1 message, open');
  assert.equal(titles.length, 2, 'embedded threads never retitle the host screen');
});

test('pull to refresh on thread hosts reloads thread readers, which drop manual refresh while followed', async () => {
  const calls = [];
  const state = hooks();
  const refresh = compile('useRefresh', '\ntype Scrollable', {
    ...state, useCallback: fn => fn, RefreshControl: 'RefreshControl', theme: { overlay1: 'overlay1' },
    useStore: () => ({
      status: 'online', inventoryGranted: true, feed: { refresh: () => calls.push('feed') },
      loadLists: async keys => { calls.push(['lists', keys]); },
      actions: { reconnect: () => calls.push('reconnect'), refreshThreads: () => calls.push('threads') },
    }),
  }, './chrome.tsx');
  const control = refresh();
  assert.equal(control.type, 'RefreshControl');
  assert.equal(control.props.refreshing, false);
  control.props.onRefresh();
  state.reset();
  assert.equal(refresh().props.refreshing, true);
  await new Promise(resolve => setImmediate(resolve));
  state.reset();
  assert.equal(refresh().props.refreshing, false);
  assert.deepEqual(calls, ['feed', 'threads', ['lists', []]]);

  // Every screen that hosts thread readers wires that native control.
  const source = file => readFileSync(new URL(file, import.meta.url), 'utf8');
  const conversation = source('./screens/Conversation.tsx');
  assert.match(conversation, /const pullRefresh = useRefresh\(\);/);
  assert.match(conversation, /<FlatList[\s\S]*?inverted\s+refreshControl=\{pullRefresh\}/);
  assert.match(conversation, /<Freshness refreshing=\{refreshing\}/, 'the follow freshness line keeps its own state');
  const missions = source('./screens/Missions.tsx');
  assert.match(missions.slice(missions.indexOf('export function MissionScreen')), /const refresh = useRefresh\(\);[\s\S]*refreshControl=\{refresh\}[\s\S]*<ThreadGroup/);
  const now = source('./screens/Now.tsx');
  const attention = now.slice(now.indexOf('export function AttentionScreen'));
  assert.match(attention, /const refresh = useRefresh\(\);/);
  assert.equal(attention.match(/<ScrollView [^>]*refreshControl=\{refresh\}/g)?.length, attention.match(/<ScrollView /g).length);
  assert.match(now.slice(0, now.indexOf('export function AttentionScreen')), /refreshControl=\{refresh\}/);
  assert.match(source('./screens/Threads.tsx'), /const refresh = useRefresh\(\);[\s\S]*?<ScrollView ref=\{viewport\}[^>]*refreshControl=\{refresh\}/);
});

test('the thread screen docks its one composer below the scrolling history; embedded threads keep it inline', () => {
  const at = (day, hour, minute) => new Date(2026, 10, day, hour, minute).getTime();
  const page = { items: [{ id: 'message/one', kind: 'message', actor_id: 'agent/reviewer', accepted_at_unix_ms: at(9, 12, 0), content: 'Hello' }], loading: false, live: true };
  const store = { actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {}, unreadClient, threadActions: new Set(['thread.reply', 'thread.reopen', 'thread.rename']) };
  const state = hooks();
  const body = compile('ThreadBody', 'type Quote', {
    ...state, useCallback: fn => fn, useRef: current => ({ current }), threadEditScope: id => `edit/${id}`,
    useStore: () => store, useThreadPage: () => page, useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], agentName: () => 'Reviewer', ago: () => '1m',
    View: 'View', T: 'T', Button: 'Button', Markdown: 'Markdown', Pressable: 'Pressable', ContextMenu: 'ContextMenu',
    ThreadComposer: 'ThreadComposer', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const thread = { id: 'thread/closed', title: 'Review', state: 'closed', unread: false };
  const composers = node => !node || typeof node !== 'object' ? 0 : (node.type === 'ThreadComposer' ? 1 : 0) + (node.children ?? []).reduce((sum, child) => sum + composers(child), 0);
  const framed = () => { state.reset(); return body({ client: {}, thread, frame: (content, dock) => ({ type: 'Frame', props: {}, children: [content, dock] }) }); };

  let [content, dock] = framed().children;
  assert.equal(composers(content) + composers(dock), 1, 'one composer instance, owned by ThreadBody');
  assert.ok(find(dock, 'ThreadComposer'), 'the composer is docked');
  assert.equal(findBy(dock, node => node.props?.accessibilityRole === 'text').props.accessibilityLabel, 'Closed, replying keeps it closed');
  assert.ok(button(dock, 'Reopen thread'), 'Reopen stays beside the docked hint');
  assert.ok(find(content, 'ContextMenu'), 'the timeline scrolls above the dock');
  assert.ok(button(content, 'Rename'));
  assert.equal(button(dock, 'Rename'), undefined);

  store.threadActions = new Set(['thread.reopen']);
  [content, dock] = framed().children;
  assert.equal(composers(dock), 0);
  assert.ok(findBy(dock, node => node.type === 'T' && node.children?.includes("Replies aren't available on this connection.")), 'the no-grant line takes the composer\'s place');

  store.threadActions = new Set(['thread.reply']);
  state.reset();
  const inline = body({ client: {}, thread });
  assert.equal(inline.type, 'View');
  assert.equal(composers(inline), 1, 'embedded threads render the same composer inline');

  const frames = [];
  const frame = (content, dock) => { frames.push({ content, dock }); return { type: 'Frame', props: {}, children: [] }; };
  const detail = compile('ThreadDetailContent', 'function threadSummaryLine', {
    ...hooks(), useStore: () => ({}), useNavigation: () => ({ setOptions() {} }), useEffect() {}, threadSummaryLine: summaryLine,
    ThreadRow: 'ThreadRow', ThreadBody: 'ThreadBody', T: 'T', Button: 'Button',
  });
  const framedBody = detail({ client: {}, thread: { ...thread, message_count: 1 }, loading: false, reload() {}, frame });
  assert.equal(framedBody.type, 'ThreadBody');
  framedBody.props.frame('history', 'dock');
  assert.deepEqual(frames[0].content.children[0], { type: 'T', props: { dim: true }, children: ['1 message'] }, 'the summary line scrolls with the history without repeating the closed banner');
  assert.equal(frames[0].dock, 'dock');
  detail({ client: {}, loading: true, reload() {}, frame });
  assert.deepEqual(frames[1].content.children, ['Loading thread…']);
  assert.equal(frames[1].dock, undefined);

  const source = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  const screen = source.slice(source.indexOf('export function ThreadScreen'), source.indexOf('function ThreadDetailContent'));
  assert.match(screen, /<SystemScreen>/, 'the thread screen reads in system text');
  assert.match(screen, /ConversationFrame\(\{[\s\S]*?children: <ScrollView ref=\{viewport\} style=\{\{ flex: 1 \}\}[\s\S]*?<\/ScrollView>,\s*composer: dock,/, 'the common frame docks below the unchanged flex:1 history scroll');
  assert.match(readFileSync(new URL('./conversationPresentation.tsx', import.meta.url), 'utf8'), /<KeyboardAvoidingView[^>]*>[\s\S]*?\{children\}\s*\{composer \? <View/, 'the dock remains a sibling after history');
  assert.match(screen, /<ThreadViewport viewport=\{viewport\}><InlineThread [^>]*frame=\{frame\}/, 'read marks still measure the history scroll');
  assert.match(readFileSync(new URL('./App.tsx', import.meta.url), 'utf8'), /name="Thread" component=\{ThreadScreen\} options=\{\(\{ route \}\) => \(\{ \.\.\.systemOptions,/);
});

test('thread groups read in system text, titles wrap at accessibility sizes and hosted groups grow taller', () => {
  const source = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  assert.match(source, /<SystemText><ContextThreadList /);
  assert.match(readFileSync(new URL('./ui.tsx', import.meta.url), 'utf8'), /export function SystemText[\s\S]*?<AppearanceScope\.Provider value="system">\{children\}<\/AppearanceScope\.Provider>/);

  let fontScale = 1;
  const lines = compile('useTitleLines', 'export function AgentThreads', { useWindowDimensions: () => ({ fontScale }), ACCESSIBILITY_FONT_SCALE: 1.3 });
  assert.equal(lines(), 1);
  fontScale = 1.35;
  assert.equal(lines(), undefined, 'enlarged titles wrap above the reviewed 1.3 font scale');
  fontScale = 1.65;
  assert.equal(lines(), undefined, 'accessibility sizes wrap');
  const wrapping = compile('ThreadRow', 'export function InboxThread', {
    useTextAppearance: () => ({ color: color => color }), useTitleLines: () => undefined, Pressable: 'Pressable', View: 'View', T: 'T', theme: { accent: 'accent' },
  })({ title: 'A long review title', label: 'A long review title, open', expanded: false, toggle() {} });
  assert.equal(wrapping.children[1].props.numberOfLines, undefined);

  const state = hooks();
  let viewports = [];
  const page = { items: [{ id: 'thread/one', title: 'Review', state: 'open' }], snapshots: new Map(), loading: false, live: true, reload() {}, replace() {} };
  const list = compile('ContextThreadList', 'function ContextThreadHeader', {
    ...state, useStore: () => ({ threadRefresh: 0 }), useMemo: compute => compute(), useCallback: fn => fn, useEffect() {}, useFocusEffect() {},
    useRef: current => ({ current }), useWindowDimensions: () => ({ height: 800 }), useTitleLines: () => undefined, useThreadViewports: () => viewports,
    THREAD_WINDOW_SIZE: 20, useThreadPage: () => page, useTextAppearance: () => ({ system: true, colors: {} }), theme: {},
    View: 'View', Pressable: 'Pressable', T: 'T', ScrollView: 'ScrollView', ThreadViewport: 'ThreadViewport', Button: 'Button',
    ExpandedContextThread: 'ExpandedContextThread', ContextThreadHeader: 'ContextThreadHeader', useMarkedUnreadPage: () => ({ items: [] }),
  });
  const render = () => { state.reset(); return list({ context: { agent_id: 'agent/reviewer' }, client: {}, openThread() {}, title: 'Threads' }); };
  const header = find(render(), 'Pressable');
  assert.equal(header.children[0].props.numberOfLines, undefined, 'the group title wraps too');
  header.props.onPress();
  assert.equal(find(render(), 'ScrollView').props.style.maxHeight, 400, 'above a conversation the group keeps half the screen');
  viewports = [{ current: null }];
  assert.equal(find(render(), 'ScrollView').props.style.maxHeight, 600, 'a scrolling host lets the group grow');
});

test('the first draft uses the retained composer through its receipt and accepted summary', async () => {
  const calls = [];
  const id = 'thread/new';
  const state = hooks();
  const draftCache = { current: new Map() };
  let accepted = false;
  const submitted = [];
  const start = {
    cacheKey: 'draft-origin', accepted: false, origin: { agentId: 'agent/reviewer', sourceId: 'notice/source', episode: 'episode/exact' },
    title: 'Review', onSubmitted: target => { submitted.push(target); }, onAccepted: target => { assert.equal(target, id); accepted = true; }, onCancel() {},
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
  assert.deepEqual(submitted, [], 'a confirmed first send needs no unknown-delivery draft handoff');
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
    useState: initial => [initial, () => {}], useRef: current => ({ current }), threadStartScope: value => JSON.stringify([value.sourceId, value.episode]),
    threadDraftKey: value => `thread-draft/${value}`, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  for (let navigation = 0; navigation < 2; navigation++) {
    const tree = component({ client, origin, agentId: 'agent/canonical', initialTitle: 'Notice', canStart: true, onOpen() {} });
    const inline = find(tree, 'InlineThread');
    assert.equal(inline.props.id, undefined);
    assert.equal(inline.props.start.target, undefined);
    assert.equal(inline.props.start.cacheKey, cacheKey);
    assert.deepEqual(inline.props.start.origin, { ...origin, agentId: 'agent/one' });
    assert.deepEqual(find(tree, 'ThreadGroup').props.context, { origin_source: origin.sourceId, origin_episode: origin.episode });
  }
});

test('collapsing the first draft preserves its origin composer for reopening and acceptance', () => {
  const state = hooks();
  const panel = compile('ThreadDetailContent', 'function threadSummaryLine', {
    ...state, useStore: () => ({}), useNavigation: () => ({ setOptions: () => assert.fail('embedded threads keep the host title') }),
    useEffect: effect => effect(), ThreadRow: 'ThreadRow', ThreadBody: 'ThreadBody',
    ThreadActionFeedback: 'ThreadActionFeedback', T: 'T', Button: 'Button',
    threadSummaryLine: thread => thread.state,
  });
  const start = { cacheKey: 'origin-draft', title: 'Notice', accepted: false };
  const props = { client: {}, embedded: true, start };
  let tree = panel(props);
  assert.equal(find(tree, 'ThreadBody').props.start, start);
  find(tree, 'ThreadRow').props.toggle();
  state.reset();
  tree = panel(props);
  assert.equal(find(tree, 'ThreadBody'), undefined);
  find(tree, 'ThreadRow').props.toggle();
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
  new Function('exports', 'threadClient', 'deliveriesByClient', 'setThreadDeliveryRevision', 'threadActionErrorText', 'ConversationAttempts', outputText)(
    exports, client, byClient, () => {}, String, ConversationAttempts);
  let exactRetries = 0;
  const original = Object.freeze({ origin: { source_id: origin.sourceId, episode: origin.episode }, agent_id: origin.agentId, content: 'Original content', title: 'Original title', signature: 'original-signature', idempotency_key: 'original-key' });
  const observed = [];
  const retry = async () => { exactRetries++; observed.push(original); return { how: 'sent' }; };
  const scope = value => JSON.stringify([value.sourceId, value.episode]);
  await exports.track(attemptScope(origin, 'thread/original'), 'thread/original', async () => ({ how: 'unconfirmed', retry }), undefined,
    { origin: { ...origin }, title: original.title });
  const ledger = new Map(byClient.current.get(client).getSnapshot().map(attempt => [attempt.id, { ...attempt.draft, retry: attempt.retry }]));
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
  const props = entry({ sourceId: origin.sourceId, pendingScope: attemptScope(origin, 'thread/original'), initialTitle: 'Changed title', onOpen() {} }).props;
  assert.deepEqual(props.origin, origin);
  assert.deepEqual(props.pending, { target: 'thread/original', agentId: origin.agentId });
  assert.equal(props.initialTitle, original.title);
  assert.equal(props.canStart, false);
  const startPanel = compile('InboxThreadStart', 'function InlineThread', {
    useStore: () => store, useState: initial => [initial, () => {}], useRef: current => ({ current }), threadStartScope: scope,
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
  assert.equal(byClient.current.get(client).getSnapshot().length, 0);
});

test('Inbox recovery lists each independent pending attempt separately, including origins still listed', () => {
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
  assert.equal(recovered.length, 3);
  assert.deepEqual(recovered.map(node => node.props.sourceId), [listed.sourceId, orphan.sourceId, orphan.sourceId]);
  assert.deepEqual(recovered.map(node => node.props.pendingScope), ['listed', 'orphan-first', 'orphan-second']);
  assert.ok(recovered.every(node => node.props.origin === undefined), 'retained context never authorizes a new Start');
});

const findAll = (node, type) => !node || typeof node !== 'object' ? [] : [
  ...(node.type === type ? [node] : []), ...(node.children ?? []).flatMap(child => findAll(child, type)),
];
const episodeScope = value => JSON.stringify([value.sourceId, value.episode]);

test('Starts never coalesce by origin: a retained attempt keeps its own receipt beside an independent new Start', () => {
  const origin = { sourceId: 'step-run/authored-step', episode: 'episode/one' };
  const retained = { ...origin, agentId: 'agent/original' };
  const ledger = new Map([[attemptScope(retained, 'thread/first'), { target: 'thread/first', retry: async () => ({ how: 'sent' }), start: { origin: retained, title: 'First' } }]]);
  const cacheKey = `thread-draft/${episodeScope(origin)}`;
  const render = ({ cached, status = 'online', pendingTarget }) => compile('InboxThreadStart', 'function InlineThread', {
    useStore: () => ({ status, threadRefresh: 0, threadDeliveries: ledger, draftsReady: true, saveDrafts() {},
      draftCache: { current: new Map(cached ? [[cacheKey, JSON.stringify({ text: 'A second question' })]] : []) } }),
    useState: initial => [initial, () => {}], useRef: current => ({ current }), threadStartScope: episodeScope,
    threadDraftKey: value => `thread-draft/${value}`, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  })({ client: {}, origin: pendingTarget ? retained : origin,
    initialTitle: 'Authored step', canStart: !pendingTarget,
    pending: pendingTarget ? { target: pendingTarget, agentId: retained.agentId } : undefined, onOpen() {} });

  const attempt = find(render({ cached: false, pendingTarget: 'thread/first' }), 'InlineThread');
  const fresh = find(render({ cached: true }), 'InlineThread');
  assert.equal(attempt.props.id, 'thread/first');
  assert.equal(attempt.props.start.target, 'thread/first');
  assert.deepEqual(attempt.props.start.origin, retained, 'the retained attempt keeps its exact recipient');
  assert.equal(attempt.props.start.cacheKey, 'thread-draft/start/thread/first');
  assert.equal(fresh.props.id, undefined, 'the new Start has no server thread until it is sent');
  assert.equal(fresh.props.start.target, undefined);
  assert.deepEqual(fresh.props.start.origin, { ...origin, agentId: 'agent/one' });
  assert.equal(fresh.props.start.cacheKey, cacheKey);

  const idle = render({ cached: false });
  assert.equal(findAll(idle, 'InlineThread').length, 0);
  assert.equal(button(idle, 'Start thread').props.disabled, false, 'an unconfirmed attempt never blocks a new Start');
  assert.equal(button(render({ cached: false, status: 'offline' }), 'Start thread').props.disabled, true, 'offline Starts stay unavailable');
});

test('a retained Start of another episode never folds into this resolved notice row and stays recoverable', () => {
  const earlier = { sourceId: 'notice/typed', episode: 'episode/one', agentId: 'agent/original' };
  const later = { sourceId: 'notice/typed', episode: 'episode/two' };
  const ledger = new Map([[attemptScope(earlier, 'thread/earlier'), { target: 'thread/earlier', start: { origin: earlier, title: 'Earlier' } }]]);
  const entry = compile('InboxThread', 'type ThreadStartDraft', {
    useStore: () => ({ threadClient: {}, threadActions: new Set(['thread.start']), threadDeliveries: ledger }),
    threadStartScope: episodeScope, InboxThreadStart: 'InboxThreadStart', T: 'T',
  });
  for (const origin of [later, { sourceId: earlier.sourceId, episode: earlier.episode }]) {
    const props = entry({ sourceId: later.sourceId, origin, agentId: 'agent/current', initialTitle: 'Notice', onOpen() {} }).props;
    assert.deepEqual(props.origin, origin);
    assert.equal(props.pending, undefined, 'fresh recipient is read from canonical context, not the Inbox row');
    assert.equal(props.initialTitle, 'Notice');
    assert.equal(props.canStart, true, 'each resolved episode can Start independently');
  }
  const pending = compile('PendingInboxThreads', 'type ThreadStartDraft', {
    useStore: () => ({ threadClient: {}, threadDeliveries: ledger, data: { attention: [{ source_id: later.sourceId, episode: later.episode }] } }),
    threadStartScope: episodeScope, View: 'View', T: 'T', InboxThread: 'InboxThread',
  })({ onOpen() {} });
  assert.deepEqual(findAll(pending, 'InboxThread').map(node => node.props.pendingScope), [attemptScope(earlier, 'thread/earlier')],
    'the earlier episode is recovered on its own, not under the listed later episode');
});

test('an unconfirmed first send keeps its text with its attempt; a refused one returns it to the composer', async () => {
  const origin = { agentId: 'agent/reviewer', sourceId: 'notice/source', episode: 'episode/exact' };
  const originKey = `thread-draft/${episodeScope(origin)}`;
  const attemptKey = 'thread-draft/start/thread/new';
  for (const outcome of [{ how: 'unconfirmed', retry: async () => ({ how: 'sent' }) }, { how: 'failed', error: 'Refused' }]) {
    const draftCache = { current: new Map([[originKey, JSON.stringify({ text: 'First message' })]]) };
    const store = { status: 'online', threadRefresh: 0, threadDeliveries: new Map(), draftCache, draftsReady: true, saveDrafts() {}, actions: { startThread: async () => outcome } };
    const panel = compile('InboxThreadStart', 'function InlineThread', {
      ...hooks(), useStore: () => store, threadDraftKey: key => `thread-draft/${key}`, threadTitleFromAsk: value => value,
      View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
    });
    const start = find(panel({ client: {}, origin, agentId: origin.agentId, initialTitle: 'Review', canStart: true, onOpen() {} }), 'InlineThread').props.start;
    const composer = compile('ThreadComposer', 'function ThreadComposeField', {
      ...hooks(), useStore: () => store, useRef: current => ({ current }),
      useEffect() {}, useFocusEffect() {}, useCallback: fn => fn, threadDraftKey: key => `thread-draft/${key}`,
      readThreadDraft: JSON.parse, threadTextIssue: () => undefined,
      Crypto: { randomUUID: () => 'new' }, View: 'View', T: 'T', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
    });
    find(composer({ start, canReply: true, selectionVersion: 0 }), 'ThreadComposeField').props.onSend();
    await new Promise(resolve => setImmediate(resolve));
    if (outcome.how === 'unconfirmed') {
      assert.equal(JSON.parse(draftCache.current.get(attemptKey)).text, 'First message');
      assert.equal(draftCache.current.has(originKey), false, 'the next Start for this origin begins empty');
      assert.equal(savedThreadStarts(draftCache.current)[0].target, 'thread/new');
    } else {
      assert.equal(JSON.parse(draftCache.current.get(originKey)).text, 'First message');
      assert.equal(draftCache.current.has(attemptKey), false);
    }
  }
});

test('a refused retained attempt keeps its text discoverable after remount, beside the origin draft', async () => {
  const origin = { sourceId: 'step-run/authored-step', episode: 'episode/one', agentId: 'agent/original' };
  const saved = { target: 'thread/refused', origin, title: 'Authored step' };
  const attemptKey = 'thread-draft/start/thread/refused';
  const originKey = `thread-draft/${episodeScope(origin)}`;
  const draftCache = { current: new Map([
    [attemptKey, JSON.stringify({ text: 'Refused question' })],
    [startRecoveryKey(saved.target), JSON.stringify(saved)],
    [originKey, JSON.stringify({ text: 'Origin draft' })],
  ]) };
  const store = { threadClient: {}, status: 'online', threadRefresh: 0, threadDeliveries: new Map(), draftCache, draftsReady: true, saveDrafts() {} };
  const panel = compile('InboxThreadStart', 'function InlineThread', {
    ...hooks(), useStore: () => store, threadDraftKey: key => `thread-draft/${key}`, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  const fresh = find(panel({ client: {}, origin, agentId: 'agent/current', initialTitle: saved.title, canStart: true, onOpen() {} }), 'InlineThread').props.start;
  assert.equal(fresh.cacheKey, originKey);
  assert.equal(fresh.target, undefined, 'the origin composer never borrows a restored target');
  const pending = compile('PendingInboxThreads', 'type ThreadStartDraft', {
    useStore: () => store, View: 'View', T: 'T', InboxThread: 'InboxThread',
  })({ onOpen() {} });
  const route = find(pending, 'InboxThread');
  assert.equal(route.props.pendingScope, attemptScope(origin, saved.target));
  assert.equal(route.props.origin, undefined, 'restoration grants no new Start authority');
  const copied = [];
  const entry = compile('InboxThread', 'type ThreadStartDraft', {
    ...hooks(), useStore: () => store, threadDraftKey: key => `thread-draft/${key}`,
    Clipboard: { setStringAsync: async text => { copied.push(text); } },
    View: 'View', T: 'T', Button: 'Button', InboxThreadStart: 'InboxThreadStart',
  })(route.props);
  const restored = entry.type(entry.props);
  assert.equal(find(restored, 'ThreadComposeField'), undefined, 'no receipt means no reconstructed retry or changed recipient');
  assert.ok(texts(restored).includes('Refused question'));
  button(restored, 'Copy message').props.onPress();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(copied, ['Refused question']);
  button(restored, 'Discard saved draft').props.onPress();
  assert.equal(draftCache.current.has(attemptKey), false);
  assert.equal(draftCache.current.has(originKey), true, 'discard never deletes the independent origin draft');
});

test('thread buttons and inline rename act immediately without dialogs, flash after acceptance, and show reverted failures', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false };
  const calls = [], pending = [];
  const edits = new ThreadEdits(() => {}, error => String(error));
  const store = {
    actor: 'person/avery', data: { agents: [] }, status: 'online', unreadClient,
    threadActions: new Set(['thread.close', 'thread.reopen', 'thread.rename']), threadEdits: edits,
    actions: {
      editThread: (summary, edit) => {
        calls.push([summary, edit]);
        const server = Promise.withResolvers();
        pending.push(server);
        return edits.run(summary, edit, () => server.promise);
      },
    },
  };
  const state = hooks();
  const body = compile('ThreadBody', 'type Quote', {
    ...state, useStore: () => store, useCallback: fn => fn, useRef: current => ({ current }),
    threadEditScope: id => `edit/${id}`, useThreadPage: () => ({ items: [] }),
    useWindowDimensions: () => ({ width: 400, height: 800 }), useSafeAreaInsets: () => ({}),
    useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [],
    View: 'View', T: 'T', Button: 'Button', Field: 'Field',
    ThreadActionFeedback: 'ThreadActionFeedback', ThreadEditFlash: 'ThreadEditFlash',
  });
  const findButton = (tree, label) => {
    if (!tree || typeof tree !== 'object') return undefined;
    if (tree.type === 'Button' && tree.props.label === label) return tree;
    return tree.children?.map(child => findButton(child, label)).find(Boolean);
  };
  const render = () => { state.reset(); return body({ client: {}, thread }); };
  let tree = render();
  findButton(tree, 'Close thread').props.onPress();
  tree = render();
  assert.equal(findButton(tree, 'Reopen thread').props.disabled, true, 'close state changes before the server completes');
  assert.equal(find(tree, 'ThreadEditFlash'), undefined);
  pending.shift().resolve({ how: 'sent' });
  await new Promise(resolve => setImmediate(resolve));
  tree = render();
  assert.equal(find(tree, 'ThreadEditFlash').props.message, 'Closed "Review" · Reopen thread to undo.');
  assert.equal(findButton(tree, 'Reopen thread').props.disabled, false);
  edits.observe({ ...thread, state: 'closed' });
  thread.state = 'closed';

  findButton(tree, 'Reopen thread').props.onPress();
  tree = render();
  assert.equal(findButton(tree, 'Close thread').props.disabled, true);
  assert.equal(find(tree, 'ThreadEditFlash'), undefined, 'starting another action clears the previous confirmation');
  pending.shift().resolve({ how: 'failed', error: 'Permission was revoked.' });
  await new Promise(resolve => setImmediate(resolve));
  tree = render();
  assert.equal(findButton(tree, 'Reopen thread').props.disabled, false);
  assert.deepEqual(find(tree, 'ThreadActionFeedback').props.result, { how: 'failed', action: 'reopen', error: 'Permission was revoked.' });
  assert.equal(find(tree, 'ThreadEditFlash'), undefined);

  findButton(tree, 'Rename').props.onPress();
  tree = render();
  assert.equal(find(tree, 'Field').props.accessibilityLabel, 'Thread title');
  find(tree, 'Field').props.onChangeText('  New title  ');
  tree = render();
  find(tree, 'Field').props.onSubmitEditing();
  assert.equal(edits.project(thread).title, 'New title');
  assert.equal(find(render(), 'ThreadEditFlash'), undefined);
  pending.shift().resolve({ how: 'sent' });
  await new Promise(resolve => setImmediate(resolve));
  tree = render();
  assert.equal(find(tree, 'ThreadEditFlash').props.message, 'Renamed to "New title"');
  assert.equal(find(tree, 'Field'), undefined);
  findButton(tree, 'Rename').props.onPress();
  tree = render();
  assert.equal(find(tree, 'Field').props.value, 'New title');
  find(tree, 'Field').props.onChangeText('Review');
  tree = render();
  assert.equal(findButton(tree, 'Save title').props.disabled, false, 'rename back to the stale authoritative title must be allowed');
  find(tree, 'Field').props.onSubmitEditing();
  assert.equal(edits.project(thread).title, 'Review', 'the no-change guard uses the displayed title, not the stale request fence');
  pending.shift().resolve({ how: 'sent' });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls.map(([, edit]) => edit), [
    { kind: 'close' }, { kind: 'reopen' }, { kind: 'rename', title: 'New title' }, { kind: 'rename', title: 'Review' },
  ]);
  assert.ok(calls.every(([summary]) => summary === thread), 'requests retain the authoritative summary and its action fence');
});

test('exact-detail replacements retain snapshot provenance and cannot stamp an older row with a newer context window', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const snapshot = {
    id: 'snapshot/context', host_id: 'host/one', store_index: 12,
    created_at: '2026-11-14T00:00:00Z', projection_version: 'client-projection.v0',
  };
  const current = { id: 'thread/one', title: 'Current context title', state: 'open' };
  const old = { ...current, title: 'Old exact title' };
  const oldSnapshot = { ...snapshot, id: 'snapshot/exact-old', store_index: 9 };
  const state = hooks();
  let focus = [], watcher;
  const read = async () => ({ value: { items: [current], next_cursor: null }, snapshot });
  const follow = handlers => { watcher = handlers; return { close() {} }; };
  const pageHook = compile('useThreadPage', 'type ThreadClient', {
    ...state, useRef: current => ({ current }), useCallback: fn => fn, useMemo: compute => compute(),
    useFocusEffect: effect => focus.push(effect),
    sameHead: (left, right) => left.length === right.length && left.every((item, index) => item.id === right[index]?.id),
  });
  const render = () => { state.reset(); focus = []; return pageHook(read, 0, follow); };
  render();
  focus[0]();
  focus[1]();
  watcher.onWindow({ items: [current], hasMore: false, snapshot });
  await new Promise(resolve => setImmediate(resolve));
  let page = render();
  assert.equal(page.snapshots.get(current.id), snapshot);

  const edits = new ThreadEdits(() => {}, error => String(error));
  await edits.run(old, { kind: 'rename', title: 'Optimistic title' }, async () => ({
    how: 'sent', snapshot: { ...snapshot, id: 'snapshot/accepted', store_index: 10 },
  }));
  let exact = { items: [old], snapshots: new Map([[old.id, oldSnapshot]]) };
  const exactHook = compile('useThreadSummary', 'function threadTextIssue', {
    useStore: () => ({ threadEdits: edits }), useCallback: fn => fn, useEffect: effect => effect(),
    useThreadPage: () => exact,
  });
  exactHook({}, old.id, 0, true, page.replace);
  assert.equal(edits.project(old).title, 'Optimistic title', 'the exact summary at 9 must not retire the edit accepted at 10');
  page = render();
  assert.equal(page.items[0], current, 'older detail cannot overwrite the newer context window');
  assert.equal(page.snapshots.get(current.id), snapshot, 'the retained context row keeps its own provenance');

  const newer = { ...current, title: 'Newer exact title' };
  const newerSnapshot = { ...snapshot, id: 'snapshot/exact-newer', store_index: 14 };
  exact = { items: [newer], snapshots: new Map([[newer.id, newerSnapshot]]) };
  exactHook({}, newer.id, 0, true, page.replace);
  page = render();
  assert.equal(page.items[0], newer);
  assert.equal(page.snapshots.get(newer.id), newerSnapshot, 'accepted exact replacements retain per-item snapshot provenance');
  assert.equal(edits.project(newer), newer, 'only the authoritative exact snapshot at 14 retires the overlay');
});

test('Retry re-reads an inline thread page and clears its live error only after a successful read', async () => {
  const state = hooks();
  let focus = [], watcher, reads = 0, failing = true;
  const thread = { id: 'thread/one', title: 'Review', state: 'open' };
  const read = async () => {
    reads++;
    if (failing) throw new Error('Temporary read failure');
    return { value: { items: [thread], next_cursor: null } };
  };
  const follow = handlers => { watcher = handlers; return { close() {} }; };
  const pageHook = compile('useThreadPage', 'type ThreadClient', {
    ...state, useRef: current => ({ current }), useCallback: fn => fn, useMemo: compute => compute(),
    useFocusEffect: effect => focus.push(effect), ClientError, errorText: error => error.message,
    sameHead: (left, right) => left.length === right.length && left.every((item, index) => item.id === right[index]?.id),
  });
  const render = () => { state.reset(); focus = []; return pageHook(read, 0, follow); };
  render();
  focus[0]();
  focus[1]();
  await new Promise(resolve => setImmediate(resolve));
  watcher.onIssue('Updates paused', false);
  assert.equal(render().issue, 'Updates paused');
  render().reload();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(render().issue, 'Updates paused', 'a failed retry does not dismiss the issue');
  failing = false;
  render().reload();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(reads, 3);
  assert.equal(render().issue, undefined);
  assert.deepEqual(render().items, [thread]);
});

test('close, reopen and rename refusals put action copy and the server reason in separate Text elements', () => {
  const announcements = [];
  const feedback = compile('ThreadActionFeedback', 'type ThreadMessage', {
    useEffect: effect => effect(), AccessibilityInfo: { announceForAccessibility: message => announcements.push(message) },
    View: 'View', T: 'Text', Button: 'Button', theme: {},
  });
  for (const action of ['close', 'reopen', 'rename']) {
    const tree = feedback({ result: { how: 'failed', action, error: 'This fixture intentionally rejects thread changes.' }, busy: false, onRetry() {} });
    const lines = tree.children.filter(child => child?.type === 'Text');
    assert.equal(lines.length, 2);
    assert.deepEqual(lines.map(line => line.children), [[`Couldn't ${action} this thread.`], ['This fixture intentionally rejects thread changes.']]);
    assert.equal(announcements.at(-1), `Couldn't ${action} this thread. This fixture intentionally rejects thread changes.`);
    assert.equal(button(tree, 'Retry original change'), undefined, 'a definitive refusal does not gain an automatic/original-request retry');
    assert.equal(findBy(tree, node => node.children?.some(child => typeof child === 'string' && child.includes('You no longer have access'))), undefined);
  }
});

test('contextual composer names include the thread once while standalone names stay unchanged', () => {
  const compose = compile('ThreadComposeField', undefined, { View: 'View', Field: 'Field', Button: 'Button' });
  for (const contextualTitle of [undefined, 'Review', 'Another thread']) {
    for (const text of ['', 'A reply']) {
      const tree = compose({ contextualTitle, text, onChange() {}, onSend() {}, enabled: true, canSend: true });
      const field = find(tree, 'Field');
      const label = contextualTitle ? `Message ${contextualTitle}` : 'Message thread';
      assert.equal([field.props.accessibilityLabel, !text ? field.props.placeholder : undefined].filter(Boolean).join(' '), label);
      assert.equal(field.props.accessibilityLabel, text ? label : undefined);
      assert.equal(button(tree, 'Send').props.accessibilityLabel, contextualTitle ? `Send reply to ${contextualTitle}` : undefined);
    }
  }
});

test('read issues announce changed text and move focus only for revoked thread access', () => {
  const announcements = [], focused = [], nativeView = {};
  let previous, pending;
  const issue = compile('ThreadReadIssue', 'function ThreadActionFeedback', {
    useRef: () => ({ current: nativeView }),
    useEffect: (effect, dependencies) => {
      if (!previous || dependencies.some((value, index) => value !== previous[index])) pending = effect;
      previous = dependencies;
    },
    AccessibilityInfo: { announceForAccessibility: text => announcements.push(text), setAccessibilityFocus: handle => focused.push(handle) },
    findNodeHandle: view => { assert.equal(view, nativeView); return 42; },
    View: 'View', T: 'Text', theme: {},
  });
  const render = message => { const tree = issue({ message }); const effect = pending; pending = undefined; effect?.(); return tree; };
  render('Updates paused');
  render('Updates paused');
  assert.deepEqual(announcements, ['Updates paused']);
  assert.deepEqual(focused, []);
  const revoked = 'You no longer have access to this thread. The grant was revoked.';
  const tree = render(revoked);
  assert.equal(tree.props.accessibilityRole, 'alert');
  assert.equal(tree.props.accessibilityLabel, revoked);
  assert.deepEqual(announcements, ['Updates paused', revoked]);
  assert.deepEqual(focused, [42]);
  render(revoked);
  assert.deepEqual(focused, [42], 'unchanged issue text does not steal focus again');
});

test('composer announces validation issues and a newly applied quote with its sender', () => {
  const announcements = [], refs = [], effectDependencies = [];
  let refIndex = 0, effectIndex = 0, effects = [];
  const state = hooks();
  const draftCache = { current: new Map() };
  const composer = compile('ThreadComposer', 'function ThreadComposeField', {
    ...state, useRef: current => refs[refIndex++] ?? (refs[refIndex - 1] = { current }),
    useEffect: (effect, dependencies) => {
      const index = effectIndex++, previous = effectDependencies[index];
      if (!previous || dependencies.some((value, at) => value !== previous[at])) effects.push(effect);
      effectDependencies[index] = dependencies;
    },
    useFocusEffect() {}, useCallback: fn => fn,
    useStore: () => ({ actions: {}, draftCache, draftsReady: true, saveDrafts() {}, status: 'online' }),
    threadDraftKey: id => id, readThreadDraft: raw => raw ? JSON.parse(raw) : { text: '' },
    threadTextIssue: text => text.startsWith('doc/') ? 'Use message text here.' : undefined,
    rememberBounded: (cache, key, value) => cache.set(key, value),
    AccessibilityInfo: { announceForAccessibility: text => announcements.push(text) },
    theme: {},
    View: 'View', T: 'Text', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  let props = { thread: { id: 'thread/one' }, canReply: true, selectionVersion: 0 };
  const render = () => {
    state.reset(); refIndex = 0; effectIndex = 0; effects = [];
    const tree = composer(props);
    effects.forEach(effect => effect());
    return tree;
  };
  find(render(), 'ThreadComposeField').props.onChange('doc/not-message-text');
  render();
  render();
  assert.deepEqual(announcements, ['Use message text here.']);
  props = { ...props, selectionVersion: 1, selection: { id: 'message/one', from: 'Reviewer', text: 'Original' } };
  render();
  render();
  assert.deepEqual(announcements, ['Use message text here.', 'Replying to Reviewer']);
});

test('expanded summary Retry names the thread without changing the visible Retry title', () => {
  const expanded = compile('ExpandedContextThread', 'export function ThreadScreen', {
    useThreadSummary: () => ({ items: [], issue: 'Updates paused', loading: false, reload() {} }), useStore: () => ({}),
    View: 'View', ContextThreadHeader: 'ContextThreadHeader', ThreadBody: 'ThreadBody', T: 'T', Button: 'Button',
  });
  const tree = expanded({ client: {}, initial: { id: 'thread/one', title: 'Review' }, refresh: 0 });
  assert.equal(button(tree, 'Retry').props.accessibilityLabel, 'Retry loading Review');
  assert.equal(find(tree, 'ThreadReadIssue').props.message, 'Updates paused');
});

test('loading wrappers start the visit entry before the body exists; a reminder that loads later is never captured', async () => {
  const reminder = { marks: [{ mark_id: 'opaque/summary-only', anchor_message_id: 'message/visible' }], anchor_message_id: 'message/visible' };
  const marked = { id: 'thread/one', title: 'Review', unread: false, unread_reminder: reminder };
  const plain = { ...marked, unread_reminder: null };
  const visit = async (wrapper, initial) => {
    // One port instance: the production wrapper and the body hook share its placement registry.
    const ports = nativeVisitPorts();
    const client = {};
    let page = { items: [], loading: true, reload() {} };
    const dependencies = {
      useStore: () => ({ actor: 'person/one' }), useThreadSummary: () => page, useUnreadVisitEntry: ports.useUnreadVisitEntry,
      ThreadDetailContent: 'ThreadDetailContent', View: 'View', ContextThreadHeader: 'ContextThreadHeader',
      ThreadBody: 'ThreadBody', ReminderThreadBody: 'ReminderThreadBody', T: 'T', Button: 'Button',
    };
    const component = wrapper === 'InlineThread'
      ? compile('InlineThread', 'function useThreadSummary', dependencies)
      : compile('ExpandedContextThread', 'export function ThreadScreen', dependencies);
    const render = () => component(wrapper === 'InlineThread'
      ? { client, id: 'thread/one', placement: 'thread:thread/one', refresh: 0 }
      : { client, initial, placement: 'context:{}:thread/one', refresh: 0, onSummary() {}, toggle() {}, onOpen() {}, revealComposer() {} });
    render();
    ports.focus(); // Entry: the summary is still loading.
    page = { items: [marked], loading: false, reload() {} };
    const body = findBy(render(), node => node.type === 'ThreadDetailContent' || node.type === 'ThreadBody');
    const edits = [];
    ports.useUnreadVisit({
      ...ports.options, scope: client, placementKey: body.props.placement, threadId: 'thread/one', actor: 'person/one', summary: marked,
      canEdit: true, api: { state: 'ready', actions: { edit: async edit => { edits.push(edit); return { how: 'sent' }; } } },
    });
    ports.focus(); await ports.flush(); await ports.advance(500);
    return { edits, placement: body.props.placement };
  };
  const inline = await visit('InlineThread');
  assert.equal(inline.placement, 'thread:thread/one', 'the body shares the wrapper placement key');
  assert.deepEqual(inline.edits, [], 'Unknown at navigation entry: the later reminder stays');
  assert.deepEqual((await visit('ExpandedContextThread', plain)).edits, [], 'a listed row without a reminder enters as none');
  const known = await visit('ExpandedContextThread', marked);
  assert.equal(known.placement, 'context:{}:thread/one');
  assert.deepEqual(known.edits, [{ kind: 'clear', observed_marks: ['opaque/summary-only'] }], 'a reminder known at expansion is captured');
});

const texts = node => !node || typeof node !== 'object' ? typeof node === 'string' ? [node] : [] : node.children?.flatMap(texts) ?? [];
const reminderOn = anchor => ({ marks: [{ mark_id: 'opaque/summary-only', anchor_message_id: anchor }], anchor_message_id: anchor });
const bodyDependencies = store => ({
  ...hooks(), useCallback: fn => fn, useRef: current => ({ current }),
  useStore: () => store, threadEditScope: id => `edit/${id}`,
  useThreadPage: () => ({ items: [{ id: 'message/reply', kind: 'message', content: 'Meaningful reply', actor_id: 'agent/one', accepted_at_unix_ms: 0 }] }),
  useWindowDimensions: () => ({ width: 400, height: 800 }), useSafeAreaInsets: () => ({}),
  useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
  useNavigation: () => ({}), useThreadViewports: () => [], theme: {},
  View: 'View', T: 'T', Button: 'Button', Pressable: 'Pressable', Markdown: 'Markdown', ContextMenu: 'ContextMenu',
  ThreadActionFeedback: 'ThreadActionFeedback', ThreadComposer: 'ThreadComposer',
});

test('unread buttons follow the live thread.mark-unread and thread.clear-unread grants and bind the store client per thread', () => {
  const grantSets = [[], ['thread.read'], ['thread.clear-unread'], ['thread.mark-unread'], ['thread.read', 'thread.mark-unread', 'thread.clear-unread']];
  for (const granted of grantSets) for (const reminder of [false, true]) {
    const bound = [];
    const store = {
      actor: 'person/one', data: { agents: [] }, status: 'online', actions: {}, threadActions: new Set(granted),
      // control.messages alone grants nothing here: the thread action capabilities are authoritative.
      caps: { capabilities: [{ id: 'control.messages', state: 'granted' }] },
      unreadClient: { forThread: thread => { bound.push(thread); return { state: 'ready', actions: {} }; } },
    };
    const thread = { id: 'thread/one', title: 'Review', unread: true, state: 'open', unread_reminder: reminder ? reminderOn('message/reply') : null };
    const tree = compile('ThreadBody', 'type Quote', bodyDependencies(store))({ client: {}, thread, placement: 'thread:thread/one' });
    const has = id => granted.includes(id);
    assert.equal(!!button(tree, 'Mark unread'), has('thread.mark-unread') && has('thread.clear-unread'), `${granted}`);
    assert.equal(!!button(tree, 'Mark read'), has('thread.read') || reminder && has('thread.clear-unread'), `${granted}`);
    assert.equal(button(tree, 'Mark unread')?.props.disabled ?? false, false);
    assert.ok(bound.every(summary => summary === thread) && bound.length > 0, 'the body binds the store client to exactly its summary');
    assert.ok(!texts(tree).some(text => /upcoming client API/.test(text)), 'no pending-API copy remains');
  }
});

test('the private reminder marker never changes the natural unread dot or counts', () => {
  const reminder = reminderOn('message/reply');
  assert.equal(summaryLine({ message_count: 3, unread: false, unread_reminder: reminder }, ', ', false), '3 messages, marked unread reminder');
  assert.equal(summaryLine({ message_count: 3, unread: true, unread_reminder: null }, ', ', false), '3 messages, unread');
  assert.equal(summaryLine({ unread: false, anchor: { message_id: 'message/pending', state: 'pending' } }, ', ', false), '');
  const row = threadRow({ title: 'Review', label: 'Review, marked unread reminder', unread: false, reminder: true, expanded: false, toggle() {} });
  const [dot, , marker] = row.children;
  assert.equal(dot.props.style.backgroundColor, 'transparent', 'a reminder alone is not a natural unread dot');
  assert.deepEqual(marker.children, ['Marked unread']);
  const store = { actor: 'person/one', data: { agents: [] }, status: 'online', actions: {}, threadActions: new Set(), unreadClient };
  const tree = compile('ThreadBody', 'type Quote', bodyDependencies(store))({
    client: {}, placement: 'thread:thread/one', thread: { id: 'thread/one', title: 'Review', unread: false, state: 'open', message_count: 1, unread_reminder: reminder },
  });
  const message = findBy(tree, node => node.props?.testID === 'thread-message:message/reply');
  assert.match(message.props.accessibilityLabel, /^Your marked unread return message\. /);
  assert.ok(texts(message).includes('Marked unread · return here'));
});

test('visible evidence counts whole rendered body lines, never paragraph spacing or margins', async () => {
  const evidence = compile('renderedLineEvidence', 'function threadTextIssue', {});
  const { visibleThreadMessages } = await import('./threadVisibility.ts');
  const viewport = { x: 0, y: 0, width: 400, height: 100 };
  const bounds = (y, height) => ({ x: 0, y, width: 400, height });
  // A one-line body whose only line is whole offers its bounds; the shared rule then needs the whole body.
  const oneLine = evidence(viewport, bounds(40, 20), [{ y: 0, height: 20 }]);
  assert.deepEqual(oneLine, { bounds: bounds(40, 20), lineHeight: 20 });
  // A clipped one-line body fails closed, at either edge.
  assert.deepEqual(evidence(viewport, bounds(90, 20), [{ y: 0, height: 20 }]), { lineHeight: 20 });
  assert.deepEqual(evidence(viewport, bounds(-10, 20), [{ y: 0, height: 20 }]), { lineHeight: 20 });
  // One visible line of a two-line body is not evidence, even with its trailing line whole.
  assert.deepEqual(evidence(viewport, bounds(-20, 40), [{ y: 0, height: 20 }, { y: 20, height: 20 }]), { lineHeight: 20 });
  // Two lines separated by a spacer, the second cut off: the trailing line is not visible.
  assert.deepEqual(evidence(viewport, bounds(50, 60), [{ y: 0, height: 20 }, { y: 40, height: 20 }]), { lineHeight: 20 });
  // Two whole lines, last included: bounds are offered to the shared geometry rule.
  const shown = evidence(viewport, bounds(20, 60), [{ y: 0, height: 20 }, { y: 40, height: 20 }]);
  assert.deepEqual(shown, { bounds: bounds(20, 60), lineHeight: 20 });
  assert.deepEqual(visibleThreadMessages(viewport, [
    { id: 'two', ...shown }, { id: 'one', ...oneLine },
    { id: 'clipped-one', ...evidence(viewport, bounds(90, 20), [{ y: 0, height: 20 }]) },
    { id: 'one-of-two', ...evidence(viewport, bounds(-20, 40), [{ y: 0, height: 20 }, { y: 20, height: 20 }]) },
  ], 20), ['two', 'one']);
  assert.deepEqual(evidence(viewport, bounds(0, 40), undefined), {}, 'no laid-out lines fails closed');
  assert.deepEqual(evidence(undefined, bounds(0, 40), [{ y: 0, height: 20 }, { y: 20, height: 20 }]), {});
});

test('a fully visible one-line reply is read, clears its viewed reminder and can be the Mark unread return anchor; a clipped one is not', async () => {
  const { visibleThreadMessages } = await import('./threadVisibility.ts');
  const message = { id: 'message/one-line', kind: 'message', content: 'Short reply', actor_id: 'agent/one', accepted_at_unix_ms: 0 };
  const visit = async ({ bounds, unread, unread_reminder }) => {
    const ports = nativeVisitPorts();
    const reads = [], edits = [];
    const store = {
      actor: 'person/one', data: { agents: [] }, status: 'online',
      threadActions: new Set(['thread.read', 'thread.clear-unread', 'thread.mark-unread']),
      actions: { markThreadRead: async (_thread, ids) => { reads.push([...ids]); return { how: 'sent' }; } },
      unreadClient: { forThread: () => ({ state: 'ready', actions: {
        edit: async edit => { edits.push(edit); return { how: 'sent' }; }, seek: () => assert.fail('the body never seeks'),
      } }) },
    };
    const body = compile('ThreadBody', 'type Quote', {
      ...bodyDependencies(store), useUnreadVisit: options => ports.useUnreadVisit({ ...options, clock: ports.options.clock }), useThreadPage: () => ({ items: [message] }),
      measureThreadViewport: async () => ({ x: 0, y: 0, width: 400, height: 100 }),
      measureThreadBounds: async view => view ? bounds : undefined, visibleThreadMessages, THREAD_WINDOW_SIZE: 20,
    });
    const thread = { id: 'thread/one', title: 'Review', state: 'open', unread, unread_reminder };
    const tree = body({ client: {}, thread, placement: 'thread:thread/one' });
    findBy(tree, node => node.type === 'View' && node.props?.collapsable === false).props.ref({});
    find(tree, 'Markdown').props.onLines([{ y: 0, height: 20 }]);
    ports.focus(); await ports.flush(); await ports.advance(500);
    return { tree, reads, edits };
  };
  const whole = { x: 0, y: 40, width: 400, height: 20 };
  const clipped = { ...whole, y: 90 };
  let result = await visit({ bounds: whole, unread: true, unread_reminder: reminderOn('message/one-line') });
  assert.deepEqual(result.reads, [['message/one-line']], 'a whole one-line reply is natural read after the dwell');
  assert.deepEqual(result.edits, [{ kind: 'clear', observed_marks: ['opaque/summary-only'] }], 'its viewed captured reminder clears');
  result = await visit({ bounds: clipped, unread: true, unread_reminder: reminderOn('message/one-line') });
  assert.deepEqual([result.reads, result.edits], [[], []], 'a clipped one-line reply is no evidence');

  for (const [bounds, expected] of [[whole, [{ kind: 'mark', anchor_message: 'message/one-line', observed_marks: [] }]], [clipped, []]]) {
    result = await visit({ bounds, unread: false, unread_reminder: null });
    button(result.tree, 'Mark unread').props.onPress();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(result.edits, expected, 'only a whole one-line reply can be the return anchor');
    assert.deepEqual(result.reads, []);
  }
});

const twoMarks = anchor => ({ marks: [
  { mark_id: 'opaque/first', anchor_message_id: anchor }, { mark_id: 'opaque/second', anchor_message_id: 'message/later' },
], anchor_message_id: anchor });

test('Clear reminder in a body sends the observed summary marks for a current or scrolled-away anchor without reading, measuring or seeking', async () => {
  const page = { items: [{ id: 'message/reply', kind: 'message', content: 'Meaningful reply', actor_id: 'agent/one', accepted_at_unix_ms: 0 }], next_cursor: null };
  for (const anchor of ['message/reply', 'message/offscreen']) for (const granted of [true, false]) {
    const edits = [];
    const store = {
      actor: 'person/one', data: { agents: [] }, status: 'online',
      threadActions: new Set(granted ? ['thread.read', 'thread.clear-unread'] : ['thread.read']),
      actions: { markThreadRead: () => assert.fail('an explicit clear never acknowledges messages') },
      unreadClient: { forThread: () => ({ state: 'ready', actions: {
        edit: async edit => { edits.push(edit); return { how: 'sent' }; }, seek: () => assert.fail('an explicit clear never seeks'),
      } }) },
    };
    const never = () => assert.fail('an explicit clear needs no geometry or dwell');
    const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: true, unread_reminder: twoMarks(anchor) };
    const tree = compile('ThreadBody', 'type Quote', { ...bodyDependencies(store), measureThreadViewport: never, measureThreadBounds: never })({
      client: {}, thread, placement: 'thread:thread/one',
      around: anchor === 'message/reply' ? { ...page, anchor: { message_id: anchor, state: 'current' } } : undefined,
    });
    const clear = button(tree, 'Clear reminder');
    assert.ok(button(tree, 'Mark read'), 'natural Mark read stays a separate control');
    if (!granted) { assert.equal(clear, undefined, `read-only connections get no clear control (${anchor})`); continue; }
    assert.equal(clear.props.disabled, false);
    clear.props.onPress();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/first', 'opaque/second'] }], `every observed summary mark (${anchor})`);
  }
});

test('loading, rejected, pending and unavailable seeks offer summary-only Clear reminder, keeping retained retries first', async () => {
  for (const anchorState of ['loading', 'rejected', 'pending', 'unavailable']) for (const granted of [true, false]) {
    const edits = [], seeks = [];
    const store = {
      unreadEnabled: true, actor: 'person/one', status: 'online', threadDeliveries: new Map(),
      threadActions: new Set(granted ? ['thread.clear-unread'] : []),
      unreadClient: { forThread: () => ({ state: 'ready', actions: {
        read: () => assert.fail('an explicit clear never acknowledges messages'),
        seek: async id => {
          seeks.push(id);
          if (anchorState === 'loading') return new Promise(() => {});
          if (anchorState === 'rejected') throw new Error('The seek failed.');
          return { state: anchorState, anchor: { message_id: id, state: anchorState } };
        },
        edit: async edit => { edits.push(edit); return { how: 'sent' }; },
      } }) },
    };
    const clearReminder = compile('ClearReminder', 'type Quote', {
      ...hooks(), useStore: () => store, threadEditScope: id => `edit/${id}`,
      View: 'View', Button: 'Button', ThreadActionFeedback: 'ThreadActionFeedback',
    });
    const state = hooks(), effects = [];
    const body = compile('ReminderThreadBody', 'export function ThreadScreen', {
      ...state, useRef: current => ({ current }), useStore: () => store, useEffect: effect => effects.push(effect),
      unreadReadIssue: String, ClearReminder: clearReminder, ThreadReadIssue: 'ThreadReadIssue', ThreadBody: 'ThreadBody', View: 'View', T: 'T', Button: 'Button',
    });
    const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder: twoMarks('message/anchor') };
    const props = { client: {}, thread, placement: 'context:marked:thread/one', onOpen() {} };
    state.reset(); body(props);
    for (const effect of effects.splice(0)) effect();
    await new Promise(resolve => setImmediate(resolve));
    state.reset();
    const tree = body(props);
    if (anchorState === 'loading') assert.deepEqual(texts(find(tree, 'T')), ['Finding the return message…']);
    else if (anchorState === 'rejected') assert.ok(find(tree, 'ThreadReadIssue'), 'the seek error stays visible beside clear');
    else assert.deepEqual(texts(find(tree, 'T')), [anchorPresentation({ message_id: 'message/anchor', state: anchorState }).notice]);
    const control = findBy(tree, node => node.type === clearReminder);
    assert.equal(control.props.thread, thread, 'the control reads the current authoritative summary');
    const rendered = clearReminder(control.props);
    if (!granted) { assert.equal(rendered, null, `read-only: no control beside a ${anchorState} notice`); continue; }
    const clear = button(rendered, 'Clear reminder');
    assert.equal(clear.props.disabled, false);
    clear.props.onPress();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/first', 'opaque/second'] }], `${anchorState}: only the observed summary marks`);
    assert.deepEqual(seeks, ['message/anchor'], 'clearing never seeks again or falls back to newer history');
  }

  // A retained unconfirmed unread change offers its own original retry first and blocks a new clear,
  // with or without the clear grant; the metadata edit's retry waits behind it.
  for (const granted of [true, false]) {
    const retries = [];
    const store = {
      actor: 'person/one', status: 'online', threadActions: new Set(granted ? ['thread.clear-unread'] : []),
      threadDeliveries: new Map([
        ['unread/thread/one', { retry: async () => { retries.push('unread'); return { how: 'sent' }; } }],
        ['edit/thread/one', { retry: async () => { retries.push('edit'); return { how: 'sent' }; } }],
      ]),
      unreadClient: { forThread: () => ({ state: 'ready', actions: { edit: () => assert.fail('no new clear while a delivery is held') } }) },
    };
    const rendered = compile('ClearReminder', 'type Quote', {
      ...hooks(), useStore: () => store, threadEditScope: id => `edit/${id}`,
      View: 'View', Button: 'Button', ThreadActionFeedback: 'ThreadActionFeedback',
    })({ thread: { id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder: twoMarks('message/anchor') } });
    assert.equal(button(rendered, 'Clear reminder')?.props.disabled ?? true, true);
    const feedback = find(rendered, 'ThreadActionFeedback');
    assert.equal(feedback.props.result.how, 'unconfirmed');
    feedback.props.onRetry();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(retries, ['unread']);
  }
});

test('the marked unread list reads through the bounded unread client; typed errors keep rows and are never empty', async () => {
  const errorText = cause => cause instanceof Error ? cause.message : String(cause);
  const unreadReadIssue = compile('unreadReadIssue', 'function markedUnreadIssue', { errorText });
  const markedUnreadIssue = compile('markedUnreadIssue', 'const NO_THREADS', { unreadReadIssue });
  const row = { id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder: reminderOn('message/one') };
  const read = async (list, enabled = true) => {
    const state = hooks();
    let focus, calls = 0;
    const reader = { listMarked: async maxPages => { calls++; assert.equal(maxPages, undefined, 'the client default bound applies'); return list; } };
    const hook = compile('useMarkedUnreadPage', 'type ThreadClient', {
      ...state, useRef: current => ({ current }), useCallback: fn => fn, useFocusEffect: effect => { focus = effect; },
      useStore: () => ({ unreadClient: reader }),
      markedUnreadIssue, NO_THREADS: [], NO_SNAPSHOTS: new Map(),
    });
    state.reset(); hook(enabled, 0);
    focus();
    await new Promise(resolve => setImmediate(resolve));
    state.reset();
    return { page: hook(enabled, 0), calls };
  };
  // Only an authoritative reminder lists a row; natural unread alone never does.
  const natural = { id: 'thread/natural', title: 'Natural', state: 'open', unread: true, unread_reminder: null };
  let { page, calls } = await read({ kind: 'items', items: [natural, row] });
  assert.deepEqual([page.items, page.issue, page.live, page.more, calls], [[row], undefined, false, undefined, 1]);
  ({ page } = await read({ kind: 'items', items: [natural] }));
  assert.deepEqual([page.items, page.issue], [[], undefined], 'natural unread rows never fill the marked list');
  ({ page } = await read({ kind: 'empty' }));
  assert.deepEqual([page.items, page.issue], [[], undefined], 'empty only after cursor exhaustion');
  ({ page, calls } = await read({ kind: 'items', items: [row] }, false));
  assert.deepEqual([page.items, calls], [[], 0], 'other contexts never read the marked list');
  const failures = [
    [{ kind: 'page-bound', maxPages: 20 }, /More marked threads exist than 20 pages/],
    [{ kind: 'invalid-page-bound', maxPages: 0 }, /0-page limit/],
    [{ kind: 'repeated-cursor', cursor: 'c1' }, /repeated a page of marked threads/],
    [{ kind: 'read-failed', reason: { kind: 'refused', code: 'forbidden' } }, /^This connection can't read marked threads\.$/],
    [{ kind: 'read-failed', reason: { kind: 'refused', code: 'rate-limited' } }, /refused to load marked threads \(rate-limited\)/],
    [{ kind: 'read-failed', reason: { kind: 'invalid-response' } }, /invalid response for marked threads/],
    [{ kind: 'read-failed', reason: { kind: 'failed', cause: new Error('offline') } }, /Couldn't load marked threads\. offline/],
  ];
  for (const [error, text] of failures) for (const items of [[], [row]]) {
    ({ page } = await read({ kind: 'error', error, items: [natural, ...items] }));
    assert.match(page.issue, text);
    assert.deepEqual(page.items, items, 'partial marked rows stay beside the error; natural rows are filtered');
  }

  // The group shows an error with no rows rather than disappearing as an empty list.
  const state = hooks();
  let reader, follower, contextItems = [];
  const marked = { items: [], snapshots: new Map(), loading: false, live: false, issue: 'This connection can\'t read marked threads.', reload() {}, replace() {} };
  const list = compile('ContextThreadList', 'function ContextThreadHeader', {
    ...state, useStore: () => ({ threadRefresh: 0, feed: { belongsTo: () => true, followThreadSummaries: () => assert.fail('marked lists are not followed') } }),
    useMemo: compute => compute(), useCallback: fn => fn, useEffect() {}, useFocusEffect() {},
    useRef: current => ({ current }), useWindowDimensions: () => ({ height: 800 }), useTitleLines: () => 1, useThreadViewports: () => [], THREAD_WINDOW_SIZE: 20,
    useThreadPage: (read, _refresh, follow) => { reader = read; follower = follow; return { items: contextItems, snapshots: new Map() }; },
    useMarkedUnreadPage: enabled => enabled ? marked : { items: [] },
    useTextAppearance: () => ({ system: false, palette: {} }), theme: {},
    View: 'View', Pressable: 'Pressable', T: 'T', ScrollView: 'ScrollView', ThreadViewport: 'ThreadViewport', Button: 'Button',
    ExpandedContextThread: 'ExpandedContextThread', ContextThreadHeader: 'ContextThreadHeader',
  });
  const client = { threadList: () => assert.fail('marked rows come only from listMarked') };
  const render = () => { state.reset(); return list({ context: { marked_unread: true }, client, openThread() {}, title: 'Marked unread' }); };
  let tree = render();
  assert.ok(tree, 'an error is never rendered as an empty (hidden) group');
  assert.deepEqual((await reader(undefined, new AbortController().signal)).value, { items: [], next_cursor: null });
  assert.equal(follower({}), undefined);
  find(tree, 'Pressable').props.onPress();
  tree = render();
  assert.equal(find(tree, 'ThreadReadIssue').props.message, 'This connection can\'t read marked threads.');
  marked.items = [row];
  marked.issue = undefined;
  find(render(), 'ContextThreadHeader').props.toggle();
  const expanded = find(render(), 'ExpandedContextThread');
  assert.equal(expanded.props.seekReminder, true, 'opening a marked row seeks its reminder anchor');
  assert.equal(expanded.props.placement, 'context:{"marked_unread":true}:thread/one');

  // No new count: the marked header is its title alone, visually and for VoiceOver,
  // while the rows keep their own markers and natural unread state untouched.
  const header = find(render(), 'Pressable');
  assert.equal(header.props.accessibilityLabel, 'Marked unread');
  assert.deepEqual(texts(header), ['Marked unread', 'Hide']);
  assert.equal(expanded.props.initial, row, 'the row summary, including natural unread, passes through unchanged');
  // Other context groups keep their existing count.
  contextItems = [row];
  state.reset();
  const recent = find(list({ context: { state: 'closed' }, client, openThread() {}, title: 'Recently closed threads' }), 'Pressable');
  assert.equal(recent.props.accessibilityLabel, 'Recently closed threads, 1');
  assert.deepEqual(texts(recent).slice(0, 3), ['Recently closed threads', ' · ', '1']);
});

test('a marked row opens around its reminder anchor; only a current anchor shows a body and seeking never acknowledges', async () => {
  const errorText = cause => cause instanceof Error ? cause.message : String(cause);
  const unreadReadIssue = compile('unreadReadIssue', 'function markedUnreadIssue', { errorText });
  const page = { anchor: { message_id: 'message/anchor', state: 'current' }, items: [], next_cursor: null };
  const open = async (results, unread_reminder = reminderOn('message/anchor')) => {
    const state = hooks();
    const effects = [], seeks = [], committed = [];
    let effectIndex = 0;
    const acknowledge = () => assert.fail('seeking around a reminder never reads or clears');
    // One stable store service, as the store memoizes it per connection.
    const store = { unreadEnabled: true, unreadClient: { forThread: () => ({ state: 'ready', actions: { read: acknowledge, edit: acknowledge, seek: async id => {
      seeks.push(id); return results.shift();
    } } }) } };
    const body = compile('ReminderThreadBody', 'export function ThreadScreen', {
      ...state, useRef: current => ({ current }), useStore: () => store,
      // Effects run only when their dependencies change, as React commits them.
      useEffect: (effect, deps) => {
        const at = effectIndex++;
        if (committed[at] && deps.every((value, index) => Object.is(value, committed[at][index]))) return;
        committed[at] = deps;
        effects.push(effect);
      },
      unreadReadIssue, ThreadBody: 'ThreadBody', ClearReminder: 'ClearReminder', View: 'View', T: 'T', Button: 'Button',
    });
    let thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder };
    const render = async () => {
      state.reset(); effects.length = 0; effectIndex = 0;
      const tree = body({ client: {}, thread, placement: 'context:marked:thread/one', onOpen() {} });
      for (const effect of effects) effect();
      await new Promise(resolve => setImmediate(resolve));
      return tree;
    };
    const first = await render();
    return { first, seeks, render, clear: () => { thread = { ...thread, unread_reminder: null }; } };
  };

  let view = await open([{ state: 'current', anchor: page.anchor, page }]);
  assert.notEqual(view.first.type, 'ThreadBody', 'nothing renders from the head before the anchor is found');
  assert.deepEqual(texts(view.first), ['Finding the return message…']);
  assert.deepEqual(view.seeks, ['message/anchor']);
  let tree = await view.render();
  assert.equal(tree.type, 'ThreadBody');
  assert.equal(tree.props.around, page, 'the body opens on the current anchor page');
  assert.equal(tree.props.placement, 'context:marked:thread/one');
  view.clear();
  tree = await view.render();
  assert.equal(tree.props.around, page, 'clearing the reminder during the visit keeps the anchored page');
  assert.deepEqual(view.seeks, ['message/anchor'], 'no reseek without a new anchor');

  for (const state of ['unavailable', 'pending']) {
    view = await open([{ state, anchor: { message_id: 'message/anchor', state } }]);
    tree = await view.render();
    assert.equal(findBy(tree, node => node.type === 'ThreadBody'), undefined, `${state} anchors render no body`);
    assert.deepEqual(texts(find(tree, 'T')), [anchorPresentation({ message_id: 'message/anchor', state }).notice]);
    assert.ok(button(tree, 'Open thread'));
    assert.ok(!texts(tree).some(text => /unread/i.test(text) && !/reminder is kept/.test(text)), 'no new unread signal');
  }

  view = await open([
    { state: 'error', error: { kind: 'refused', code: 'forbidden' } },
    { state: 'error', error: { kind: 'invalid-response' } },
    { state: 'current', anchor: page.anchor, page },
  ]);
  tree = await view.render();
  assert.equal(find(tree, 'ThreadReadIssue').props.message, 'This connection can\'t read the return message.');
  assert.equal(button(tree, 'Retry').props.accessibilityLabel, 'Retry finding the return message in Review');
  button(tree, 'Retry').props.onPress();
  await view.render();
  tree = await view.render();
  assert.equal(find(tree, 'ThreadReadIssue').props.message, 'The connected Fractal returned an invalid response for the return message.');
  button(tree, 'Retry').props.onPress();
  await view.render();
  tree = await view.render();
  assert.equal(tree.props.around, page);
  assert.deepEqual(view.seeks, ['message/anchor', 'message/anchor', 'message/anchor']);

  view = await open([], null);
  assert.equal(view.first.type, 'ThreadBody', 'without a reminder the thread opens at its latest history');
  assert.equal(view.first.props.around, undefined);
  assert.deepEqual(view.seeks, []);
});

test('threads-v1 v2 hides reminder UI and never requests the new list, seek or actions', async () => {
  const requests = [];
  const caps = { capabilities: [
    { id: 'threads-v1', version: 2, state: 'granted' },
    ...['thread.read', 'thread.mark-unread', 'thread.clear-unread'].map(id => ({ id, version: 1, state: 'granted' })),
  ] };
  const client = new FractalClient({ baseUrl: 'https://v2.invalid', fetchImpl: async (...args) => { requests.push(args); assert.fail('new v3 requests must not reach v2'); } });
  const service = bindUnreadClient(client, {
    supported: () => unreadSupported(caps), canRead: () => true, canWrite: () => true,
    read: async () => ({ how: 'sent' }), submit: () => assert.fail('no reminder write on v2'),
  });
  const store = { unreadEnabled: unreadSupported(caps), unreadClient: service, threadClient: client,
    actor: 'person/one', status: 'online', data: { agents: [] }, actions: {},
    threadActions: new Set(caps.capabilities.filter(c => c.id.startsWith('thread.')).map(c => c.id)),
  };
  const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: true, unread_reminder: reminderOn('message/reply') };
  const group = compile('ThreadGroup', 'function ContextThreadList', {
    useStore: () => store, SystemText: 'SystemText', ContextThreadList: () => assert.fail('v2 must not mount the marked list reader'),
  });
  assert.equal(group({ context: { marked_unread: true }, title: 'Marked unread', openThread() {} }), null);
  assert.ok(group({ context: { agent_id: 'agent/one' }, openThread() {} }), 'legacy thread lists still work on v2');
  const tree = compile('ThreadBody', 'type Quote', bodyDependencies(store))({ client, thread, placement: 'thread:thread/one' });
  assert.equal(button(tree, 'Mark unread'), undefined);
  assert.ok(button(tree, 'Mark read'), 'legacy natural read remains available');
  assert.equal(button(tree, 'Clear reminder'), undefined, 'v2 offers no reminder clear');
  assert.ok(!texts(tree).some(text => /Marked unread|Couldn't load/.test(text)));
  const effects = [];
  const reminderBody = compile('ReminderThreadBody', 'export function ThreadScreen', {
    ...hooks(), useRef: current => ({ current }), useStore: () => store, useEffect: effect => effects.push(effect),
    ThreadBody: 'ThreadBody', T: 'T', View: 'View', Button: 'Button',
  });
  assert.equal(reminderBody({ client, thread, placement: 'context:marked', onOpen() {} }).type, 'ThreadBody');
  for (const effect of effects) effect();
  const upgrade = await service.listMarked();
  const issue = compile('unreadReadIssue', 'function markedUnreadIssue', { errorText: String })(upgrade.error.reason, 'marked threads');
  assert.equal(issue, 'Upgrade the connected Fractal to use unread reminders.');
  assert.deepEqual(requests, []);
});

test('pending episode A never replaces or disables the fresh Start for episode B', () => {
  const old = { sourceId: 'notice/same', episode: 'episode/A', agentId: 'agent/old' };
  const origin = { sourceId: old.sourceId, episode: 'episode/B' };
  const store = { threadClient: {}, threadActions: new Set(['thread.start']), threadDeliveries: new Map([
    [attemptScope(old, 'thread/old'), { target: 'thread/old', start: { origin: old, title: 'Earlier' } }],
  ]) };
  const entry = compile('InboxThread', 'type ThreadStartDraft', {
    useStore: () => store, InboxThreadStart: 'InboxThreadStart', T: 'T',
  });
  const fresh = entry({ sourceId: origin.sourceId, origin, agentId: 'agent/new', initialTitle: 'Current', onOpen() {} });
  assert.equal(fresh.props.origin, origin);
  assert.equal(fresh.props.pending, undefined);
  assert.equal(fresh.props.initialTitle, 'Current');
  assert.equal(fresh.props.canStart, true);
  assert.equal(fresh.props.pending, undefined);
  const recovered = entry({ sourceId: old.sourceId, pendingScope: attemptScope(old, 'thread/old'), initialTitle: 'Ignored', onOpen() {} });
  assert.equal(recovered.props.origin, old);
  assert.deepEqual(recovered.props.pending, { target: 'thread/old', agentId: old.agentId });
  assert.equal(recovered.props.canStart, false);
});

test('same-origin Starts have independent slots and action keys; first recovery cannot replay the second', async () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const track = source.slice(source.indexOf('  async function trackThreadDelivery('), source.indexOf('  const actions ='));
  const start = source.slice(source.indexOf('    startThread(thread:'), source.indexOf('    replyToThread('));
  const scope = source.slice(source.indexOf('export function threadStartScope('), source.indexOf('export function threadEditScope(')).replaceAll('export ', '');
  const { outputText } = ts.transpileModule(`${scope}\n${track}\nconst actions = {${start}}; exports.start = actions.startThread;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const client = {}, byClient = { current: new WeakMap() }, exports = {}, sent = [];
  let nextKey = 0;
  const submit = async (target, _kind, _revisions, prepare) => {
    const id = `action/${++nextKey}`;
    const member = { threadStart: async request => { sent.push(request); return { how: 'sent' }; } };
    const prepared = await prepare(member, id);
    const request = { id, idempotency_key: id };
    const run = () => prepared.send(request);
    await run();
    return { how: 'unconfirmed', retry: run };
  };
  new Function('exports', 'threadClient', 'deliveriesByClient', 'setThreadDeliveryRevision',
    'threadActionErrorText', 'submitThreadAction', 'signingKeyRef', 'readPhoneSigning', 'signThreadMessage', 'ConversationAttempts', outputText)(
    exports, client, byClient, () => {}, String, submit, { current: {} },
    async () => ({ person: 'person/one' }), async key => ({ signature: `signature/${key}`, messageId: `message/${key}` }), ConversationAttempts);
  const origin = { sourceId: 'notice/same', episode: 'episode/same', agentId: 'agent/one' };
  const first = await exports.start('thread/first', origin, 'First', 'One');
  const second = await exports.start('thread/second', origin, 'Second', 'Two');
  assert.equal(first.how, 'unconfirmed');
  assert.equal(second.how, 'unconfirmed');
  assert.deepEqual(sent.map(request => request.parameters.target_id), ['thread/first', 'thread/second']);
  assert.notEqual(sent[0].id, sent[1].id);
  assert.notEqual(sent[0].idempotency_key, sent[1].idempotency_key);
  const ledger = byClient.current.get(client);
  assert.deepEqual(ledger.getSnapshot().map(attempt => attempt.id), [attemptScope(origin, 'thread/first'), attemptScope(origin, 'thread/second')]);
  assert.equal((await first.retry()).how, 'sent');
  assert.deepEqual(sent[2], sent[0], 'first recovery retains only its original signed intent and key');
  assert.deepEqual(ledger.getSnapshot().map(attempt => attempt.id), [attemptScope(origin, 'thread/second')], 'only the first slot is released');
  assert.equal((await second.retry()).how, 'sent');
  assert.deepEqual(sent[3], sent[1]);
  assert.equal(ledger.getSnapshot().length, 0);
});

test('an unconfirmed first composer moves to separate recovery and leaves a fresh UUID-free Start available', () => {
  const state = hooks(), origin = { sourceId: 'notice/same', episode: 'episode/same' }, client = {};
  const draftCache = { current: new Map() }, threadDeliveries = new Map();
  const component = compile('InboxThreadStart', 'function InlineThread', {
    ...state, useStore: () => ({ status: 'online', threadRefresh: 0, threadDeliveries, draftCache, draftsReady: true, saveDrafts() {} }),
    threadStartScope: episodeScope, threadDraftKey: value => value, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  const props = { client, origin, agentId: 'agent/one', initialTitle: 'Notice', canStart: true, onOpen() {} };
  const render = () => { state.reset(); return component(props); };
  button(render(), 'Start thread').props.onPress();
  const first = find(render(), 'InlineThread').props.start;
  threadDeliveries.set(attemptScope(origin, 'thread/first'), { target: 'thread/first', start: { origin, title: 'Notice' }, retry() {} });
  first.onSubmitted('thread/first', JSON.stringify({ text: '' }));
  const after = render();
  assert.equal(find(after, 'InlineThread'), undefined, 'the first signed request is recovered separately');
  assert.equal(button(after, 'Start thread').props.disabled, false);
  button(after, 'Start thread').props.onPress();
  assert.equal(find(render(), 'InlineThread').props.start.target, undefined, 'the next send allocates a new thread UUID');
  assert.ok(threadDeliveries.has(attemptScope(origin, 'thread/first')), 'the first attempt remains recoverable');
});

test('a nonempty Start draft transfers intact to its own recovery across remount and a durable store restart', async () => {
  const origin = { sourceId: 'notice/same', episode: 'episode/same' }, client = {};
  const originKey = 'thread-draft/["notice/same","episode/same"]', attemptKey = 'thread-draft/start/thread/first';
  const original = JSON.stringify({ text: 'Original first message\nwith a second line', quote: { id: 'message/quote', from: 'Agent', text: 'Exact quoted words' } });
  const values = new Map(), storage = {
    async getItem(key) { return values.get(key) ?? null; },
    async setItem(key, value) { values.set(key, value); },
  };
  let persistence = new LocalDrafts(storage), writes = [];
  let store = {
    status: 'online', threadRefresh: 0, draftsReady: true,
    draftCache: { current: new Map([[originKey, original]]) },
    threadDeliveries: new Map([[attemptScope(origin, 'thread/first'), { target: 'thread/first', retry() {} }]]),
    saveDrafts() { writes.push(persistence.save('person/one', store.draftCache.current)); },
  };
  store.saveDrafts(); await Promise.all(writes);
  const panel = () => {
    const state = hooks();
    const component = compile('InboxThreadStart', 'function InlineThread', {
      ...state, useStore: () => store, threadStartScope: episodeScope,
      threadDraftKey: key => `thread-draft/${key}`, threadTitleFromAsk: value => value,
      View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
    });
    return props => { state.reset(); return component(props); };
  };
  const props = { client, origin, agentId: 'agent/one', initialTitle: 'Notice', canStart: true, onOpen() {} };
  const first = find(panel()(props), 'InlineThread').props.start;
  first.onSubmitted('thread/first', original);
  await Promise.all(writes);
  assert.equal(store.draftCache.current.get(attemptKey), original, 'copy precedes the persisted origin deletion');
  assert.equal(store.draftCache.current.has(originKey), false);
  const parse = compile('readThreadDraft', 'function ThreadComposer', {});
  const assertRemount = () => {
    const fresh = panel()(props);
    assert.equal(find(fresh, 'InlineThread'), undefined, 'the fresh origin composer is empty and independent');
    assert.equal(button(fresh, 'Start thread').props.disabled, false);
    const recovery = find(panel()({ ...props, canStart: false, pending: { target: 'thread/first', agentId: 'agent/one' }, showHistory: false }), 'InlineThread').props.start;
    assert.equal(recovery.cacheKey, attemptKey);
    assert.deepEqual(parse(store.draftCache.current.get(recovery.cacheKey)), JSON.parse(original), 'the recovered composer reads the original text and quote');
    return recovery;
  };
  const remounted = assertRemount();
  // A retry that remains unknown must not delete text typed into a later fresh composer.
  const later = JSON.stringify({ text: 'A separate next Start' });
  store.draftCache.current.set(originKey, later);
  remounted.onSubmitted('thread/first', original);
  await Promise.all(writes);
  assert.equal(store.draftCache.current.get(originKey), later);
  assert.equal(store.draftCache.current.get(attemptKey), original);
  store.draftCache.current.delete(originKey); store.saveDrafts(); await Promise.all(writes);
  // Simulate process loss: neither a cache nor a bound-delivery closure is reused.
  persistence = new LocalDrafts(storage);
  store = { ...store, draftCache: { current: new Map() }, threadDeliveries: new Map() };
  assert.equal(await persistence.restore('person/one', store.draftCache.current), 2);
  store.threadClient = client;
  const pending = compile('PendingInboxThreads', 'type ThreadStartDraft', {
    useStore: () => store, threadStartScope: episodeScope,
    View: 'View', T: 'T', InboxThread: 'InboxThread',
  })({ onOpen() {} });
  const route = find(pending, 'InboxThread');
  assert.ok(route, 'production PendingInboxThreads discovers the restored attempt without a ledger');
  const copied = [], state = hooks();
  const entry = compile('InboxThread', 'type ThreadStartDraft', {
    ...state, useStore: () => store, threadStartScope: episodeScope,
    threadDraftKey: key => `thread-draft/${key}`, readThreadDraft: parse,
    Clipboard: { setStringAsync: async text => { copied.push(text); } },
    View: 'View', T: 'T', Button: 'Button', InboxThreadStart: 'InboxThreadStart',
  })(route.props);
  const restored = entry.type(entry.props);
  assert.ok(texts(restored).includes(JSON.parse(original).text));
  assert.ok(texts(restored).some(text => text.includes('Delivery status unknown after restart')));
  assert.equal(find(restored, 'ThreadComposeField'), undefined, 'no reconstructed original send or old-target write');
  button(restored, 'Copy message').props.onPress();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(copied, [JSON.parse(original).text]);
  const fresh = panel()(props);
  assert.equal(find(fresh, 'InlineThread'), undefined);
  assert.equal(button(fresh, 'Start thread').props.disabled, false);
  button(restored, 'Discard saved draft').props.onPress();
  await Promise.all(writes);
  assert.equal(savedThreadStarts(store.draftCache.current).length, 0);
  assert.equal(store.draftCache.current.has(attemptKey), false);
});

for (const divergence of ['before invocation', 'while pending', 'owner switched', 'generation reset'])
for (const outcome of ['unconfirmed', 'sent'])
test(`a Start fences its ${outcome} draft result when ${divergence}`, async () => {
  const origin = { sourceId: 'notice/delayed', episode: 'episode/one' }, client = {};
  const originKey = 'thread-draft/["notice/delayed","episode/one"]';
  const original = JSON.stringify({ text: 'Submitted original', quote: { id: 'message/quote', from: 'Agent', text: 'Original quote' } });
  const later = JSON.stringify({ text: 'Typed after navigating back' });
  const pending = Promise.withResolvers(), calls = [], saves = [];
  const store = {
    status: 'online', threadRefresh: 0, draftsReady: true,
    draftsOwner: { current: 'person/avery' }, draftGeneration: { current: 0 },
    draftCache: { current: new Map([[originKey, original]]) }, threadDeliveries: new Map(),
    saveDrafts() { saves.push([...store.draftCache.current]); },
    actions: { startThread: async (...args) => { calls.push(args); return pending.promise; } },
  };
  const props = { client, origin, agentId: 'agent/one', initialTitle: 'Notice', canStart: true, onOpen() {} };
  const createPanel = () => compile('InboxThreadStart', 'function InlineThread', {
    ...hooks(), useStore: () => store, threadStartScope: episodeScope,
    threadDraftKey: key => `thread-draft/${key}`, threadTitleFromAsk: value => value,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
  });
  const start = find(createPanel()(props), 'InlineThread').props.start;
  const composer = compile('ThreadComposer', 'function ThreadComposeField', {
    ...hooks(), useStore: () => store, useRef: current => ({ current }), useEffect() {}, useFocusEffect() {},
    useCallback: callback => callback,
    threadDraftKey: key => `thread-draft/${key}`, threadStartScope: episodeScope,
    readThreadDraft: JSON.parse, threadTextIssue: () => undefined,
    Crypto: { randomUUID: () => 'first' }, View: 'View', T: 'T', Button: 'Button',
    ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  })({ start, canReply: true, selectionVersion: 0 });
  if (divergence === 'before invocation') store.draftCache.current.set(originKey, later);
  find(composer, 'ThreadComposeField').props.onSend();
  assert.equal(calls.length, 1);
  assert.equal(calls[0][3], 'Submitted original');
  // Remounting while the network is pending can legitimately create later typing.
  if (divergence === 'while pending') store.draftCache.current.set(originKey, later);
  const ownershipChanged = divergence === 'owner switched' || divergence === 'generation reset';
  if (ownershipChanged) {
    // clearCaches resets this same Map in place, then the next owner can type
    // even the exact same text. Neither compare-delete nor persistence is safe.
    store.draftCache.current.clear();
    store.draftCache.current.set(originKey, original);
    if (divergence === 'owner switched') store.draftsOwner.current = 'person/ada';
    else store.draftGeneration.current++;
  }
  const remounted = find(createPanel()(props), 'InlineThread').props.start;
  assert.equal(remounted.target, undefined);
  pending.resolve({ how: outcome, retry: async () => ({ how: 'sent' }) });
  await new Promise(resolve => setImmediate(resolve));
  if (ownershipChanged) {
    assert.deepEqual([...store.draftCache.current], [[originKey, original]], 'no old-owner draft, metadata or cleanup touches the reset cache');
    assert.deepEqual(saves, [], 'the old completion cannot persist into the new owner');
    assert.equal(calls.length, 1, 'no reconstruction or resend');
    return;
  }
  if (outcome === 'sent') {
    assert.equal(store.draftCache.current.get(originKey), later, 'an accepted older Start cannot delete newer origin typing');
    assert.equal(store.draftCache.current.has('thread-draft/start/thread/first'), false);
    assert.deepEqual(savedThreadStarts(store.draftCache.current), []);
    assert.equal(calls.length, 1);
    return;
  }
  assert.equal(store.draftCache.current.get('thread-draft/start/thread/first'), original);
  assert.equal(store.draftCache.current.get(originKey), later);
  assert.deepEqual(savedThreadStarts(store.draftCache.current), [{
    target: 'thread/first', origin: { ...origin, agentId: 'agent/one' }, title: 'Notice',
  }]);
  assert.equal(calls.length, 1, 'the handoff never reconstructs or re-signs the submitted action');
});
