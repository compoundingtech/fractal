// The Start UI bound to the real ThreadStartContextReader: production JSX of InboxThreadStart,
// ThreadComposer and HistoryStart, executed without booting React Native. The client returns
// generated ThreadStartContext DTOs; the store's startThread records exactly what would be signed.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { ThreadStartContextReader } from './threadStartContext.ts';
import { appliedAnswer, ClientError, FractalClient, resendBoundAction } from '../../clients/typescript/fractal-client/index.ts';
import { sendStart } from './startDelivery.ts';
import { agentName } from './agentsView.ts';

const React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat() }) };
const source = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
function compile(name, next, dependencies) {
  const a = source.search(new RegExp(`function ${name}(?=[(<])`));
  const b = source.indexOf(`\n${next}`, a);
  assert.ok(a >= 0 && b > a, name);
  const { outputText } = ts.transpileModule(`${source.slice(a, b)}\nexports.value = ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  });
  const exports = {};
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exports, React, ...Object.values(dependencies));
  return exports.value;
}
const findAll = (node, match) => !node || typeof node !== 'object' ? [] : [
  ...(match(node) ? [node] : []), ...(node.children ?? []).flatMap(child => findAll(child, match)),
];
const find = (node, type) => findAll(node, candidate => candidate.type === type)[0];
const settle = () => new Promise(resolve => setImmediate(resolve));

/** One component instance: indexed state, memo and effects that rerun when their dependencies change. */
function instance() {
  const slots = [], cleanups = [];
  let index = 0;
  const changed = (previous, next) => !previous || next.some((value, at) => !Object.is(value, previous[at]));
  return {
    reset: () => { index = 0; },
    useState(initial) {
      const at = index++;
      if (!(at in slots)) slots[at] = typeof initial === 'function' ? initial() : initial;
      return [slots[at], next => { slots[at] = typeof next === 'function' ? next(slots[at]) : next; }];
    },
    useMemo(compute, deps) {
      const at = index++;
      if (!slots[at] || changed(slots[at].deps, deps)) slots[at] = { value: compute(), deps };
      return slots[at].value;
    },
    useEffect(effect, deps) {
      const at = index++;
      if (deps && slots[at] && !changed(slots[at], deps)) return;
      slots[at] = deps;
      cleanups[at]?.();
      cleanups[at] = effect() ?? undefined;
    },
    useRef(current) {
      const at = index++;
      return slots[at] ??= { current };
    },
    unmount: () => { for (const cleanup of cleanups) cleanup?.(); },
  };
}

const origin = { sourceId: 'notice/typed-system', episode: 'episode/exact' };
const scope = value => JSON.stringify([value.sourceId, value.episode]);
const attemptScope = (value, thread) => JSON.stringify([value.sourceId, value.episode, thread]);
const cacheKey = `thread-draft/${scope(origin)}`;
/** @type {(agent_id: string, basis?: import('../../clients/typescript/fractal-client').ThreadStartRecipient['basis']) => import('../../clients/typescript/fractal-client').ThreadStartContext} */
const context = (agent_id, basis = { _tag: 'configured' }) => ({
  kind: 'thread-start-context', source_id: origin.sourceId, episode: origin.episode, start_recipient: { agent_id, basis },
});
const fallback = (agent_id, original_agent_id) => context(agent_id, {
  _tag: 'fallback', original_agent_id, retirement: { claim_id: `claim/${original_agent_id}`, kind: 'agent.retired', state: 'retired' },
});

// Bind real serialized refusals through the production store's completion mapping.
const storeSource = ts.createSourceFile('store.tsx', readFileSync(new URL('./store.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let finishNode;
const visitFinish = node => {
  if (ts.isVariableDeclaration(node) && node.name.getText(storeSource) === 'finish' && node.initializer?.getText(storeSource).includes('refreshStartContext')) finishNode = node;
  ts.forEachChild(node, visitFinish);
};
visitFinish(storeSource);
assert.ok(finishNode);
const finishCode = ts.transpileModule(`const kind = 'start'; const ${finishNode.getText(storeSource)}; exports.finish = finish;`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
const finishExports = {};
new Function('exports', 'ClientError', 'acceptedSnapshot', 'threadActionErrorText', 'setThreadRefresh', 'appliedAnswer', 'resendBoundAction', finishCode)(
  finishExports, ClientError, undefined, String, () => {}, appliedAnswer, resendBoundAction);
const goldenRefusals = JSON.parse(readFileSync(new URL('../../docs/st3/client-v0/fixtures/thread-start-refusals.json', import.meta.url), 'utf8'));
const wireRefusal = key => new ClientError(goldenRefusals[key], 409);

/** A Start panel and its composer over one store, one generated client and one draft cache. */
function harness({ contexts, outcomes = [], status = 'online', ledger = new Map(), draft = 'Hello' }) {
  const reads = [], signed = [];
  const client = {
    async threadStartContext(sourceID, episode, options) {
      reads.push([sourceID, episode, !!options?.signal]);
      const next = contexts.shift();
      if (next instanceof Error) throw next;
      return { value: next };
    },
  };
  const draftCache = { current: new Map(draft ? [[cacheKey, JSON.stringify({ text: draft })]] : []) };
  let uuid = 0;
  const store = {
    status, threadRefresh: 0, threadDeliveries: ledger, draftCache, draftsReady: true, saveDrafts() {},
    draftsOwner: { current: 'person/one' }, draftGeneration: { current: 0 },
    actions: { startThread: async (...args) => {
      signed.push(args);
      const outcome = outcomes.shift() ?? { how: 'sent' };
      if (!(outcome instanceof ClientError)) return outcome;
      return finishExports.finish(await sendStart('fixture', {
        build: async () => ({ fence: {}, target: args[0], origin: args[1], title: args[2], content: args[3] }),
        fence: async () => ({}), send: async () => { throw outcome; }, wait: async () => {},
      }));
    } },
  };
  const panelHooks = instance(), composerHooks = instance();
  const Panel = compile('InboxThreadStart', 'function InlineThread', {
    ...panelHooks, useStore: () => store, useSyncExternalStore: (_subscribe, snapshot) => snapshot(), ThreadStartContextReader,
    useCallback: (callback, deps) => panelHooks.useMemo(() => callback, deps),
    useFocusEffect: callback => panelHooks.useEffect(callback, [callback]),
    threadStartScope: scope, threadDraftKey: key => `thread-draft/${key}`, threadStartSlot: thread => `start/${thread}`,
    threadStartAttemptScope: attemptScope, threadTitleFromAsk: value => value, startRecoveryKey: thread => `start-recovery/${thread}`,
    View: 'View', T: 'T', Button: 'Button', ThreadGroup: 'ThreadGroup', InlineThread: 'InlineThread',
    StartRecipientPreview: 'StartRecipientPreview', ThreadReadIssue: 'ThreadReadIssue', theme: {},
  });
  const Composer = compile('ThreadComposer', 'function ThreadComposeField', {
    ...composerHooks, useStore: () => store, useFocusEffect() {}, useCallback: fn => fn,
    threadDraftKey: key => key, threadStartAttemptScope: attemptScope, readThreadDraft: raw => raw ? JSON.parse(raw) : { text: '' },
    threadTextIssue: () => undefined, AccessibilityInfo: { announceForAccessibility() {} }, Clipboard: { setStringAsync: async () => {} },
    Crypto: { randomUUID: () => `uuid-${++uuid}` }, theme: {},
    View: 'View', T: 'T', Button: 'Button', ThreadComposeField: 'ThreadComposeField', ThreadActionFeedback: 'ThreadActionFeedback',
  });
  const props = { client, origin, initialTitle: 'Typed notice', canStart: true, onOpen() {} };
  const panel = (extra = {}) => { panelHooks.reset(); return Panel({ ...props, ...extra }); };
  const composer = start => { composerHooks.reset(); return Composer({ start, canReply: true, selectionVersion: 0 }); };
  return { reads, signed, panel, composer, store, unmount: panelHooks.unmount };
}
const startOf = tree => find(tree, 'InlineThread')?.props.start;
const field = tree => find(tree, 'ThreadComposeField').props;

test('a typed notice with no Inbox agent or roster entry shows and signs exactly the StartContext recipient', async () => {
  const h = harness({ contexts: [context('agent/unlisted-system-recipient')] });
  let tree = h.panel();
  assert.ok(findAll(tree, node => node.type === 'T' && node.children.join('') === 'Reading who this Start reaches…').length);
  assert.equal(startOf(tree).origin, undefined, 'no recipient is assumed before the read');
  assert.equal(field(h.composer(startOf(tree))).enabled, false, 'the composer waits for the recipient');
  await settle();
  tree = h.panel();
  assert.deepEqual(h.reads, [[origin.sourceId, origin.episode, true]], 'the exact source episode is read');
  const preview = find(tree, 'StartRecipientPreview').props;
  assert.deepEqual(preview, { recipient: { agent_id: 'agent/unlisted-system-recipient', basis: { _tag: 'configured' } }, changed: false });
  const start = startOf(tree);
  assert.deepEqual(start.origin, { ...origin, agentId: 'agent/unlisted-system-recipient' });
  assert.equal(start.onRecipientRefused !== undefined, true);
  const ready = field(h.composer(start));
  assert.equal(ready.enabled, true);
  ready.onSend();
  await settle();
  assert.deepEqual(h.signed, [['thread/uuid-1', { ...origin, agentId: 'agent/unlisted-system-recipient' }, 'Typed notice', 'Hello']]);
});

/** Render the production preview against existing roster metadata; no recipient selection here. */
function visiblePreview(h, panel) {
  const previewSource = readFileSync(new URL('./screens/StartRecipientPreview.tsx', import.meta.url), 'utf8');
  const start = previewSource.indexOf('export function StartRecipientPreview(');
  const { outputText } = ts.transpileModule(`${previewSource.slice(start)}\nexports.preview = StartRecipientPreview;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  });
  const exports = {};
  new Function('exports', 'React', 'View', 'T', 'theme', 'agentName', 'useStore', outputText)(
    exports, React, 'View', 'T', {}, agentName, () => h.store);
  return exports.preview(find(panel, 'StartRecipientPreview').props);
}

