import assert from 'node:assert/strict';
import { detailRow, homeRows, homeSections, keepClosed, onHome } from '@fractal/views/homeView';

const now = Date.parse('2026-09-30T12:00:00Z');
// What Fractal presents for each kind (docs/clients/ui-contract.md, Attention row): Home reads it and
// derives nothing, so these rows carry it as Fractal serves them.
const presented = (attention_kind, extra) => {
  const card = {
    'human-gate': { tier: 'stopped', kind: extra.review_mode === 'feedback' ? 'feedback' : 'review' },
    'person-step': extra.update ? { tier: 'later', kind: 'update' } : { tier: 'stopped', kind: 'request' },
    'agent-request': { tier: 'stopped', kind: 'request' },
    'launch-approval': { tier: 'today', kind: 'launch' },
    'revision-approval': { tier: 'today', kind: 'revision' },
    'harness-prompt': { tier: 'stopped', kind: 'prompt' },
    'harness-login': { tier: 'stopped', kind: 'login' },
    fault: { tier: null, kind: 'fault' },
  }[attention_kind] ?? { tier: null, kind: null };
  const step = extra.step_run_id;
  return { ...card, agent: null, raised_by: null, waiting: step ? { step, path: step.split('/').pop() } : null };
};
const item = (id, attention_kind, extra = {}) => ({ id: `attention/${id}`, kind: 'attention', attention_kind, title: id, detail: '', priority: 'normal', state: 'open', person_id: 'person/alex', requested_at: '2026-09-30T11:00:00Z', source_id: 'x', actions: [], presentation: presented(attention_kind, extra), ...extra });

// Home shows what the Fractal terminal UI's Home shows: the person's unresolved requests and reviews, items without
// app actions included. A message, a fault and a kind Fractal adds later never reach Home.
const items = [
  item('note', 'unread-message'),
  item('fault', 'fault'),
  item('gate', 'human-gate', { actions: ['review.approve'], step_run_id: 'step-run/abc/build' }),
  item('launch', 'launch-approval'),
  item('broke', 'fault', { priority: 'critical' }),
  item('ask', 'agent-request'),
  item('step', 'person-step', { step_run_id: 'step-run/abc/approve-copy' }),
  item('done', 'human-gate', { state: 'resolved' }),
  item('theirs', 'human-gate', { person_id: 'person/someone-else' }),
  item('new-kind', 'something-new'),
];
const rows = homeRows(items, 'person/alex', now);
assert.deepEqual(rows.map(row => row.item.id.replace('attention/', '')), ['gate', 'ask', 'step', 'launch']);
assert.deepEqual(homeSections(rows).map(section => [section.title, section.count]), [['somebody is stopped on you', 3], ['today', 1]]);
assert.deepEqual([rows[0].kind, rows[0].glyph, rows[0].waiting, rows[0].age], ['review', '◆', 'step build', '1h']);
assert.deepEqual([rows[2].kind, rows[2].waiting], ['request', 'step approve-copy']);

// Before the connection names its person, nothing is hidden for belonging to someone else.
assert.equal(onHome(item('theirs', 'human-gate', { person_id: 'person/other' }), undefined), true);
assert.equal(onHome(item('done', 'human-gate', { state: 'resolved' }), undefined), false);

// A paired phone acts as person/NAME/session/ID: the person's items are still its own.
assert.equal(onHome(item('mine', 'review', { person_id: 'person/alex' }), 'person/alex/session/0190abcd'), true);
assert.equal(onHome(item('theirs', 'review', { person_id: 'person/robin' }), 'person/alex/session/0190abcd'), false);

// An update brings what the person asked for: it sits under "when there is time", after anything
// that waits on them, and asks nothing.
const withUpdate = homeRows([
  item('update', 'person-step', { update: { version: 1, type: 'update', about: 'message/0123456789abcdef' } }),
  item('ask', 'person-step'),
], 'person/alex', now);
assert.deepEqual(withUpdate.map(row => [row.item.id.replace('attention/', ''), row.kind, row.glyph]), [['ask', 'request', '◆'], ['update', 'update', '✦']]);
assert.deepEqual(homeSections(withUpdate).map(section => section.title), ['somebody is stopped on you', 'when there is time']);

// Renderers choose actual colors; the shared model describes their meaning.
assert.deepEqual(withUpdate.map(row => row.color), ['person', 'green']);

