// The shared thread body mounted with React and react-dom in jsdom: its real hooks, Markdown line
// reports, unread visit lifecycle, viewport measurement and the store's bound delivery code.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire, stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';
import ts from 'typescript';
import { API_VERSION, ClientError, actionReceiptCheck, resendBoundAction, sendBoundAction } from '../../clients/typescript/fractal-client/index.ts';
import { THREAD_WINDOW_SIZE } from './feed.ts';
import { markdown } from './markdown.ts';
import { threadClock, threadExactTime, threadRelativeTime } from './threadClock.ts';
import * as threadVisibility from './threadVisibility.ts';
import { applyVisitEffect, pendingUnreadAPI, UnreadVisitController } from './unreadActions.ts';
import { unreadClient as bindUnreadClient, unreadSupported } from './unreadClient.ts';
import { anchorPresentation, reminderFromSummary, unreadVisit } from './unreadVisit.ts';
import { ConversationAttempts } from './conversationAttempts.ts';

// react-dom decides whether it runs in a DOM when it is first loaded, so the document exists first.
// React is loaded beside react-dom so both share one React instance.
const require = createRequire(import.meta.url);
const { JSDOM } = require('jsdom');
const dom = new JSDOM('<!doctype html><html><body><div id="root"></div></body></html>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const { createRoot } = require('react-dom/client');
const React = createRequire(require.resolve('react-dom/client'))('react');
const { act } = React;

// Native views are DOM elements here. The body passes its ref straight to the element, so React
// itself decides when a ref callback is detached (null) and attached again. measureInWindow
// reports the window geometry a test assigns to the element.
dom.window.HTMLElement.prototype.measureInWindow = function measureInWindow(done) {
  const { x, y, width, height } = this.bounds ?? { x: 0, y: 0, width: 0, height: 0 };
  done(x, y, width, height);
};
const hostProps = new Map();
let hostRenders = 0;
const host = type => function Host({ ref, children, ...props }) {
  hostRenders++;
  const id = React.useRef(Symbol(type)).current;
  hostProps.set(id, { type, props });
  React.useEffect(() => () => { hostProps.delete(id); }, [id]);
  return React.createElement('div', { ref, 'data-host': type, 'aria-label': props.accessibilityLabel, 'data-collapsable': props.collapsable === false ? 'false' : undefined }, children);
};
const hosts = Object.fromEntries(['View', 'T', 'Text', 'Button', 'Field', 'Pressable', 'ContextMenu', 'Runs',
  'ThreadComposer', 'ThreadEditFlash', 'ThreadReadIssue', 'RequestHistory', 'HistoryStart'].map(type => [type, host(type)]));
const mounted = () => [...hostProps.values()];
const button = label => mounted().find(({ type, props }) => type === 'Button' && props.label === label)?.props;

const source = file => readFileSync(new URL(file, import.meta.url), 'utf8');
const compilerOptions = { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React };
function compile(file, name, next, dependencies) {
  const text = source(file);
  const a = text.search(new RegExp(`function ${name}(?=[(<])`));
  const b = next === null ? text.length : text.indexOf(`\n${next}`, a);
  assert.ok(a >= 0 && b > a, `${name} in ${file}`);
  dependencies = {
    ConversationComposer: props => compile('./conversationComposer.tsx', 'ConversationComposer', null, dependencies)(props),
    ConversationFeedback: props => compile('./conversationPresentation.tsx', 'ConversationFeedback', null, dependencies)(props),
    ConversationMessage: props => compile('./conversationPresentation.tsx', 'ConversationMessage', null, dependencies)(props), ...dependencies };
  const { outputText } = ts.transpileModule(`${text.slice(a, b)}\nexports.value = ${name};`, { compilerOptions });
  const exports = {};
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exports, React, ...Object.values(dependencies));
  return exports.value;
}
function load(file, modules, globals = {}) {
  const { outputText } = ts.transpileModule(source(file), { compilerOptions });
  const exports = {};
  new Function('require', 'exports', 'React', ...Object.keys(globals), outputText)(name => {
    assert.ok(Object.hasOwn(modules, name), `unexpected module ${name} in ${file}`);
    return modules[name];
  }, exports, React, ...Object.values(globals));
  return exports;
}

// Native ports: one focused route in the foreground, a scroll viewport clipping the window, and
// animation frames the test runs explicitly. A focused screen runs its focus effect after
// commit and again when the callback changes, as React Navigation does.
const frames = new Map();
let frameId = 0;
const flushFrames = () => {
  const due = [...frames.values()];
  frames.clear();
  for (const run of due) run();
};
let navigationFocused = true;
const navigationListeners = { focus: new Set(), blur: new Set(), beforeRemove: new Set() };
const navigation = {
  isFocused: () => navigationFocused,
  addListener: (event, callback) => { navigationListeners[event].add(callback); return () => navigationListeners[event].delete(callback); },
};
function navigate(focused) {
  navigationFocused = focused;
  for (const callback of [...navigationListeners[focused ? 'focus' : 'blur']]) callback();
}
function useNativeFocusEffect(callback) {
  React.useEffect(() => {
    let release = navigation.isFocused() ? callback() : undefined;
    const focus = navigation.addListener('focus', () => { release?.(); release = callback(); });
    const blur = navigation.addListener('blur', () => { release?.(); release = undefined; });
    return () => { release?.(); focus(); blur(); };
  }, [callback]);
}
const route = { key: 'route/thread' };
const scrollHost = { measureInWindow: done => done(0, 0, 400, 600) };
const viewportRef = { current: { getNativeScrollRef: () => scrollHost } };
const viewports = [viewportRef];
const threadViewport = load('./threadViewport.tsx', {
  react: React, 'react-native': { Keyboard: { metrics: () => undefined } }, './threadVisibility': threadVisibility,
});
const { useUnreadVisit, useUnreadVisitEntry } = load('./unreadVisitNative.ts', {
  react: React,
  'react-native': {
    AppState: { currentState: 'active', addEventListener: () => ({ remove() {} }) },
    Keyboard: { addListener: () => ({ remove() {} }) },
  },
  '@react-navigation/native': {
    useNavigation: () => navigation, useRoute: () => route,
    useFocusEffect: useNativeFocusEffect,
  },
  './unreadActions': { applyVisitEffect, UnreadVisitController },
  './unreadVisit': { unreadVisit },
  './threadViewport': { watchThreadViewports: threadViewport.watchThreadViewports, useThreadViewports: () => viewports },
  './ownedPresentation': { ownedPresentations: { isOccluded: () => false, watch: () => () => {} } },
}, {
  requestAnimationFrame: callback => { frames.set(++frameId, callback); return frameId; },
  cancelAnimationFrame: id => { frames.delete(id); },
});