/** Production startThread signing fields and generated HTTP serialization over test ports. */
function signedStartTransport(h) {
  const signed = [], posts = [];
  // Same canonical capabilities response used by fractal-client/contract.test.cjs.
  const capabilities = JSON.parse(readFileSync(new URL('../../docs/st3/client-v0/fixtures/capabilities.json', import.meta.url), 'utf8'));
  const client = new FractalClient({ baseUrl: 'https://fixture.example', fetchImpl: async (url, init) => {
    if (String(url).endsWith('/capabilities')) {
      return new Response(JSON.stringify(capabilities), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }
    const body = JSON.parse(init.body);
    posts.push({ url: String(url), method: init.method, body });
    /** @type {import('../../clients/typescript/fractal-client').EnvelopeOf<import('../../clients/typescript/fractal-client').ActionResult>} */
    const receipt = {
      api_version: 'fractal.client.v0', request_id: 'request/unit',
      snapshot: { id: 'snapshot/unit', host_id: 'host/unit', created_at: '2026-11-17T00:00:00Z', projection_version: 'client-projection.v0', store_index: 1 },
      value: { kind: 'action-result', action_id: body.id, affected_ids: [body.parameters.target_id, 'message/first'], operation_id: 'operation/unit', snapshot_id: 'snapshot/unit', status: 'completed' },
    };
    return new Response(JSON.stringify(receipt), { status: 200, headers: { 'Content-Type': 'application/json' } });
  } });
  const text = storeSource.text;
  const a = text.indexOf('    startThread(thread: string,');
  const b = text.indexOf('    replyToThread(', a);
  const { outputText } = ts.transpileModule(`exports.action = { ${text.slice(a, b)} }.startThread;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const ports = {
    signingKeyRef: { current: 'profile/exact' }, threadStartAttemptScope: attemptScope,
    trackThreadDelivery: (_scope, _target, send) => send(),
    readPhoneSigning: async () => ({ person: 'person/avery' }),
    signThreadMessage: async (_key, _signing, fields) => { signed.push(fields); return { signature: 'exact-signature', messageId: 'message/first' }; },
    submitThreadAction: async (_target, _kind, _revisions, prepare) => {
      const prepared = await prepare(client, 'action/key');
      await prepared.send({ id: 'action/key', idempotency_key: 'action/key', fence: { snapshot_id: 'snapshot/unit' } });
      return { how: 'sent' };
    },
  };
  const exports = {};
  new Function('exports', ...Object.keys(ports), outputText)(exports, ...Object.values(ports));
  h.store.actions.startThread = exports.action;
  return { signed, posts };
}

test('offline or stopped original stays visibly named and signed despite a plausible upstream; only canonical retired fallback changes the recipient', async () => {
  for (const state of ['down', 'stopped', 'retired']) {
    const originalId = 'agent/original-reviewer', fallbackId = 'agent/dotfiles/manager';
    const selected = state === 'retired' ? fallback(fallbackId, originalId) : context(originalId, { _tag: 'original' });
    const h = harness({ contexts: [selected] });
    /** @type {import('../../clients/typescript/fractal-client').Agent[]} */
    const agents = [
      { id: originalId, name: 'original-reviewer', kind: 'agent', revision: 'roster/one', updated_at: '2026-11-17T00:00:00Z', state, reachability: 'unreachable', harness_state: 'stopped', runtime_ids: [] },
      { id: fallbackId, name: 'Chief of Staff', kind: 'agent', revision: 'roster/two', updated_at: '2026-11-17T00:00:00Z', state: 'running', reachability: 'reachable', harness_state: 'idle', runtime_ids: [] },
    ];
    h.store.data = { agents };
    const transport = signedStartTransport(h);
    h.panel();
    await settle();
    const panel = h.panel();
    const preview = visiblePreview(h, panel);
    const heading = find(preview, 'T').children.join('');
    assert.equal(heading, state === 'retired' ? 'New discussion with Chief of Staff' : 'To Original Reviewer');
    assert.equal(transport.signed.length, 0, 'the visible recipient precedes Send');
    assert.equal(transport.posts.length, 0);
    field(h.composer(startOf(panel))).onSend();
    await settle();
    const expected = state === 'retired' ? fallbackId : originalId;
    assert.equal(transport.signed.length, 1);
    assert.equal(transport.signed[0].to, expected);
    assert.equal(transport.posts.length, 1);
    assert.equal(transport.posts[0].method, 'POST');
    assert.equal(transport.posts[0].body.type, 'thread.start');
    assert.equal(transport.posts[0].body.parameters.agent_id, expected);
    assert.equal(transport.posts[0].body.parameters.signature, 'exact-signature');
  }
});

test('fallback agentName is display-only: Chief of Staff is shown while signing and POST use its canonical agent_id', async () => {
  const id = 'agent/dotfiles/manager';
  const h = harness({ contexts: [fallback(id, 'agent/retired-reviewer')] });
  /** @type {import('../../clients/typescript/fractal-client').Agent[]} */
  const agents = [{ id, name: 'Chief of Staff', kind: 'agent', revision: 'roster/one', updated_at: '2026-11-17T00:00:00Z', state: 'running', reachability: 'reachable', harness_state: 'idle', runtime_ids: [] }];
  h.store.data = { agents };
  const transport = signedStartTransport(h);
  h.panel();
  await settle();
  const panel = h.panel();
  const preview = visiblePreview(h, panel);
  assert.equal(find(preview, 'T').children.join(''), 'New discussion with Chief of Staff');
  assert.match(preview.props.accessibilityLabel, /Chief of Staff \(agent\/dotfiles\/manager\)/);
  assert.equal(transport.signed.length, 0);
  assert.equal(transport.posts.length, 0);
  field(h.composer(startOf(panel))).onSend();
  await settle();
  assert.equal(transport.signed[0].to, id);
  assert.equal(transport.posts[0].method, 'POST');
  assert.equal(transport.posts[0].body.parameters.agent_id, id);
});

test('a retired-origin fallback shows the original source agent separately and signs only the fallback', async () => {
  const h = harness({ contexts: [fallback('agent/chief-of-staff', 'agent/retired-reviewer')] });
  h.panel();
  await settle();
  const tree = h.panel();
  assert.equal(find(tree, 'StartRecipientPreview').props.recipient.basis.original_agent_id, 'agent/retired-reviewer');
  field(h.composer(startOf(tree))).onSend();
  await settle();
  assert.equal(h.signed[0][1].agentId, 'agent/chief-of-staff');
});

test('a failed read never implies a recipient: it shows the error and retries the read only', async () => {
  const h = harness({ contexts: [new Error('offline'), context('agent/original', { _tag: 'original' })] });
  h.panel();
  await settle();
  let tree = h.panel();
  assert.ok(find(tree, 'ThreadReadIssue'));
  assert.equal(find(tree, 'StartRecipientPreview'), undefined);
  assert.equal(startOf(tree).origin, undefined, 'offline never falls back to any agent');
  assert.equal(field(h.composer(startOf(tree))).enabled, false);
  const retry = findAll(tree, node => node.type === 'Button' && node.props.label === 'Retry')[0];
  retry.props.onPress();
  await settle();
  tree = h.panel();
  assert.deepEqual(find(tree, 'StartRecipientPreview').props, { recipient: { agent_id: 'agent/original', basis: { _tag: 'original' } }, changed: false });
  assert.equal(h.reads.length, 2);
  assert.deepEqual(h.signed, [], 'reading never sends');
});

test('an unconfigured source with no original agent visibly refuses Start and never invents a recipient', async () => {
  const refusal = wireRefusal('recipient-unassigned');
  const h = harness({ contexts: [refusal] });
  h.panel();
  await settle();
  const tree = h.panel();
  assert.match(find(tree, 'ThreadReadIssue').props.message, /No Start recipient is configured/);
  assert.equal(find(tree, 'StartRecipientPreview'), undefined);
  assert.equal(startOf(tree).origin, undefined);
  const composer = h.composer(startOf(tree));
  assert.equal(field(composer).enabled, false);
  field(composer).onSend();
  await settle();
  assert.deepEqual(h.signed, [], 'neither an Inbox agent nor a guessed Chief of Staff authorizes a send');
  assert.equal(h.reads.length, 1);
  assert.equal(findAll(tree, node => node.type === 'Button' && node.props.label === 'Retry').length, 0);
  assert.equal(h.store.draftCache.current.get(cacheKey), JSON.stringify({ text: 'Hello' }));
});

test('golden definitive POST recipient-unassigned invalidates the old preview and blocks setup without Retry or further sends', async () => {
  const h = harness({
    contexts: [context('agent/A'), wireRefusal('recipient-unassigned')],
    outcomes: [wireRefusal('recipient-unassigned')],
  });
  h.panel();
  await settle();
  field(h.composer(startOf(h.panel()))).onSend();
  await settle();
  const tree = h.panel();
  assert.equal(h.reads.length, 2, 'one visible canonical reread after definitive failure');
  assert.equal(find(tree, 'StartRecipientPreview'), undefined);
  assert.match(find(tree, 'ThreadReadIssue').props.message, /No Start recipient is configured/);
  assert.equal(findAll(tree, node => node.type === 'Button' && node.props.label === 'Retry').length, 0);
  assert.equal(startOf(tree).origin, undefined);
  const composer = h.composer(startOf(tree));
  assert.equal(field(composer).enabled, false);
  field(composer).onSend();
  await settle();
  h.panel();
  assert.equal(h.reads.length, 2, 'the blocked state has no automatic reread loop');
  assert.equal(h.signed.length, 1, 'zero further sends after the refused POST');
  assert.equal(h.store.draftCache.current.get(cacheKey), JSON.stringify({ text: 'Hello' }));
});

for (const key of ['stale-snapshot']) {
  test(`POST ${key} without applied proof retains the unknown packet and never refreshes or retargets`, async () => {
    const h = harness({ contexts: [context('agent/A')], outcomes: [wireRefusal(key)] });
    h.panel();
    await settle();
    field(h.composer(startOf(h.panel()))).onSend();
    await settle();
    const tree = h.panel();
    assert.equal(h.reads.length, 1, 'an unknown POST cannot release its signed recipient');
    assert.equal(h.signed.length, 1);
    assert.equal(h.signed[0][1].agentId, 'agent/A');
    assert.equal(h.store.draftCache.current.get('thread-draft/start/thread/uuid-1'), JSON.stringify({ text: 'Hello' }));
    assert.equal(find(tree, 'ThreadReadIssue'), undefined);
  });
}

test('GET recipient-retired is a distinct terminal refusal with no Retry or send', async () => {
  const h = harness({ contexts: [wireRefusal('recipient-retired')] });
  h.panel();
  await settle();
  const tree = h.panel();
  assert.match(find(tree, 'ThreadReadIssue').props.message, /recipient is retired/);
  assert.equal(findAll(tree, node => node.type === 'Button' && node.props.label === 'Retry').length, 0);
  assert.equal(startOf(tree).origin, undefined);
  assert.equal(field(h.composer(startOf(tree))).enabled, false);
  assert.deepEqual(h.signed, []);
});

test('409 recipient-retired independently refreshes to canonical B and requires a fresh explicit Send', async () => {
  const h = harness({ contexts: [context('agent/A'), fallback('agent/B', 'agent/A')], outcomes: [wireRefusal('recipient-retired'), { how: 'sent' }] });
  h.panel();
  await settle();
  field(h.composer(startOf(h.panel()))).onSend();
  await settle();
  const tree = h.panel();
  assert.equal(find(tree, 'StartRecipientPreview').props.recipient.agent_id, 'agent/B');
  assert.equal(find(tree, 'StartRecipientPreview').props.changed, true);
  assert.equal(h.signed.length, 1, 'a retired refusal never auto-resends');
  field(h.composer(startOf(tree))).onSend();
  await settle();
  assert.deepEqual(h.signed.map(([target, selected]) => [target, selected.agentId]), [
    ['thread/uuid-1', 'agent/A'], ['thread/uuid-2', 'agent/B'],
  ]);
});

test('A→B and B→C stale-recipient refusals reread and show the new recipient; only a fresh user send reaches it', async () => {
  const refused = { how: 'failed', error: 'The recipient changed.', refreshStartContext: true };
  const h = harness({
    contexts: [context('agent/A'), fallback('agent/B', 'agent/A'), fallback('agent/C', 'agent/B')],
    outcomes: [refused, refused, { how: 'sent' }],
  });
  h.panel();
  await settle();
  let start = startOf(h.panel());
  const sends = [];
  for (const [from, to] of [['agent/A', 'agent/B'], ['agent/B', 'agent/C']]) {
    assert.equal(start.origin.agentId, from);
    field(h.composer(start)).onSend();
    await Promise.resolve();
    // The refusal rereads at once: the old recipient is cleared and the composer disabled.
    let tree = h.panel();
    if (find(tree, 'StartRecipientPreview') === undefined) {
      assert.equal(startOf(tree).origin, undefined, 'the old recipient is gone while rereading');
      assert.equal(field(h.composer(startOf(tree))).enabled, false);
    }
    await settle();
    tree = h.panel();
    sends.push(h.signed.length);
    const preview = find(tree, 'StartRecipientPreview').props;
    assert.equal(preview.recipient.agent_id, to);
    assert.equal(preview.changed, true, 'the new recipient is shown as refreshed');
    start = startOf(tree);
    assert.equal(start.origin.agentId, to);
    assert.equal(start.target, undefined, 'no thread ID carries over from the refused send');
    const composed = h.composer(start);
    assert.equal(field(composed).enabled, true);
    assert.equal(find(composed, 'ThreadActionFeedback').props.result, refused, 'the refusal stays visible');
  }
  assert.deepEqual(sends, [1, 2], 'nothing is sent again until the person sends');
  field(h.composer(start)).onSend();
  await settle();
  assert.deepEqual(h.signed.map(([target, signed]) => [target, signed.agentId]), [
    ['thread/uuid-1', 'agent/A'], ['thread/uuid-2', 'agent/B'], ['thread/uuid-3', 'agent/C'],
  ], 'each explicit send is a new Start with its own ID; no signed request is retargeted');
  assert.equal(h.reads.length, 3);
});

test('a handed-off attempt keeps its original agent, never reads StartContext and is never retargeted', async () => {
  const pending = { target: 'thread/original', agentId: 'agent/original' };
  const ledger = new Map([[attemptScope(origin, pending.target), { target: pending.target, start: { origin: { ...origin, agentId: pending.agentId }, title: 'Original' } }]]);
  const h = harness({ contexts: [context('agent/new')], ledger, draft: '' });
  const tree = h.panel({ pending, canStart: false, showHistory: false });
  await settle();
  assert.deepEqual(h.reads, [], 'a pending receipt never rereads its recipient');
  assert.equal(find(tree, 'StartRecipientPreview'), undefined);
  const start = startOf(tree);
  assert.deepEqual(start.origin, { ...origin, agentId: 'agent/original' });
  assert.equal(start.target, 'thread/original');
  assert.equal(start.onRecipientRefused, undefined);
});

test('a handed-off attempt refused for a stale recipient offers Copy, never a resend on its old ID', async () => {
  const refused = { how: 'failed', error: 'The recipient changed.', refreshStartContext: true };
  const h = harness({ contexts: [], outcomes: [refused], ledger: new Map(), draft: '' });
  const start = { target: 'thread/original', cacheKey: 'start/thread/original', accepted: false, title: 'Original',
    origin: { ...origin, agentId: 'agent/original' }, onAccepted() {}, onCancel() {}, onSubmitted() {} };
  h.store.draftCache.current.set('start/thread/original', JSON.stringify({ text: 'Original content' }));
  field(h.composer(start)).onSend();
  await settle();
  const tree = h.composer(start);
  assert.equal(field(tree).canSend, false, 'the old ID is never sent to anyone again');
  assert.ok(findAll(tree, node => node.type === 'Button' && node.props.label === 'Copy message').length);
  assert.deepEqual(h.signed.map(([target, signed]) => [target, signed.agentId]), [['thread/original', 'agent/original']]);
  assert.deepEqual(h.reads, []);
});

test('a retained history row Starts on its exact key without an Inbox item, only with the thread.start grant', () => {
  const navigated = [];
  const render = actions => compile('HistoryStart', 'function RestoredThreadStart', {
    useStore: () => ({ threadClient: { id: 'client' }, threadActions: new Set(actions) }),
    useNavigation: () => ({ navigate: (...args) => navigated.push(args) }),
    threadStartScope: scope, InboxThreadStart: 'InboxThreadStart',
  })({ episode: { source_id: 'step-run/retained', episode: 'episode/E1' }, title: 'Earlier request' });
  const start = render(['thread.start']);
  assert.equal(start.type, 'InboxThreadStart');
  assert.deepEqual(start.props.origin, { sourceId: 'step-run/retained', episode: 'episode/E1' });
  assert.equal(start.props.canStart, true);
  assert.equal(start.props.showHistory, false);
  assert.equal(start.props.pending, undefined, 'a fresh Start; its recipient comes from StartContext');
  assert.equal('agentId' in start.props, false);
  start.props.onOpen('thread/new', 'Earlier request');
  assert.deepEqual(navigated, [['Thread', { id: 'thread/new', title: 'Earlier request' }]]);
  assert.equal(render([]), null);
});
