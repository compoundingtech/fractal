import assert from 'node:assert/strict';
import { test } from 'node:test';
import { nativeVisitPorts } from './unreadVisitTestPorts.mjs';

const mount = () => { const ports = nativeVisitPorts(); const visit = ports.useUnreadVisit(ports.options); return { ports, visit }; };
const stable = async ports => { await ports.flush(); await ports.advance(500); };
const reminder = id => ({ marks: [{ mark_id: id, anchor_message_id: 'message/visible' }], anchor_message_id: 'message/visible' });
const editable = ports => {
  const edits = [];
  ports.options.canEdit = true;
  ports.options.api = { state: 'ready', actions: { edit: async edit => { edits.push(edit); return { how: 'sent' }; } } };
  return edits;
};

test('only a focused foreground placement with 500ms stable measured content reads', async () => {
  const { ports } = mount();
  await stable(ports); assert.deepEqual(ports.reads, []);
  ports.focus(); await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  await ports.advance(1);
  assert.deepEqual(ports.reads, [['message/visible']]);
  ports.scroll(); await stable(ports);
  assert.equal(ports.reads.length, 1, 'acknowledge once per visit');
  ports.blur(); ports.scroll(); await stable(ports);
  assert.equal(ports.reads.length, 1);
});

test('viewport, layout, focus and AppState changes reset the entire dwell interval', async () => {
  for (const change of ['scroll', 'layout', 'focus', 'app']) {
    const { ports, visit } = mount();
    ports.focus(); await ports.flush(); await ports.advance(499);
    if (change === 'scroll') ports.scroll();
    if (change === 'layout') visit.refresh();
    if (change === 'focus') { ports.blur(); ports.focus(); }
    if (change === 'app') { ports.app('background'); ports.app('active'); }
    await ports.flush(); await ports.advance(499);
    assert.deepEqual(ports.reads, [], change);
    await ports.advance(1);
    assert.equal(ports.reads.length, 1, change);
  }
});

test('invisible or background content never starts a dwell timer', async () => {
  const { ports } = mount();
  ports.app('background'); ports.focus(); await stable(ports);
  assert.deepEqual(ports.reads, []);
  ports.options.measure = async () => ({ messageIds: [], readMessageIds: [] });
  ports.app('active'); await stable(ports);
  assert.deepEqual(ports.reads, []);
});

test('measurements completing after layout, blur or AppState change cannot start old timers', async () => {
  for (const hide of ['blur', 'background', 'layout']) {
    const { ports, visit } = mount();
    const pending = Promise.withResolvers();
    ports.options.measure = () => pending.promise;
    ports.focus(); await ports.flush();
    if (hide === 'blur') ports.blur();
    if (hide === 'background') { ports.app('background'); ports.app('active'); }
    if (hide === 'layout') visit.refresh();
    pending.resolve({ messageIds: ['message/old'], readMessageIds: ['message/old'] });
    await ports.advance(500);
    assert.deepEqual(ports.reads, [], hide);
  }
});

test('expiry remeasures and refuses content that changed without a geometry callback', async () => {
  const { ports } = mount();
  ports.focus(); await ports.flush();
  ports.options.measure = async () => ({ messageIds: ['message/new'], readMessageIds: ['message/new'] });
  await ports.advance(500); assert.deepEqual(ports.reads, []);
});

test('read-only clients cannot read or execute reminder intents', async () => {
  const { ports, visit } = mount();
  ports.options.canRead = false;
  ports.focus(); await stable(ports);
  assert.deepEqual(ports.reads, []);
  assert.equal((await visit.measure('unread')).how, 'failed');
});

test('explicit Mark unread is immediate, preserves natural history and supersedes observed IDs', async () => {
  const { ports, visit } = mount();
  const edits = editable(ports);
  ports.options.summary.unread_reminder = reminder('opaque/from-summary');
  ports.focus();
  assert.equal((await visit.measure('unread')).how, 'sent');
  assert.deepEqual(ports.reads, []);
  assert.deepEqual(edits, [{ kind: 'mark', anchor_message: 'message/visible', observed_marks: ['opaque/from-summary'] }]);
});

test('same-placement rerenders and remounts preserve entry capture; actual navigation exit ends it', async () => {
  const { ports, visit } = mount();
  const edits = editable(ports);
  ports.focus();
  assert.equal((await visit.measure('unread')).how, 'sent');
  ports.options.summary.unread_reminder = reminder('opaque/new-in-this-visit');
  ports.rerender(); await stable(ports);
  assert.equal(edits.filter(edit => edit.kind === 'clear').length, 0);
  for (let remount = 0; remount < 2; remount++) {
    ports.unmount(); // React cleanup alone is not a navigation blur.
    ports.useUnreadVisit(ports.options); ports.focus(); await stable(ports);
    assert.equal(edits.filter(edit => edit.kind === 'clear').length, 0);
  }
  assert.equal(ports.reads.length, 1, 'natural receipts also survive same-placement remounts');
  ports.blur(); ports.useUnreadVisit(ports.options); ports.focus(); await stable(ports);
  assert.deepEqual(edits.at(-1), { kind: 'clear', observed_marks: ['opaque/new-in-this-visit'] });
});

test('Unknown reminder at entry is never retroactively captured after loading or remount', async () => {
  const { ports } = mount();
  const edits = editable(ports);
  ports.options.summary = undefined;
  ports.focus(); await stable(ports);
  ports.options.summary = { unread: true, unread_reminder: reminder('opaque/late') };
  ports.unmount(); ports.useUnreadVisit(ports.options); ports.focus(); await stable(ports);
  assert.deepEqual(edits, []);
  ports.blur(); ports.useUnreadVisit(ports.options); ports.focus(); await stable(ports);
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/late'] }]);
});

test('different placements cannot share captured visits', async () => {
  const { ports } = mount();
  const edits = editable(ports);
  ports.focus(); await stable(ports);
  ports.options.summary.unread_reminder = reminder('opaque/new');
  ports.unmount(); ports.options.placementKey = 'context/other';
  ports.useUnreadVisit(ports.options); ports.focus(); await stable(ports);
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/new'] }]);
});

test('a failed natural read releases the unsent clear for the next stable view', async () => {
  const { ports } = mount();
  const edits = editable(ports);
  ports.options.summary.unread_reminder = reminder('opaque/captured');
  let failed = false;
  ports.options.read = async () => {
    if (!failed) { failed = true; return { how: 'failed', error: 'Read refused' }; }
    return { how: 'sent' };
  };
  ports.focus(); await stable(ports); assert.deepEqual(edits, []);
  ports.scroll(); await stable(ports);
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/captured'] }]);
});