const Markdown = compile('./ui.tsx', 'Markdown', 'export const styles', {
  useRef: React.useRef, markdown, theme: {}, styles: {}, fonts: {}, LINE: 20, Runs: hosts.Runs, Text: hosts.Text, View: hosts.View,
  useTextAppearance: () => ({ system: true, palette: {}, colors: { secondary: 'secondary', action: 'action' }, color: color => color }),
});
const ThreadActionFeedback = compile('./screens/Threads.tsx', 'ThreadActionFeedback', 'type ThreadMessage', {
  useEffect: React.useEffect, AccessibilityInfo: { announceForAccessibility() {} }, theme: {}, View: hosts.View, T: hosts.T, Button: hosts.Button,
});
const renderedLineEvidence = compile('./screens/Threads.tsx', 'renderedLineEvidence', 'function threadTextIssue', {});
const BottomTabBarHeightContext = React.createContext(undefined);

// The store's own delivery code: submitThreadAction, trackThreadDelivery and its unread ports.
const storeSource = source('./store.tsx');
const storeSlice = (start, end) => {
  const a = storeSource.indexOf(start);
  const b = storeSource.indexOf(end, a);
  assert.ok(a >= 0 && b > a, start);
  return storeSource.slice(a, b);
};
const threadEditScope = new Function(`${stripTypeScriptTypes(storeSlice('function threadEditScope(', '\nexport type ThreadStartPresentation'))}\nreturn threadEditScope;`)();
const storeDelivery = stripTypeScriptTypes(storeSlice('  async function submitThreadAction(', '\n  const actions = {')
  + storeSlice('  unreadPorts.current = {', '\n  const unreadClient = useMemo('));

function deliveryStore(thread) {
  let ids = 0, fences = 0, changed = () => {};
  const member = {
    outcome: 'lost', sends: [],
    threadClearUnread: request => answer('clear', request),
    threadMarkUnread: request => answer('mark', request),
  };
  async function answer(kind, request) {
    member.sends.push({ kind, request: structuredClone(request) });
    if (member.outcome === 'lost') throw new ClientError({
      api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: request.id,
      code: 'internal', message: 'The reply was lost.', details: { applied: 'unknown' }, retryable: true,
    }, 502);
    return { value: {
      kind: 'action-result', status: 'completed', action_id: request.id, operation_id: `operation/${member.sends.length}`,
      snapshot_id: 'snapshot/accepted', affected_ids: [thread.id],
    } };
  }
  const deliveriesByClient = { current: new WeakMap() };
  const ledger = () => new Map(deliveriesByClient.current.get(member)?.getSnapshot()
    .filter(attempt => attempt.phase === 'sending' || attempt.phase === 'unconfirmed')
    .map(attempt => [attempt.id, { ...attempt.draft, retry: attempt.retry }]));
  const unreadPorts = { current: undefined };
  const store = {
    actor: 'person/one', data: { agents: [] }, status: 'online', threadRefresh: 0, threadDeliveries: new Map(), reads: [], unreadEnabled: true,
    draftsOwner: { current: 'person/one' }, draftGeneration: { current: 0 },
    threadActions: new Set(['thread.read', 'thread.clear-unread', 'thread.mark-unread']),
    actions: { markThreadRead: async (_summary, messageIds) => { store.reads.push([...messageIds]); return { how: 'sent' }; } },
  };
  const capsRef = { current: { capabilities: [{ id: 'threads-v1', version: 3, state: 'granted' }, ...['thread.clear-unread', 'thread.mark-unread'].map(id => ({ id, state: 'granted' }))] } };
  const statusRef = { current: 'online' };
  const dependencies = {
    threadClient: member, threadActions: store.threadActions, status: 'online', proof: undefined,
    selected: { route: { kind: 'tailscale' } }, fabricMember: undefined, url: 'https://gateway.example', carrier: { snapshot: null },
    threadConnection: { current: member }, actionId: () => `action/ios-${++ids}`,
    fence: async revisions => ({ snapshot_id: `snapshot/${++fences}`, subject_revisions: revisions }),
    actionReceiptCheck, sendBoundAction, resendBoundAction, setThreadRefresh: () => { store.threadRefresh++; changed(); }, ClientError,
    threadActionErrorText: error => error instanceof Error ? error.message : String(error),
    deliveriesByClient, setThreadDeliveryRevision: () => { store.threadDeliveries = new Map(ledger()); changed(); },
    ConversationAttempts,
    unreadPorts, client: member, capsRef, capsClient: { current: member }, statusRef, unreadSupported, actions: store.actions,
    // Only the lost-reply repeat waits; it need not take a real second here.
    setTimeout: callback => setImmediate(callback),
  };
  const { trackThreadDelivery } = new Function(...Object.keys(dependencies), `${storeDelivery}\nreturn { trackThreadDelivery };`)(...Object.values(dependencies));
  store.unreadClient = bindUnreadClient(member, unreadPorts.current);
  return { store, member, ledger, track: trackThreadDelivery, onChange: next => { changed = next; },
    setVersion(version) { capsRef.current.capabilities.find(capability => capability.id === 'threads-v1').version = version; store.unreadEnabled = unreadSupported(capsRef.current); changed(); },
    setGrant(kind, state) { capsRef.current.capabilities.find(capability => capability.id === `thread.${kind}`).state = state; changed(); },
    setOnline(online) { statusRef.current = online ? 'online' : 'offline'; store.status = statusRef.current; changed(); },
  };
}

