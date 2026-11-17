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
  return React.createElement('div', { ref, 'data-host': type, 'data-collapsable': props.collapsable === false ? 'false' : undefined }, children);
};
const hosts = Object.fromEntries(['View', 'T', 'Text', 'Button', 'Field', 'Pressable', 'ContextMenu', 'Runs',
  'ThreadComposer', 'ThreadEditFlash', 'ThreadReadIssue'].map(type => [type, host(type)]));
const mounted = () => [...hostProps.values()];
const button = label => mounted().find(({ type, props }) => type === 'Button' && props.label === label)?.props;

const source = file => readFileSync(new URL(file, import.meta.url), 'utf8');
const compilerOptions = { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React };
function compile(file, name, next, dependencies) {
  const text = source(file);
  const a = text.search(new RegExp(`function ${name}(?=[(<])`));
  const b = next === null ? text.length : text.indexOf(`\n${next}`, a);
  assert.ok(a >= 0 && b > a, `${name} in ${file}`);
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
const navigation = { isFocused: () => true, addListener: () => () => {} };
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
    useFocusEffect: callback => React.useEffect(() => callback(), [callback]),
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
  const ledger = () => deliveriesByClient.current.get(member) ?? new Map();
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
async function mountMarkedList(harness, initial) {
  let summary = initial, listReads = 0, summaryHandlers;
  const StoreContext = React.createContext(harness.store);
  const useStore = () => React.useContext(StoreContext);
  const hooks = { useStore, useCallback: React.useCallback, useEffect: React.useEffect,
    useMemo: React.useMemo, useRef: React.useRef, useState: React.useState,
    useFocusEffect: callback => React.useEffect(() => callback(), [callback]) };
  const appearance = () => ({ system: true, palette: {}, colors: {}, color: value => value });
  const deps = { ...hooks, ...hosts, ClientError, THREAD_WINDOW_SIZE, reminderFromSummary, ACCESSIBILITY_FONT_SCALE: 1.3,
    errorText: error => String(error), theme: {}, useTextAppearance: appearance,
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
    threadDraftKey: id => `thread/${id}`, readThreadDraft: value => value ? JSON.parse(value) : { text: '' },
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
  Object.assign(harness.store, {
    threadClient: harness.member, draftCache: { current: new Map() }, draftsReady: true, saveDrafts() {},
    feed: { belongsTo: client => client === harness.member,
      followThreadSummary: (_id, handlers) => { summaryHandlers = handlers; return { close() {} }; } },
  });
  harness.store.threadActions.add('thread.reply');
  const originalUnread = harness.store.unreadClient;
  harness.store.unreadClient = { ...originalUnread,
    listMarked: async () => { listReads++; return reminderFromSummary(summary) ? { kind: 'items', items: [summary] } : { kind: 'empty' }; },
  };
  harness.member.threadGet = async () => ({ value: summary });
  harness.member.threadTimeline = async () => ({ value: { items: [{ ...message, thread_quote_id: null }], next_cursor: null, anchor: { state: 'current', message_id: message.id } } });
  const clear = harness.member.threadClearUnread;
  harness.member.threadClearUnread = async request => {
    const result = await clear(request);
    summary = { ...summary, unread_reminder: undefined };
    summaryHandlers?.onWindow({ items: [summary], snapshot: { host_id: 'host/one', store_index: 2 } });
    return result;
  };
  function Store() {
    const [, update] = React.useState(0);
    React.useEffect(() => { harness.onChange(() => update(value => value + 1)); return () => harness.onChange(() => {}); }, []);
    return React.createElement(StoreContext.Provider, { value: { ...harness.store } },
      React.createElement(ThreadGroup, { context: { marked_unread: true }, title: 'Marked unread', openThread() {} }));
  }
  const root = createRoot(document.getElementById('root'));
  await step(() => root.render(React.createElement(Store)));
  return { listReads: () => listReads, update: () => step(() => harness.setOnline(true)),
    unmount: () => step(() => root.unmount()) };
}

for (const ending of ['collapse', 'owner', 'generation', 'read-grant']) {
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
      const body = await layOut(100);
      const composer = document.querySelector('[data-composer]');
      assert.ok(composer, 'the production composer mounted');
      composer.focus();
      body.scrollTop = 29;
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
      assert.equal(body.scrollTop, 29);
      assert.equal(row('Review').accessibilityLabel.includes('marked unread reminder'), false);
      assert.equal(mounted().some(({ type, props }) => type === 'T' && props.children === 'Marked unread'), false);
      if (ending === 'collapse') await step(() => row('Review').onPress());
      else {
        if (ending === 'owner') harness.store.actor = 'person/ada';
        if (ending === 'generation') harness.store.draftGeneration.current++;
        if (ending === 'read-grant') harness.store.threadClient = undefined;
        await view.update();
      }
      assert.equal(document.querySelector('[data-collapsable="false"]'), null);
      assert.equal(document.querySelector('[data-composer]'), null);
    } finally { await view.unmount(); }
  });
}