// Nothing leaves Home by itself: an item Fractal closes (or resolves) while it is shown stays, marked.
{
  const open = id => ({ id, state: 'open', attention_kind: 'person-step', actions: ['work.done'], title: id, requested_at: '2026-10-07T07:00:00Z', update: { summary: 'x', about: 'y' }, presentation: presented('person-step', { update: true }) });
  const first = [open('attention/a'), open('attention/b'), open('attention/c')];
  // a is gone, b is resolved, c was answered here.
  const next = [{ ...open('attention/b'), state: 'resolved' }];
  const kept = keepClosed(first, next, new Set(['attention/c']));
  assert.deepEqual(kept.map(item => item.id).sort(), ['attention/a', 'attention/b']);
  assert.ok(kept.every(item => item.closedElsewhere && item.actions.length === 0));
  const rows = homeRows(kept, undefined, Date.parse('2026-10-07T07:05:00Z'));
  assert.equal(rows.length, 2, 'a resolved copy is replaced by the open one that was shown');
  assert.match(rows[0].waiting, /closed; stays/);
  const sections = homeSections(homeRows([open('attention/open'), ...kept], undefined, Date.parse('2026-10-07T07:05:00Z')));
  assert.deepEqual(sections.map(section => [section.title, section.count]), [['when there is time', 1], ['Recently closed: clear each', 2]], 'closed items have a section of their own');
  // Again, with the marked ones as the previous list: they stay.
  assert.equal(keepClosed(kept, next, new Set()).length, 2);
  // An item that was never open on Home is not invented.
  assert.deepEqual(keepClosed([{ ...open('attention/z'), state: 'resolved' }], [], new Set()), []);
  // Only the latest five stay listed; the oldest drops first.
  const all = [0, 1, 2, 3, 4, 5, 6, 7].map(n => open(`attention/${n}`));
  let list = all;
  for (let n = 0; n < 7; n++) list = keepClosed(list, all.slice(n + 1), new Set());
  assert.deepEqual(list.filter(item => item.closedElsewhere).map(item => item.id).sort(), ['attention/2', 'attention/3', 'attention/4', 'attention/5', 'attention/6']);
  // An item that comes back open loses its mark.
  assert.equal(keepClosed(kept, [open('attention/a')], new Set()).find(item => item.id === 'attention/a').closedElsewhere, undefined);
}

// Fractal places each item and names its step: a gate Fractal puts in Today sits in Today, and the step is
// named by its path, not by its ID.
{
  const gate = item('gate', 'human-gate', { alert: true, presentation: { tier: 'today', kind: 'feedback', agent: null, raised_by: null, waiting: { step: 'step-run/abc/1/review', path: 'review' } } });
  const [row] = homeRows([gate], 'person/alex', now);
  assert.deepEqual([row.tier, row.kind, row.waiting], ['today', 'review', 'step review']);
  // Only a null group keeps it off Home; a group Home has no section for (alert, or one a newer Fractal
  // adds) shows where Fractal's alert flag puts it, never dropped.
  assert.deepEqual(homeRows([{ ...gate, presentation: { ...gate.presentation, tier: null } }], 'person/alex', now), []);
  assert.deepEqual(homeRows([{ ...gate, presentation: { ...gate.presentation, tier: 'alert' } }], 'person/alex', now).map(row => [row.tier, row.kind]), [['stopped', 'review']]);
  // A card this app does not know is unplaced, with Fractal's text, never dropped.
  const [unknown] = homeRows([{ ...gate, presentation: { ...gate.presentation, kind: 'poll' } }], 'person/alex', now);
  assert.deepEqual([unknown.tier, unknown.kind, unknown.glyph], ['today', 'unplaced', '?']);
  assert.match(unknown.waiting, /does not know/);
}

// A Fractal that presents nothing (an older Fractal, or an item it has not presented yet): each item is
// unplaced where its alert flag puts it, read as Fractal's CLI reads it, and says why; Fractal puts neither
// a message nor a fault on Now.
{
  const bare = (id, attention_kind, extra) => item(id, attention_kind, { presentation: undefined, ...extra });
  const rows = homeRows([
    bare('gate', 'human-gate', { alert: true }),
    bare('note', 'person-step', { alert: false }),
    bare('old', 'person-step', {}),
    bare('message', 'unread-message', { alert: false }),
    bare('fault', 'fault', { alert: true }),
  ], 'person/alex', now);
  assert.deepEqual(rows.map(row => [row.item.id, row.tier, row.kind]), [['attention/gate', 'stopped', 'unplaced'], ['attention/old', 'stopped', 'unplaced'], ['attention/note', 'later', 'unplaced']]);
  assert.match(rows[0].waiting, /Fractal is older, or has not presented it yet/);
}

// intake-b's #3048 cases: a null card keeps an item off Home; a custom kind shows as the Today
// request st presents; a presented item with no held step names none.
{
  const offNow = item('fault', 'fault', { alert: true, presentation: { tier: 'stopped', kind: null, agent: null, raised_by: null, waiting: null } });
  assert.deepEqual(homeRows([offNow], 'person/alex', now), []);
  const custom = item('deploy', 'custom.deploy-window', { alert: true, presentation: { tier: 'today', kind: 'request', agent: null, raised_by: null, waiting: null } });
  const [row] = homeRows([custom], 'person/alex', now);
  assert.deepEqual([row.tier, row.kind, row.waiting], ['today', 'request', null]);
}

// An item's own screen always has a row: the one Home lists, or an unplaced one for an item
// Home does not list (a fault or message opened by a link), keeping Fractal's alert flag.
{
  const gate = item('gate', 'human-gate', { alert: true, step_run_id: 'step-run/abc/build' });
  assert.deepEqual(detailRow(gate, now), homeRows([gate], 'person/alex', now)[0]);
  const fault = item('fault', 'fault', { alert: true });
  const message = item('note', 'unread-message', { alert: false });
  for (const [off, tier] of [[fault, 'stopped'], [message, 'later']]) {
    const row = detailRow(off, now);
    assert.deepEqual([row.kind, row.tier, row.glyph, typeof row.color], ['unplaced', tier, '?', 'string']);
    assert.match(row.waiting, /off Now/);
  }
}