const settle = async (turns = 12) => { for (let turn = 0; turn < turns; turn++) await new Promise(resolve => setImmediate(resolve)); };
// Every interaction runs inside act, so React commits its updates before the test continues.
const step = work => act(async () => { await work?.(); await settle(); });

// The store provider rerenders consumers when its delivery revision changes.
function bodyComponent(useStore, useThreadPage, extra = {}) {
  return compile('./screens/Threads.tsx', 'ThreadBody', 'type Quote', {
    useCallback: React.useCallback, useContext: React.useContext, useEffect: React.useEffect, useMemo: React.useMemo,
    useRef: React.useRef, useState: React.useState, useSyncExternalStore: React.useSyncExternalStore, BottomTabBarHeightContext,
    useThreadPage, useWindowDimensions: () => ({ width: 400, height: 800 }),
    useSafeAreaInsets: () => ({ top: 0, left: 0, right: 0, bottom: 0 }), useHeaderHeight: () => 0,
    useNavigation: () => navigation, useThreadViewports: () => viewports,
    measureThreadBounds: threadViewport.measureThreadBounds, measureThreadViewport: threadViewport.measureThreadViewport,
    visibleThreadMessages: threadVisibility.visibleThreadMessages, renderedLineEvidence, THREAD_WINDOW_SIZE,
    threadEditScope, threadClock, threadExactTime, threadRelativeTime, reminderFromSummary, anchorPresentation, pendingUnreadAPI, useUnreadVisit,
    agentName: agent => agent.id, theme: {}, Markdown, ThreadActionFeedback, ...hosts,
    useStore, ...extra,
  });
}
async function mountBody(harness, thread, page) {
  const ThreadBody = bodyComponent(() => React.useContext(StoreContext), () => page);
  const StoreContext = React.createContext(harness.store);
  function Store() {
    const [, setRevision] = React.useState(0);
    React.useEffect(() => { harness.onChange(() => setRevision(previous => previous + 1)); return () => harness.onChange(() => {}); }, []);
    return React.createElement(StoreContext.Provider, { value: { ...harness.store } },
      React.createElement(ThreadBody, { client: harness.member, thread, placement: `thread:${thread.id}` }));
  }
  const root = createRoot(document.getElementById('root'));
  await step(() => { root.render(React.createElement(Store)); });
  return { unmount: () => step(() => { root.unmount(); }) };
}

const reminder = { marks: [{ mark_id: 'mark/one', anchor_message_id: 'message/reply' }], anchor_message_id: 'message/reply' };
const message = { id: 'message/reply', kind: 'message', actor_id: 'agent/one', accepted_at_unix_ms: 0, content: 'A reply long enough to wrap onto a second line' };
const page = { items: [message], loading: false, live: true };
// The message body is a laid-out, two-line Markdown paragraph: its geometry reaches the body only
// through Markdown's own onLayout and onTextLayout handlers.
async function layOut(y) {
  const body = document.querySelector('[data-collapsable="false"]');
  assert.ok(body, 'the message body view is mounted');
  body.bounds = { x: 0, y, width: 400, height: 40 };
  const text = mounted().find(({ type, props }) => type === 'Text' && props.onTextLayout);
  assert.ok(text, 'Markdown reports text layout for the body');
  await step(() => {
    text.props.onLayout({ nativeEvent: { layout: { x: 0, y: 0, width: 400, height: 40 } } });
    text.props.onTextLayout({ nativeEvent: { lines: [{ x: 0, y: 0, width: 380, height: 20 }, { x: 0, y: 20, width: 210, height: 20 }] } });
  });
  return body;
}
const press = async label => {
  const props = button(label);
  assert.ok(props, `${label} is shown`);
  assert.equal(props.disabled, false, `${label} is enabled`);
  await step(() => { props.onPress(); });
};

test('laid-out Markdown lines survive the shared clock rerender; a scroll then reads and clears after 500ms', async context => {
  context.mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'], now: 0 });
  frames.clear();
  const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: true, unread_reminder: reminder };
  const harness = deliveryStore(thread);
  harness.member.outcome = 'confirmed';
  const view = await mountBody(harness, thread, page);
  try {
    const body = await layOut(100);
    // The shared one-second clock rerenders the body with unchanged geometry. Native layout
    // is not reported again, so the laid-out lines must remain the visible evidence.
    const rendered = hostRenders;
    await step(() => { context.mock.timers.tick(1000); });
    assert.ok(hostRenders > rendered, 'the clock tick rerendered the mounted body');
    body.bounds = { ...body.bounds, y: 60 };
    await step(() => { threadViewport.threadViewportChanged(viewportRef); flushFrames(); });
    await step(() => { context.mock.timers.tick(499); });
    assert.deepEqual(harness.store.reads, [], 'not before 500ms of stable visibility');
    assert.deepEqual(harness.member.sends, []);
    await step(() => { context.mock.timers.tick(1); });
    assert.deepEqual(harness.store.reads, [['message/reply']], 'the scrolled, still laid-out body is read');
    assert.deepEqual(harness.member.sends.map(({ kind, request }) => [kind, request.parameters]),
      [['clear', { target_id: thread.id, observed_marks: ['mark/one'] }]], 'the captured reminder on the viewed anchor is cleared');
    assert.equal(harness.ledger().size, 0, 'the confirmed clear releases its delivery slot');
  } finally { await view.unmount(); }
});

