import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { ThreadEdits } from './threadEdits.ts';

const React = { createElement: (type, props, ...children) => ({ type, props, children: children.flat() }) };
const find = (node, type) => findBy(node, candidate => candidate.type === type);
const findBy = (node, match) => !node || typeof node !== 'object' ? undefined : match(node) ? node
  : node.children?.map(child => findBy(child, match)).find(Boolean);
const button = (node, label) => findBy(node, candidate => candidate.type === 'Button' && candidate.props.label === label);

function compile(name, next, dependencies, file = './screens/Threads.tsx') {
  const source = readFileSync(new URL(file, import.meta.url), 'utf8');
  const a = source.search(new RegExp(`function ${name}(?=[(<])`));
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

  // A row shows an unread dot, the title and Closed; count, state and unread are spoken only.
  const thread = { id: 'thread/one', title: 'Review', state: 'closed', unread: true, message_count: 2 };
  const header = compile('ContextThreadHeader', 'function ExpandedContextThread', {
    useStore: () => ({}), threadSummaryLine: summaryLine, ThreadRow: 'ThreadRow',
  })({ thread, open: false, toggle() {} });
  assert.deepEqual({ ...header.props, toggle: undefined }, {
    title: 'Review', label: 'Review, 2 messages, closed, unread', unread: true, trailing: 'Closed', expanded: false, toggle: undefined,
  });
  const row = threadRow(header.props);
  assert.equal(row.props.accessibilityRole, 'button');
  assert.equal(row.props.accessibilityLabel, 'Review, 2 messages, closed, unread');
  assert.deepEqual(row.props.accessibilityState, { expanded: false });
  assert.equal(row.props.style.minHeight, 44);
  const [dot, title, trailing] = row.children;
  assert.deepEqual([dot.props.style.width, dot.props.style.backgroundColor], [8, 'accent']);
  assert.deepEqual(title.children, ['Review'], 'no disclosure glyph, count, state words or age');
  assert.equal(title.props.style.flex, 1);
  assert.deepEqual(trailing.children, ['Closed']);
  const openRow = threadRow({ title: 'Review', label: 'Review, 1 message, open', expanded: true, toggle() {} });
  assert.equal(openRow.children[0].props.style.backgroundColor, 'transparent', 'read rows keep the 8pt spacer');
  assert.equal(openRow.children[2], null, 'open rows show no trailing state');

  const state = hooks();
  const page = { items: [thread], snapshots: new Map(), loading: false, live: false, reload() {}, replace() {} };
  const list = compile('ContextThreadList', 'function ContextThreadHeader', {
    ...state, useStore: () => ({ threadRefresh: 0 }), useMemo: compute => compute(), useCallback: fn => fn, useEffect() {},
    useRef: current => ({ current }), useWindowDimensions: () => ({ height: 800 }), useTitleLines: () => 1, useThreadViewports: () => [], THREAD_WINDOW_SIZE: 20,
    useThreadPage: () => page, useTextAppearance: () => ({ system: false, palette: {} }), theme: {},
    View: 'View', Pressable: 'Pressable', T: 'T', ScrollView: 'ScrollView', ThreadViewport: 'ThreadViewport', Button: 'Button',
    ExpandedContextThread: 'ExpandedContextThread', ContextThreadHeader: 'ContextThreadHeader',
  });
  const render = () => { state.reset(); return list({ context: { agent_id: 'agent/reviewer' }, client: {}, openThread() {}, title: 'Threads' }); };
  let group = find(render(), 'Pressable');
  assert.equal(group.props.accessibilityRole, 'button');
  assert.equal(group.props.accessibilityLabel, 'Threads, 1');
  assert.deepEqual(group.props.accessibilityState, { expanded: false });
  assert.equal(group.props.style.minHeight, 44);
  assert.deepEqual(group.children[0].children[1].children, [' · ', '1'], 'count, not the first thread title');
  assert.deepEqual(group.children[1].children, ['Show']);
  group.props.onPress();
  let shown = render();
  group = find(shown, 'Pressable');
  assert.deepEqual(group.props.accessibilityState, { expanded: true });
  assert.deepEqual(group.children[1].children, ['Hide']);
  assert.equal(button(shown, 'Refresh threads').props.disabled, false, 'an HTTP-only list keeps manual refresh');
  page.live = true;
  page.more = () => {};
  shown = render();
  assert.equal(button(shown, 'Refresh threads'), undefined, 'a followed list refreshes itself');
  assert.ok(button(shown, 'More threads'));
  assert.equal(find(shown, 'Pressable').props.accessibilityLabel, 'Threads, 1 or more');

  const body = compile('ThreadBody', 'type Quote', {
    ...hooks(), useCallback: fn => fn, useRef: current => ({ current }), threadEditScope: id => `edit/${id}`,
    useStore: () => ({ actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {},
      threadActions: new Set(['thread.rename', 'thread.close', 'thread.reopen']) }),
    useThreadPage: () => ({ items: [], loading: true }), useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], View: 'View', T: 'T', Button: 'Button',
    ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const open = body({ client: {}, thread: { ...thread, state: 'open', unread: false }, onOpen() {} });
  assert.equal(button(open, 'Rename').props.disabled, false);
  assert.equal(button(open, 'Close thread').props.disabled, false);
  assert.ok(button(open, 'Open thread'));
  assert.equal(button(open, 'Refresh history').props.disabled, true, 'refresh announces dimmed while history loads');
  assert.equal(button(open, 'Reopen thread'), undefined);
});

