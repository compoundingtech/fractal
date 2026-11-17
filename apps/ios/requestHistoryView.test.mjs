// The thread's request history view, executed from its production JSX without booting React
// Native. Fixtures follow the generated RequestHistoryPage DTO exactly; no field is synthesized.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { requestHistoryKey, requestHistoryRelations, requestHistoryRows } from './requestHistory.ts';
import { keepClosed } from '../../clients/typescript/fractal-views/homeView.ts';

const React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat() }) };
const source = readFileSync(new URL('./screens/RequestHistory.tsx', import.meta.url), 'utf8');
function compile(from, to, name, dependencies) {
  const a = source.indexOf(from);
  const b = to ? source.indexOf(to, a + from.length) : source.length;
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
const text = node => !node || typeof node !== 'object' ? String(node ?? '') : node.children.map(text).join('');
const texts = tree => findAll(tree, node => node.type === 'T').map(text);
const buttons = tree => findAll(tree, node => node.type === 'Button').map(node => node.props);

const UNAVAILABLE = compile('const UNAVAILABLE', '/** Retained requests', 'UNAVAILABLE', {});
const views = { View: 'View', Pressable: 'Pressable', T: 'T', Button: 'Button', Markdown: 'Markdown', Note: 'Note', theme: {} };
const opened = [], linked = [];
const ReferenceLine = compile('function ReferenceLine(', undefined, 'ReferenceLine', {
  ...views, openDocumentLink: reference => opened.push(reference), Linking: { openURL: async url => { linked.push(url); } },
});
const navigated = [];
const current = { id: 'attention/current', source_id: 'step-run/review', episode: 'episode/2026-11-01', state: 'open', title: 'Current wording of the review' };
const historyRow = attention => compile('function HistoryRow(', '/** Retained references', 'HistoryRow', {
  ...views, UNAVAILABLE, ReferenceLine: 'ReferenceLine',
  useStore: () => ({ data: { attention } }), useTextAppearance: () => ({ color: color => color }),
  useNavigation: () => ({ navigate: (...args) => navigated.push(args) }),
});
const HistoryRow = historyRow([current]);
const HistoryPage = compile('function HistoryPage(', 'function HistoryRow(', 'HistoryPage', {
  ...views, HistoryRow: 'HistoryRow', requestHistoryKey, requestHistoryRelations, requestHistoryRows,
});

/** @type {import('../../clients/typescript/fractal-client').RequestHistoryItem} */
const available = {
  key: { source_id: 'step-run/review', episode: 'episode/2026-11-01' },
  complete: true,
  state: { availability: 'available', status: 'answered', answers: [{ outcome: 'accept', id: 'approve' }] },
  payload: {
    availability: 'available', title: 'Review the loadUser rename', text: 'See [the plan](https://example.com/plan).',
    provenance: { kind: 'authored-episode', mission_revision: 'revision/one', opening_claim_id: 'claim/opening' },
    request: {}, references: [
      { target_id: 'doc/plan', revision: 'revision/retained', semantics: 'retained-reference' },
      { target_id: 'doc/runbook', revision: null, semantics: 'live-reference' },
      { target_id: 'pull/42', revision: null, semantics: 'live-reference' },
    ],
  },
};
/** @type {import('../../clients/typescript/fractal-client').RequestHistoryItem} */
const unavailable = {
  key: { source_id: 'step-run/review', episode: 'episode/2026-10-01' },
  complete: false,
  state: { availability: 'unavailable' },
  payload: { availability: 'unavailable', reason: 'historical-interpolation-unavailable' },
};

test('an available payload shows its immutable title, text, exact episode, status and answers', () => {
  const tree = HistoryRow({ item: available, boundary: false, follows: [] });
  const lines = texts(tree);
  assert.ok(lines.includes('Review the loadUser rename'));
  assert.equal(findAll(tree, node => node.type === 'Markdown')[0].props.text, available.payload.text, 'links stay in the rendered Markdown');
  assert.ok(lines.includes('episode episode/2026-11-01'));
  assert.ok(lines.includes('status answered'));
  assert.ok(lines.includes(`answer ${JSON.stringify(available.state.answers[0])}`), 'unknown answers are shown read-only as returned');
});

test('an available payload without a title says the original title is unavailable', () => {
  const untitled = { ...available, payload: { ...available.payload, title: null } };
  const lines = texts(HistoryRow({ item: untitled, boundary: false, follows: [] }));
  assert.ok(lines.includes('The original title is unavailable.'));
  assert.ok(!lines.some(line => line.includes(current.title)), 'the current Inbox wording never stands in');
});

test('an unavailable payload states its reason and never falls back to the current source text', () => {
  const tree = HistoryRow({ item: unavailable, boundary: true, follows: [] });
  const lines = texts(tree);
  assert.ok(lines.includes('The request wording as it was shown is unavailable.'));
  assert.ok(!lines.some(line => line.includes(current.title)), 'the current Inbox wording never stands in');
  assert.equal(findAll(tree, node => node.type === 'Markdown').length, 0);
  assert.ok(lines.includes('episode episode/2026-10-01 · boundary · incomplete'));
  assert.ok(lines.includes('Answer state unavailable.'));
  for (const reason of Object.keys(UNAVAILABLE)) assert.ok(UNAVAILABLE[reason].length);
});

test('source navigation is labelled as the current source, never as the historical episode', () => {
  navigated.length = 0;
  const open = buttons(HistoryRow({ item: available, boundary: false, follows: [] })).find(props => props.label === 'Open current source');
  assert.ok(open);
  assert.match(open.hint, /current Inbox item/);
  open.onPress();
  assert.deepEqual(navigated, [['Attention', { id: current.id }]]);
  const other = { ...available, key: { source_id: 'notice/gone', episode: 'episode/one' } };
  assert.equal(buttons(HistoryRow({ item: other, boundary: false, follows: [] })).length, 0, 'no route exists for a past episode');
});

test('closed episode caches never label the older E1 card current for historical E2', () => {
  const e1 = { ...current, id: 'attention/E1', episode: 'episode/E1' };
  const e2 = { ...current, id: 'attention/E2', episode: 'episode/E2' };
  const firstReplacement = keepClosed([e1], [e2], new Set());
  assert.deepEqual(firstReplacement.map(item => [item.episode, !!item.closedElsewhere]), [
    [e2.episode, false], [e1.episode, true],
  ]);
  const closed = keepClosed(firstReplacement, [], new Set());
  assert.deepEqual(closed.map(item => [item.episode, !!item.closedElsewhere]), [
    [e1.episode, true], [e2.episode, true],
  ]);
  const historical = episode => ({ ...available, key: { source_id: e1.source_id, episode } });
  for (const episode of [e1.episode, e2.episode]) {
    const row = historyRow(closed)({ item: historical(episode), boundary: false, follows: [] });
    assert.equal(buttons(row).length, 0, 'neither closed cache entry proves a current navigation target');
  }
  const active = historyRow(firstReplacement)({ item: historical(e2.episode), boundary: false, follows: [] });
  const open = buttons(active).find(props => props.label === 'Open current source');
  assert.ok(open);
  navigated.length = 0;
  open.onPress();
  assert.deepEqual(navigated, [['Attention', { id: e2.id }]]);
  assert.equal(buttons(historyRow(firstReplacement)({ item: historical(e1.episode), boundary: false, follows: [] })).length, 0,
    'a different active episode cannot stand in for the historical episode');
  assert.equal(buttons(historyRow([{ ...e2, state: 'resolved' }])({ item: historical(e2.episode), boundary: false, follows: [] })).length, 0);
});

test('every retained row offers Start on its exact key, with no current Inbox item or agent', () => {
  const starts = tree => findAll(tree, node => node.type === 'StartAction').map(node => node.props);
  // No attention at all: no current item proves navigation, yet Start still uses the retained key.
  const row = historyRow([])({ item: unavailable, boundary: true, follows: [], title: 'step-run/review · episode episode/2026-10-01', StartAction: 'StartAction' });
  assert.deepEqual(starts(row), [{ episode: unavailable.key, title: 'step-run/review · episode episode/2026-10-01' }]);
  assert.equal(starts(row)[0].episode, unavailable.key, 'the exact retained key object, never a current Inbox origin');
  assert.equal(buttons(row).length, 0, 'Start never stands in for current-source navigation');
  assert.ok(texts(row).includes('The request wording as it was shown is unavailable.'), 'the retained row stays read-only history');
  assert.deepEqual(starts(HistoryRow({ item: available, boundary: false, follows: [] })), [], 'no seam, no Start');
  // Navigation proof is unchanged beside Start: only the exact active episode opens a current source.
  const both = HistoryRow({ item: available, boundary: false, follows: [], title: available.payload.title, StartAction: 'StartAction' });
  assert.deepEqual(buttons(both).map(props => props.label), ['Open current source']);
  assert.deepEqual(starts(both), [{ episode: available.key, title: available.payload.title }]);
});

test('a page hands each row the Start seam and its retained title or exact key label', () => {
  /** @type {import('../../clients/typescript/fractal-client').RequestHistoryPage} */
  const page = {
    items: [available], boundary_items: [unavailable], complete: true, next_cursor: null, source_support: 'person-step',
    source_episodes: [], relations: [],
  };
  const rows = findAll(HistoryPage({ page, loadingMore: false, reload() {}, more() {}, StartAction: 'StartAction' }), node => node.type === 'HistoryRow');
  assert.deepEqual(rows.map(row => [row.props.title, row.props.StartAction]), [
    ['step-run/review · episode episode/2026-10-01', 'StartAction'], ['Review the loadUser rename', 'StartAction'],
  ]);
});

test('retained and live references are tagged; retained documents open as their current version', () => {
  opened.length = 0;
  const [retained, live, other] = available.payload.references.map(reference => ReferenceLine({ reference }));
  assert.deepEqual(texts(retained), ['doc/plan · retained reference · revision revision/retained']);
  assert.equal(buttons(retained)[0].label, 'Open current version');
  assert.deepEqual(texts(live), ['doc/runbook · live reference']);
  assert.equal(buttons(live)[0].label, 'Open document');
  assert.equal(buttons(other).length, 0);
  buttons(retained)[0].onPress();
  assert.deepEqual(opened, ['doc/plan']);
});

test('external references stay links; other targets never get an invented address', () => {
  linked.length = 0;
  const external = ReferenceLine({ reference: { target_id: 'https://example.com/plan', revision: null, semantics: 'live-reference' } });
  const [open] = buttons(external);
  assert.equal(open.label, 'Open link');
  open.onPress();
  assert.deepEqual(linked, ['https://example.com/plan']);
  assert.equal(buttons(ReferenceLine({ reference: { target_id: 'pull/42', revision: null, semantics: 'retained-reference' } })).length, 0);
});

test('a page keeps boundary rows, server grouping and only relations whose endpoints it holds', () => {
  const outside = { source_id: 'step-run/hidden', episode: 'episode/hidden' };
  /** @type {import('../../clients/typescript/fractal-client').RequestHistoryPage} */
  const page = {
    items: [available], boundary_items: [unavailable], complete: true, next_cursor: 'cursor/next', source_support: 'person-step',
    source_episodes: [{ source_id: 'step-run/review', episodes: [unavailable.key, available.key] }],
    relations: [
      { predecessor: unavailable.key, request: available.key, reason: 'reopened' },
      { predecessor: outside, request: available.key, reason: 'hidden predecessor' },
    ],
  };
  let more = 0;
  const tree = HistoryPage({ page, loadingMore: false, reload() {}, more: () => more++ });
  assert.ok(texts(tree).includes('step-run/review · 2 episodes'));
  const rows = findAll(tree, node => node.type === 'HistoryRow');
  assert.deepEqual(rows.map(row => row.props.item.key.episode), [unavailable.key.episode, available.key.episode]);
  assert.deepEqual(rows.map(row => row.props.boundary), [true, false]);
  assert.deepEqual(rows[1].props.follows, [{ predecessor: 'step-run/review · episode episode/2026-10-01', reason: 'reopened' }],
    'an unavailable predecessor is named by its exact key; an absent one is never drawn');
  buttons(tree).find(props => props.label === 'Next page').onPress();
  assert.equal(more, 1);
  const unsupported = HistoryPage({ page: { ...page, items: [], boundary_items: [], source_episodes: [], relations: [], next_cursor: null, source_support: 'unsupported-source', complete: false }, loadingMore: false, reload() {}, more() {} });
  assert.ok(findAll(unsupported, node => node.type === 'Note').length >= 2);
  assert.ok(texts(unsupported).includes('No earlier requests.'));
});

test('history stays collapsed until expanded, rereads on every reopen and closes on collapse or blur', () => {
  const loads = [];
  let closes = 0;
  class Reader {
    constructor(client, threadId) { this.threadId = threadId; }
    subscribe = () => () => {};
    getSnapshot = () => ({ kind: 'loading' });
    load = () => { loads.push(this.threadId); return Promise.resolve(); };
    more = () => Promise.resolve();
    close = () => { closes++; };
  }
  const values = [];
  let index = 0, blur;
  const RequestHistory = compile('export function RequestHistory(', 'function HistoryPage(', 'RequestHistory', {
    ...views, RequestHistoryReader: Reader, HistoryPage: 'HistoryPage',
    useMemo: compute => (values.memo ??= compute()), useSyncExternalStore: (_subscribe, snapshot) => snapshot(),
    useCallback: fn => fn, useFocusEffect: effect => { blur = effect(); },
    useState: initial => { const at = index++; if (!(at in values)) values[at] = initial; return [values[at], next => { values[at] = next; }]; },
  });
  const render = () => { index = 0; return RequestHistory({ client: {}, threadId: 'thread/one' }); };
  const header = tree => findAll(tree, node => node.type === 'Pressable')[0];
  let tree = render();
  assert.equal(text(header(tree)), '▸ Request history');
  assert.deepEqual(header(tree).props.accessibilityState, { expanded: false });
  assert.deepEqual(loads, [], 'nothing is read before an explicit expand');
  header(tree).props.onPress();
  tree = render();
  assert.deepEqual(loads, ['thread/one']);
  assert.ok(texts(tree).includes('History only · requests remain independent'));
  assert.ok(texts(tree).includes('Loading request history…'));

  header(tree).props.onPress();
  tree = render();
  assert.equal(closes, 1, 'collapsing releases the page');
  assert.deepEqual(header(tree).props.accessibilityState, { expanded: false });
  header(tree).props.onPress();
  tree = render();
  assert.deepEqual(loads, ['thread/one', 'thread/one'], 'reopening reads again instead of reusing a page');

  blur();
  tree = render();
  assert.equal(closes, 2, 'leaving the screen releases the page');
  assert.deepEqual(header(tree).props.accessibilityState, { expanded: false });
});