const unconfirmedThread = () => ({ id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder: reminder });
const sameRequest = (retried, original) => {
  assert.equal(retried.kind, original.kind);
  assert.equal(retried.request.id, original.request.id, 'the original request ID');
  assert.equal(retried.request.idempotency_key, original.request.idempotency_key, 'the original idempotency key');
  assert.deepEqual(retried.request.parameters, original.request.parameters, 'the original observed marks');
  assert.notDeepEqual(retried.request.fence, original.request.fence, 'only the fence is read again');
  const { fence: _oldFence, ...oldIdentity } = original.request;
  const { fence: _newFence, ...retriedIdentity } = retried.request;
  assert.deepEqual(retriedIdentity, oldIdentity, 'all signed fields and original operation identity remain unchanged');
};

test('an unconfirmed Mark read clear is recovered after remount before a held metadata edit, with its original request', async () => {
  frames.clear();
  const thread = unconfirmedThread();
  const harness = deliveryStore(thread);
  let view = await mountBody(harness, thread, page);
  try {
    await layOut(100);
    await press('Mark read');
    assert.deepEqual(harness.member.sends.map(({ kind, request }) => [kind, request.parameters]), [
      ['clear', { target_id: thread.id, observed_marks: ['mark/one'] }], ['clear', { target_id: thread.id, observed_marks: ['mark/one'] }],
    ], 'one lost receipt, repeated once as the identical request');
    assert.equal(harness.member.sends[1].request.id, harness.member.sends[0].request.id);
    assert.deepEqual([...harness.ledger().keys()], [`unread/${thread.id}`], 'the store retains the bound clear');
    assert.ok(button('Retry original change'));
    assert.equal(button('Mark read').disabled, true);

    // A metadata edit whose receipt was also lost is held under its own scope.
    let metadataRetries = 0;
    await step(() => harness.track(threadEditScope(thread.id), thread.id, async () => ({
      how: 'unconfirmed', retry: async () => { metadataRetries++; return { how: 'sent' }; },
    })));
    await view.unmount();
    view = await mountBody(harness, thread, page);
    assert.equal(button('Retry original change').disabled, false, 'the remounted body recovers the retained retry');
    assert.equal(button('Mark read').disabled, true, 'no new edit while a delivery is held');
    assert.equal(button('Mark unread').disabled, true);

    harness.member.outcome = 'confirmed';
    await press('Retry original change');
    assert.equal(harness.member.sends.length, 3);
    sameRequest(harness.member.sends[2], harness.member.sends[0]);
    assert.equal(metadataRetries, 0, 'the unread delivery takes precedence over the metadata edit');
    assert.deepEqual([...harness.ledger().keys()], [threadEditScope(thread.id)], 'the confirmed clear releases its slot');
    assert.equal(button('Mark read').disabled, true, 'the metadata edit is still held');

    await press('Retry original change');
    assert.equal(metadataRetries, 1, 'the metadata edit retries its own original request next');
    assert.equal(harness.member.sends.length, 3);
    assert.equal(harness.ledger().size, 0);
    assert.equal(button('Retry original change'), undefined);
    assert.equal(button('Mark read').disabled, false);
  } finally { await view.unmount(); }
});

test('an unconfirmed Mark unread is recovered after remount and retried with its original request', async () => {
  frames.clear();
  const thread = unconfirmedThread();
  const harness = deliveryStore(thread);
  let view = await mountBody(harness, thread, page);
  try {
    await layOut(100);
    await press('Mark unread');
    assert.deepEqual(harness.member.sends.map(({ kind, request }) => [kind, request.parameters]), [
      ['mark', { target_id: thread.id, anchor_message_id: 'message/reply', observed_marks: ['mark/one'] }],
      ['mark', { target_id: thread.id, anchor_message_id: 'message/reply', observed_marks: ['mark/one'] }],
    ]);
    assert.deepEqual([...harness.ledger().keys()], [`unread/${thread.id}`]);

    await view.unmount();
    view = await mountBody(harness, thread, page);
    assert.equal(button('Retry original change').disabled, false);
    assert.equal(button('Mark unread').disabled, true);

    harness.member.outcome = 'confirmed';
    await press('Retry original change');
    assert.equal(harness.member.sends.length, 3);
    sameRequest(harness.member.sends[2], harness.member.sends[0]);
    assert.equal(harness.ledger().size, 0, 'the confirmed mark releases its slot');
    assert.equal(button('Retry original change'), undefined);
    assert.equal(button('Mark unread').disabled, false);
  } finally { await view.unmount(); }
});

test('bound unread retries never send after negotiated threads-v1 falls back to v2', async () => {
  for (const edit of [{ kind: 'mark', anchor_message: 'message/reply', observed_marks: ['mark/one'] }, { kind: 'clear', observed_marks: ['mark/one'] }]) {
    const thread = unconfirmedThread(), harness = deliveryStore(thread);
    const result = await harness.store.unreadClient.forThread(thread).actions.edit(edit);
    assert.equal(result.how, 'unconfirmed');
    const before = structuredClone(harness.member.sends);
    harness.setVersion(2);
    harness.member.outcome = 'confirmed';
    const retried = await result.retry();
    assert.equal(retried.how, 'unconfirmed', 'a downgrade cannot resolve the earlier unknown outcome');
    assert.deepEqual([...harness.ledger().keys()], [`unread/${thread.id}`], 'retain the bound request until it can be confirmed');
    assert.deepEqual(harness.member.sends, before, 'no original or refreshed unread write is sent to v2');
  }
});