test('a closed thread explains that replying keeps it closed, and a missing reply grant explains the absent composer', () => {
  // Fixture-like history days before the capture date: the latest day reads as clock times under its date.
  const at = (day, hour, minute) => new Date(2026, 10, day, hour, minute).getTime();
  const event = (id, kind, accepted) => ({ id, kind, actor_id: 'agent/reviewer', accepted_at_unix_ms: accepted, content: id });
  const items = [ // newest first, as the timeline is served; the Reopen is clock-skewed before the Close
    event('event/closed', 'thread.closed', at(9, 12, 59)), event('event/reopened', 'thread.reopened', at(9, 12, 58)),
    event('message/latest-day', 'message', at(9, 12, 57)), event('message/earlier-day', 'message', at(7, 10, 0)),
  ];
  const store = { actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {}, threadActions: new Set(['thread.reply', 'thread.reopen']) };
  const page = { items, loading: false, live: true };
  const state = hooks();
  const body = compile('ThreadBody', 'type Quote', {
    ...state, useCallback: fn => fn, useRef: current => ({ current }), threadEditScope: id => `edit/${id}`,
    useStore: () => store, useThreadPage: () => page, useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({}), useHeaderHeight: () => 0, useContext: () => 0, BottomTabBarHeightContext: {},
    useNavigation: () => ({}), useThreadViewports: () => [], agentName: () => 'Reviewer', ago: () => '2d', eventLine: event => event.kind,
    View: 'View', T: 'T', Button: 'Button', Markdown: 'Markdown', Pressable: 'Pressable', ContextMenu: 'ContextMenu',
    ThreadComposer: 'ThreadComposer', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const thread = { id: 'thread/closed', title: 'Review', state: 'closed', unread: false };
  const render = () => { state.reset(); return body({ client: {}, thread }); };
  const hint = tree => findBy(tree, node => node.props?.accessibilityRole === 'text');

  let tree = render();
  assert.equal(hint(tree).props.accessibilityLabel, 'Closed, replying keeps it closed');
  assert.deepEqual(find(hint(tree), 'T').children, ['Closed · replying keeps it closed']);
  assert.equal(button(tree, 'Reopen thread').props.disabled, false, 'Reopen sits with the hint');
  assert.equal(find(tree, 'ThreadComposer').props.canReply, true, 'the closed composer stays');
  assert.equal(findBy(tree, node => node.children?.includes("Replies aren't available on this connection.")), undefined);
  assert.equal(button(tree, 'Refresh history'), undefined, 'a followed timeline needs no manual refresh');

  const rows = tree.children.filter(child => child?.type === 'ContextMenu' || child?.type === 'T' && child.props.dim);
  const time = row => row.type === 'ContextMenu' ? row.children[0].children[0].children[1].children[1] : row.children.at(-1);
  assert.deepEqual(rows.map(row => row.type === 'T' && row.children.length === 1 ? `[${row.children[0]}]` : time(row)), [
    '2d ago', `[${new Date(at(9, 0, 0)).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}]`, '12:57', '12:58', '12:59',
  ], 'earlier days keep their age; the latest displayed day is named once and its rows show HH:mm');

  page.items = [event('message/today', 'message', Date.now())];
  tree = render();
  assert.equal(tree.children.filter(child => child?.type === 'T' && child.props.dim && child.children.length === 1).length, 0,
    'a thread active today needs no day heading');

  store.threadActions = new Set(['thread.reopen']);
  page.live = false;
  tree = render();
  assert.equal(hint(tree).props.accessibilityLabel, 'Closed');
  assert.equal(find(tree, 'ThreadComposer'), undefined);
  assert.ok(findBy(tree, node => node.type === 'T' && node.children?.includes("Replies aren't available on this connection.")));
  assert.ok(button(tree, 'Refresh history'), 'an HTTP-only timeline keeps manual refresh');
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
  const store = { actor: 'person/avery', data: { agents: [] }, status: 'online', actions: {}, threadActions: new Set(['thread.reply', 'thread.reopen', 'thread.rename']) };
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
  assert.deepEqual(frames[0].content.children[0], { type: 'T', props: { dim: true }, children: ['1 message · closed'] }, 'the summary line scrolls with the history');
  assert.equal(frames[0].dock, 'dock');
  detail({ client: {}, loading: true, reload() {}, frame });
  assert.deepEqual(frames[1].content.children, ['Loading thread…']);
  assert.equal(frames[1].dock, undefined);

  const source = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  const screen = source.slice(source.indexOf('export function ThreadScreen'), source.indexOf('function ThreadDetailContent'));
  assert.match(screen, /<SystemScreen>/, 'the thread screen reads in system text');
  assert.match(screen, /<KeyboardAvoidingView style=\{\{ flex: 1 \}\}[^>]*>\s*<ScrollView ref=\{viewport\} style=\{\{ flex: 1 \}\}[\s\S]*?<\/ScrollView>\s*\{dock \? <View/, 'the dock is a sibling below a flex:1 scroll');
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
    ...state, useStore: () => ({ threadRefresh: 0 }), useMemo: compute => compute(), useCallback: fn => fn, useEffect() {},
    useRef: current => ({ current }), useWindowDimensions: () => ({ height: 800 }), useTitleLines: () => undefined, useThreadViewports: () => viewports,
    THREAD_WINDOW_SIZE: 20, useThreadPage: () => page, useTextAppearance: () => ({ system: true, colors: {} }), theme: {},
    View: 'View', Pressable: 'Pressable', T: 'T', ScrollView: 'ScrollView', ThreadViewport: 'ThreadViewport', Button: 'Button',
    ExpandedContextThread: 'ExpandedContextThread', ContextThreadHeader: 'ContextThreadHeader',
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

test('thread buttons and inline rename act immediately without dialogs, flash after acceptance, and show reverted failures', async context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false };
  const calls = [], pending = [];
  const edits = new ThreadEdits(() => {}, error => String(error));
  const store = {
    actor: 'person/avery', data: { agents: [] }, status: 'online',
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
  assert.deepEqual(find(tree, 'ThreadActionFeedback').props.result, { how: 'failed', error: 'Permission was revoked.' });
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