test('specific unread action revocation or offline status blocks a retained retry without losing its identity', async () => {
  for (const kind of ['mark', 'clear']) for (const blockedBy of ['grant', 'offline']) {
    const thread = unconfirmedThread(), harness = deliveryStore(thread);
    const edit = kind === 'mark'
      ? { kind, anchor_message: 'message/reply', observed_marks: ['mark/one'] }
      : { kind, observed_marks: ['mark/one'] };
    const original = await harness.store.unreadClient.forThread(thread).actions.edit(edit);
    assert.equal(original.how, 'unconfirmed');
    const before = structuredClone(harness.member.sends);
    if (blockedBy === 'grant') harness.setGrant(`${kind === 'mark' ? 'mark' : 'clear'}-unread`, 'denied');
    else harness.setOnline(false);
    harness.member.outcome = 'confirmed';
    const blocked = await original.retry();
    assert.equal(blocked.how, 'unconfirmed', `${kind}: ${blockedBy} cannot settle an unknown outcome`);
    assert.deepEqual(harness.member.sends, before, `${kind}: no request while ${blockedBy} is unavailable`);
    assert.deepEqual([...harness.ledger().keys()], [`unread/${thread.id}`]);
    harness.setGrant(`${kind === 'mark' ? 'mark' : 'clear'}-unread`, 'granted');
    harness.setOnline(true);
    const restored = await blocked.retry();
    assert.equal(restored.how, 'sent');
    assert.equal(harness.member.sends.length, before.length + 1);
    sameRequest(harness.member.sends.at(-1), before[0]);
    assert.equal(harness.ledger().size, 0, 'only a legitimate receipt releases the original slot');
  }
});

test('initial mark and clear deliveries settle from the real unread server receipt without a metadata claim', async () => {
  for (const edit of [
    { kind: 'mark', anchor_message: 'message/reply', observed_marks: ['mark/one'] },
    { kind: 'clear', observed_marks: ['mark/one'] },
  ]) {
    const thread = unconfirmedThread(), harness = deliveryStore(thread);
    harness.member.outcome = 'confirmed';
    const result = await harness.store.unreadClient.forThread(thread).actions.edit(edit);
    assert.equal(result.how, 'sent', edit.kind);
    assert.equal(harness.member.sends.length, 1, 'the first real receipt settles without resending');
    assert.equal(harness.ledger().size, 0, 'no unknown delivery remains after confirmation');
  }
});

// Compile the production collection, exact summary, seek, history and composer together.
// Only native hosts and transport are ports; no substitute collection/body UI owns retention.
async function mountMarkedList(harness, initial, { inlineTwin, recentlyClosed = false, deniedStatus = 403 } = {}) {
  let summary = initial, listReads = 0, contextReads = 0, collectionHandlers, listRefused = false, revision = 1;
  const summaryHandlers = new Map();
  const others = [
    { ...initial, id: 'thread/two', title: 'Other one' },
    { ...initial, id: 'thread/three', title: 'Other two' },
  ];
  const members = () => [summary, ...others].filter(thread => recentlyClosed ? thread.state === 'closed' : !!reminderFromSummary(thread));
  const refresh = () => { harness.store.threadRefresh++; harness.setOnline(true); };
  const publish = next => {
    summary = next;
    const snapshot = { host_id: 'host/one', store_index: ++revision };
    summaryHandlers.get(next.id)?.onWindow({ items: [next], snapshot });
    collectionHandlers?.onWindow({ items: members(), snapshot });
  };
  const StoreContext = React.createContext(harness.store);
  const useStore = () => React.useContext(StoreContext);
  const hooks = { useStore, useCallback: React.useCallback, useEffect: React.useEffect,
    useMemo: React.useMemo, useRef: React.useRef, useState: React.useState,
    useFocusEffect: useNativeFocusEffect };
  const appearance = () => ({ system: true, palette: {}, colors: {}, color: value => value });
  const deps = { ...hooks, ...hosts, ClientError, THREAD_WINDOW_SIZE, reminderFromSummary, ACCESSIBILITY_FONT_SCALE: 1.3,
    errorText: error => String(error), theme: {}, useTextAppearance: appearance,
    useNavigation: () => navigation,
    useWindowDimensions: () => ({ height: 800, width: 400, fontScale: 1 }),
    useThreadViewports: () => viewports, threadViewportChanged: threadViewport.threadViewportChanged,
    ThreadViewport: ({ children }) => children, SystemText: ({ children }) => children,
    ScrollView: host('ScrollView'), useUnreadVisitEntry,
    NO_THREADS: [], NO_SNAPSHOTS: new Map(),
  };
  const build = (name, next) => compile('./screens/Threads.tsx', name, next, deps);
  deps.sameHead = build('sameHead', '// The live window');
  deps.useThreadPage = build('useThreadPage', '/** Typed read failures');
  deps.unreadReadIssue = build('unreadReadIssue', 'function markedUnreadIssue');
  deps.markedUnreadIssue = build('markedUnreadIssue', 'const NO_THREADS');
  deps.useMarkedUnreadPage = build('useMarkedUnreadPage', 'type ThreadClient');
  deps.useTitleLines = build('useTitleLines', 'export function AgentThreads');
  deps.useThreadSummary = build('useThreadSummary', '/** Only whole rendered lines');
  deps.threadSummaryLine = build('threadSummaryLine', '/** One thread row');
  deps.ThreadRow = build('ThreadRow', 'export function InboxThread');
  deps.ContextThreadHeader = build('ContextThreadHeader', 'function ExpandedContextThread');
  const Field = ({ ref, value, onChangeText, ...props }) => React.createElement('input', {
    ref, value, onInput: event => onChangeText(event.target.value), 'data-composer': true,
  });
  const composerDeps = { ...deps, Field, ThreadActionFeedback, AccessibilityInfo: { announceForAccessibility() {} },
    threadDraftKey: id => `thread/${id}`, threadStartScope: id => `start/${id}`, readThreadDraft: value => value ? JSON.parse(value) : { text: '' },
    threadTextIssue: () => undefined };
  composerDeps.ThreadComposeField = compile('./screens/Threads.tsx', 'ThreadComposeField', null, composerDeps);
  const ThreadComposer = compile('./screens/Threads.tsx', 'ThreadComposer', 'function ThreadComposeField', composerDeps);
  deps.ThreadBody = bodyComponent(useStore, deps.useThreadPage, { ThreadComposer });
  deps.ThreadActionFeedback = ThreadActionFeedback;
  deps.threadEditScope = threadEditScope;
  deps.anchorPresentation = anchorPresentation;
  deps.reminderClear = build('reminderClear', '/** One thread');
  deps.useThreadEditing = build('useThreadEditing', 'type Quote');
  deps.ClearReminder = build('ClearReminder', '/** An explicit clear');
  deps.ReminderThreadBody = build('ReminderThreadBody', 'export function ThreadScreen');
  deps.ExpandedContextThread = build('ExpandedContextThread', '/** A marked thread');
  deps.ContextThreadList = build('ContextThreadList', 'function ContextThreadHeader');
  const ThreadGroup = build('ThreadGroup', 'function ContextThreadList');
  deps.ThreadDetailContent = build('ThreadDetailContent', 'function threadSummaryLine');
  const InlineThread = build('InlineThread', 'function useThreadSummary');
  Object.assign(harness.store, {
    threadClient: harness.member, draftCache: { current: new Map() }, draftsReady: true, saveDrafts() {},
    feed: { belongsTo: client => client === harness.member,
      followThreadSummaries: (_context, handlers) => { collectionHandlers = handlers; return { close: () => { collectionHandlers = undefined; } }; },
      followThreadTimeline: () => undefined,
      followThreadSummary: (id, handlers) => { summaryHandlers.set(id, handlers); return { close: () => { summaryHandlers.delete(id); } }; } },
  });
  harness.store.threadActions.add('thread.reply');
  const originalUnread = harness.store.unreadClient;
  harness.store.unreadClient = { ...originalUnread,
    listMarked: async () => {
      listReads++;
      if (listRefused) return { kind: 'error', items: [], error: { kind: 'read-failed', reason: { kind: 'refused', code: 'forbidden' } } };
      return { kind: 'items', items: members() };
    },
  };
  harness.member.threadList = async () => {
    contextReads++;
    if (listRefused) throw new ClientError({ api_version: API_VERSION, error_version: 'st3.client.error.v0',
      request_id: 'request/one', code: 'forbidden', message: 'Thread access was revoked.', retryable: false }, deniedStatus);
    return { value: { items: recentlyClosed ? members() : [summary], next_cursor: null } };
  };
  harness.member.threadGet = async id => ({ value: id === inlineTwin?.id ? inlineTwin : id === summary.id ? summary : others.find(thread => thread.id === id) });
  harness.member.threadTimeline = async id => ({ value: {
    items: [{ ...message, id: id === initial.id ? message.id : id === inlineTwin?.id ? 'message/two' : `message/${id}`,
      content: id === initial.id ? message.content : id === inlineTwin?.id ? 'The independently started second message' : `Reply in ${id}`, thread_quote_id: null }],
    next_cursor: null, anchor: { state: 'current', message_id: message.id },
  } });
  const clear = harness.member.threadClearUnread;
  harness.member.threadClearUnread = async request => {
    const result = await clear(request);
    publish({ ...summary, unread_reminder: undefined });
    return result;
  };
  if (recentlyClosed) harness.store.threadActions.add('thread.reopen');
  harness.store.actions.editThread = async (_thread, edit) => {
    assert.equal(edit.kind, 'reopen');
    publish({ ...summary, state: 'open' });
    refresh();
    return { how: 'sent' };
  };
  function Store() {
    const [, update] = React.useState(0);
    React.useEffect(() => { harness.onChange(() => update(value => value + 1)); return () => harness.onChange(() => {}); }, []);
    return React.createElement(StoreContext.Provider, { value: { ...harness.store } },
      React.createElement(React.Fragment, null,
        React.createElement(ThreadGroup, { context: inlineTwin ? { origin_source: 'ask/one', origin_episode: 'episode/one' } : recentlyClosed ? { recently_closed: true } : { marked_unread: true },
          title: inlineTwin ? 'Threads' : recentlyClosed ? 'Recently closed' : 'Marked unread', openThread: () => navigate(false), excludeId: inlineTwin?.id }),
        inlineTwin ? React.createElement(InlineThread, { client: harness.member, id: inlineTwin.id, placement: 'inbox:one',
          refresh: harness.store.threadRefresh, embedded: true, onOpen: () => navigate(false),
          start: { target: inlineTwin.id, cacheKey: 'start/one', accepted: true, title: inlineTwin.title } }) : null));
  }
  const root = createRoot(document.getElementById('root'));
  await step(() => root.render(React.createElement(Store)));
  return { listReads: () => listReads, update: () => step(() => harness.setOnline(true)),
    remoteClear: () => step(() => publish({ ...summary, unread_reminder: undefined })),
    denySummary: id => step(() => summaryHandlers.get(id).onIssue('Thread access was revoked.', true)),
    denyList: () => step(() => { listRefused = true; collectionHandlers?.onIssue('Thread access was revoked.', true); refresh(); }),
    members,
    contextReads: () => contextReads,
    liveList: () => step(() => collectionHandlers.onWindow({ items: members(), hasMore: false, snapshot: { host_id: 'host/one', store_index: ++revision } })),
    unmount: () => step(() => root.unmount()) };
}

for (const ending of ['collapse', 'owner', 'generation', 'read-grant', 'tab-blur', 'push-blur']) {
  test(`marked-list removal preserves the open native reading context until ${ending}`, async context => {
    context.mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'], now: 0 });
    frames.clear();
    const thread = { id: 'thread/one', title: 'Review', state: 'open', unread: false, unread_reminder: reminder };
    const harness = deliveryStore(thread);
    harness.member.outcome = 'confirmed';
    const view = await mountMarkedList(harness, thread);
    const row = label => mounted().find(({ type, props }) => type === 'Pressable' && props.accessibilityLabel?.startsWith(label))?.props;
    try {
      await step(() => row('Marked unread').onPress());
      await step(() => row('Review').onPress());
      const scroll = document.querySelector('[data-host="ScrollView"]');
      const order = () => [...scroll.querySelectorAll('[data-host="Pressable"][aria-label]')].map(node => node.getAttribute('aria-label').split(',')[0])
        .filter(title => ['Review', 'Other one', 'Other two'].includes(title));
      assert.deepEqual(order(), ['Review', 'Other one', 'Other two']);
      scroll.scrollTop = 29;
      const body = await layOut(100);
      const composer = document.querySelector('[data-composer]');
      assert.ok(composer, 'the production composer mounted');
      composer.focus();
      assert.ok(scroll.contains(body), 'the conversation belongs to the real list ScrollView');
      await step(() => {
        const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set;
        setter.call(composer, 'Keep this draft');
        composer.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
      });
      const before = view.listReads();
      await step(() => { threadViewport.threadViewportChanged(viewportRef); flushFrames(); });
      await step(() => { context.mock.timers.tick(500); flushFrames(); });
      assert.equal(harness.member.sends.filter(send => send.kind === 'clear').length, 1);
      assert.ok(view.listReads() > before, 'confirmed clear refreshed marked membership');
      assert.equal(composer.value, 'Keep this draft');
      assert.equal(JSON.parse(harness.store.draftCache.current.get(`thread/${thread.id}`)).text, 'Keep this draft');
      assert.equal(document.querySelector('[data-collapsable="false"]'), body);
      assert.equal(document.querySelector('[data-composer]'), composer);
      assert.equal(document.activeElement, composer);
      assert.equal(document.querySelector('[data-host="ScrollView"]'), scroll);
      assert.equal(scroll.scrollTop, 29, 'the actual ScrollView retains its offset');
      assert.deepEqual(order(), ['Review', 'Other one', 'Other two'], 'membership removal never moves the open row below its neighbors');
      assert.equal(row('Review').accessibilityLabel.includes('marked unread reminder'), false);
      assert.equal(scroll.querySelector('[aria-label^="Review"]').textContent.includes('Marked unread'), false);
      if (ending === 'collapse') await step(() => row('Review').onPress());
      else if (ending === 'tab-blur' || ending === 'push-blur') {
        await step(() => ending === 'push-blur' ? button('Open thread').onPress() : navigate(false));
        assert.equal(document.querySelector('[data-collapsable="false"]'), null, 'navigation exit releases the retained body');
        await step(() => navigate(true));
        assert.equal(row('Review'), undefined, 'refocus cannot restore a cleared membership from the old visit');
        assert.deepEqual(order(), ['Other one', 'Other two']);
      } else {
        if (ending === 'owner') harness.store.actor = 'person/ada';
        if (ending === 'generation') harness.store.draftGeneration.current++;
        if (ending === 'read-grant') harness.store.threadClient = undefined;
        await view.update();
      }
      assert.equal(document.querySelector('[data-collapsable="false"]'), null);
      assert.equal(document.querySelector('[data-composer]'), null);
    } finally { await view.unmount(); navigate(true); }
  });
}

test('same-title accepted Inbox start is visibly and accessibly distinct; each real row collapses only its own body', async () => {
  const original = { id: 'thread/one', title: 'Review', message_count: 1, state: 'open', unread: false };
  const next = { ...original, id: 'thread/two' };
  const harness = deliveryStore(original);
  const view = await mountMarkedList(harness, original, { inlineTwin: next });
  const row = label => mounted().find(({ type, props }) => type === 'Pressable' && props.accessibilityLabel === label)?.props;
  const bodies = () => [...document.querySelectorAll('[data-collapsable="false"]')];
  try {
    await step(() => row('Threads, 1').onPress());
    await step(() => row('Review, 1 message, open').onPress());
    assert.equal(bodies().length, 2);
    const [oldBody, newBody] = bodies();
    assert.match(oldBody.closest('[data-host="Pressable"]').getAttribute('aria-label'), /A reply long enough/);
    assert.match(newBody.closest('[data-host="Pressable"]').getAttribute('aria-label'), /independently started second message/);
    assert.match(document.querySelector('[aria-label="Review, 1 message, open, started here"]').textContent, /Started here/);
    await step(() => row('Review, 1 message, open, started here').onPress());
    assert.deepEqual(bodies(), [oldBody], 'collapsing the accepted start never collapses the original');
    await step(() => row('Review, 1 message, open, started here').onPress());
    assert.equal(bodies().length, 2);
    const reopenedNew = bodies()[1];
    await step(() => row('Review, 1 message, open').onPress());
    assert.deepEqual(bodies(), [reopenedNew], 'collapsing the original never collapses the accepted start');
    assert.equal(row('Review, 1 message, open, started here').accessibilityState.expanded, true);
  } finally { await view.unmount(); }
});

for (const change of ['explicit-clear', 'remote-clear', 'reopen']) for (const ending of ['owner-reset', 'exact-denial', 'collection-denial']) {
  test(`${change} removes filter membership, not selected reading contexts; ${ending} still clears protected content`, async () => {
    const recentlyClosed = change === 'reopen';
    const thread = { id: 'thread/one', title: 'Review', message_count: 1, unread: true,
      state: recentlyClosed ? 'closed' : 'open', unread_reminder: recentlyClosed ? undefined : reminder };
    const harness = deliveryStore(thread);
    harness.member.outcome = 'confirmed';
    const view = await mountMarkedList(harness, thread, { recentlyClosed });
    const row = label => mounted().find(({ type, props }) => type === 'Pressable' && props.accessibilityLabel?.startsWith(label))?.props;
    const bodies = () => [...document.querySelectorAll('[data-collapsable="false"]')];
    try {
      await step(() => row(recentlyClosed ? 'Recently closed, 3' : 'Marked unread').onPress());
      for (const title of ['Review', 'Other one', 'Other two']) await step(() => row(title).onPress());
      const originalBodies = bodies();
      const originalComposers = [...document.querySelectorAll('[data-composer]')];
      assert.equal(originalBodies.length, 3);
      assert.equal(originalComposers.length, 3);
      const scroll = document.querySelector('[data-host="ScrollView"]');
      const order = () => [...scroll.querySelectorAll('[data-host="Pressable"][aria-label]')]
        .map(node => node.getAttribute('aria-label').split(',')[0]).filter(title => ['Review', 'Other one', 'Other two'].includes(title));
      const composer = originalComposers[0];
      composer.focus();
      scroll.scrollTop = 61;
      await step(() => {
        Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set.call(composer, 'An unfinished reply');
        composer.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
      });
      if (change === 'remote-clear') {
        const before = view.listReads();
        await view.remoteClear();
        assert.equal(view.listReads(), before, 'the exact summary stream alone updates marked membership, with no synthetic refresh');
      }
      else await press(recentlyClosed ? 'Reopen thread' : 'Clear reminder');
      assert.deepEqual(view.members().map(member => member.id), ['thread/two', 'thread/three'], 'authoritative filtered membership excludes the changed thread');
      assert.deepEqual(bodies(), originalBodies, 'all selected bodies retain their identity and order');
      assert.deepEqual([...document.querySelectorAll('[data-composer]')], originalComposers);
      assert.deepEqual(order(), ['Review', 'Other one', 'Other two']);
      assert.equal(document.querySelector('[data-host="ScrollView"]'), scroll);
      assert.equal(scroll.scrollTop, 61);
      assert.equal(document.activeElement, composer);
      assert.equal(composer.value, 'An unfinished reply');
      assert.equal(JSON.parse(harness.store.draftCache.current.get(`thread/${thread.id}`)).text, 'An unfinished reply');
      assert.equal(row('Review').accessibilityLabel.includes(', unread'), true, 'natural unread is unchanged by reminder clear or reopen');
      assert.equal(row('Review').accessibilityLabel.includes('marked unread reminder'), false);
      assert.deepEqual(harness.store.reads, [], 'explicit/remote clear never acknowledges unseen messages');
      if (recentlyClosed) {
        assert.ok(row('Recently closed, 2'), 'the count reports filter members, not retained open placements');
        assert.equal(row('Review').accessibilityLabel.includes('closed'), false);
      } else {
        assert.equal(row('Marked unread').accessibilityLabel, 'Marked unread', 'no phantom marked count is added');
        assert.equal(scroll.querySelector('[aria-label^="Review"]').textContent.includes('Marked unread'), false);
        assert.equal(harness.member.sends.length, change === 'explicit-clear' ? 1 : 0);
      }
      if (ending === 'owner-reset') {
        harness.store.draftGeneration.current++;
        await view.update();
      } else if (ending === 'exact-denial') await view.denySummary(thread.id);
      else await view.denyList();
      assert.equal(document.contains(originalBodies[0]), false);
      assert.equal(document.contains(composer), false);
      if (ending === 'exact-denial') {
        assert.ok(document.contains(originalBodies[1]), 'denial of one exact thread does not revoke another');
      } else assert.equal(bodies().length, 0);
    } finally { await view.unmount(); }
  });
}

for (const status of [401, 403, 404]) {
  test(`an authorized unchanged live membership supersedes an earlier HTTP ${status} refusal without reloading`, async () => {
    const thread = { id: 'thread/one', title: 'Review', state: 'closed', unread: false };
    const harness = deliveryStore(thread);
    const view = await mountMarkedList(harness, thread, { recentlyClosed: true, deniedStatus: status });
    const row = label => mounted().find(({ type, props }) => type === 'Pressable' && props.accessibilityLabel?.startsWith(label))?.props;
    try {
      await view.liveList();
      await step(() => row('Recently closed, 3').onPress());
      await step(() => row('Review').onPress());
      assert.ok(document.querySelector('[data-collapsable="false"]'));
      await view.denyList();
      assert.equal(row('Review'), undefined, 'a current HTTP read denial hides the collection');
      assert.equal(document.querySelector('[data-collapsable="false"]'), null);
      assert.ok(mounted().some(({ type }) => type === 'ThreadReadIssue'));
      const reads = view.contextReads();
      await view.liveList(); // Same IDs and hasMore, while HTTP still refuses.
      assert.equal(view.contextReads(), reads, 'unchanged live membership does not need another HTTP request');
      assert.ok(row('Recently closed, 3'), 'the authorized live collection is visible again');
      assert.ok(row('Review'));
      assert.equal(mounted().some(({ type }) => type === 'ThreadReadIssue'), false, 'the superseded refusal is no longer shown');
      assert.equal(document.querySelector('[data-collapsable="false"]'), null, 'revoked selections do not reopen themselves');
      await step(() => row('Review').onPress());
      assert.ok(document.querySelector('[data-collapsable="false"]'), 'the person can deliberately open the authorized thread again');
    } finally { await view.unmount(); }
  });
}
