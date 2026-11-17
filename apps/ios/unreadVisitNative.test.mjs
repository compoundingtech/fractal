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

test('an action sheet opening at 300ms suspends reads and captured clears until a fresh 500ms', async () => {
  const { ports } = mount();
  const edits = editable(ports);
  ports.options.summary.unread_reminder = reminder('opaque/captured');
  ports.focus(); await ports.flush(); await ports.advance(300);
  ports.showSheet(); await ports.flush(); await ports.advance(1000);
  assert.deepEqual(ports.reads, []);
  assert.deepEqual(edits, []);
  ports.dismissSheet(); await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  assert.deepEqual(edits, []);
  await ports.advance(1);
  assert.deepEqual(ports.reads, [['message/visible']]);
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/captured'] }]);
});

test('opening and dismissing a sheet fences queued initial and expiry native measurements', async () => {
  for (const phase of ['initial', 'expiry']) {
    const { ports } = mount();
    const pending = Promise.withResolvers();
    if (phase === 'initial') ports.options.measure = () => pending.promise;
    ports.focus(); await ports.flush();
    if (phase === 'expiry') {
      ports.options.measure = () => pending.promise;
      await ports.advance(500);
    }
    ports.showSheet(); ports.dismissSheet();
    ports.options.measure = async () => ({ messageIds: ['message/fresh'], readMessageIds: ['message/fresh'] });
    pending.resolve({ messageIds: ['message/stale'], readMessageIds: ['message/stale'] });
    await ports.advance(500);
    assert.deepEqual(ports.reads, [], phase);
    await ports.flush(); await ports.advance(499);
    assert.deepEqual(ports.reads, [], phase);
    await ports.advance(1);
    assert.deepEqual(ports.reads, [['message/fresh']], phase);
  }
});

test('nested presentation owners release independently and idempotently', async () => {
  const { ports } = mount();
  ports.focus(); await ports.flush(); await ports.advance(300);
  const release = ports.ownedPresentations.acquire();
  ports.showSheet();
  release(); release();
  await stable(ports);
  assert.deepEqual(ports.reads, []);
  ports.dismissSheet(); await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  await ports.advance(1);
  assert.equal(ports.reads.length, 1);
});

test('a sheet callback opening another sheet never uncovers the body between them', async () => {
  const { ports } = mount();
  ports.focus(); await ports.flush();
  ports.showSheet(() => ports.showSheet());
  ports.dismissSheet();
  assert.equal(ports.ownedPresentations.isOccluded(), true);
  await stable(ports); assert.deepEqual(ports.reads, []);
  ports.dismissSheet(); await stable(ports);
  assert.equal(ports.reads.length, 1);
});

test('owned Modal holds occlusion until native dismissal and releases on unmount', async () => {
  const { ports } = mount();
  ports.focus(); await ports.flush();
  let dismissed = 0;
  const modal = ports.modal({ visible: true, onDismiss: () => dismissed++ });
  modal.componentDidMount();
  modal.props = { ...modal.props, visible: false };
  modal.componentDidUpdate();
  await stable(ports); assert.deepEqual(ports.reads, []);
  modal.render().props.onDismiss();
  assert.equal(dismissed, 1);
  await stable(ports); assert.equal(ports.reads.length, 1);
  modal.componentWillUnmount();
  assert.equal(ports.ownedPresentations.isOccluded(), false);
  const removed = ports.modal({ visible: true });
  removed.componentDidMount(); removed.componentWillUnmount();
  assert.equal(ports.ownedPresentations.isOccluded(), false);
});

test('keyboard show, hide and frame events without scrolling invalidate the entire dwell lease', async () => {
  for (const event of ['keyboardWillShow', 'keyboardDidShow', 'keyboardWillHide', 'keyboardDidHide', 'keyboardWillChangeFrame', 'keyboardDidChangeFrame']) {
    const { ports } = mount();
    ports.focus(); await ports.flush(); await ports.advance(300);
    ports.keyboard(event); await ports.flush(); await ports.advance(499);
    assert.deepEqual(ports.reads, [], event);
    await ports.advance(1); assert.equal(ports.reads.length, 1, event);
  }
});

test('keyboard transitions invalidate late queued measurements even without scrolling', async () => {
  const { ports } = mount();
  const pending = Promise.withResolvers();
  ports.options.measure = () => pending.promise;
  ports.focus(); await ports.flush();
  ports.keyboard('keyboardWillShow'); ports.keyboard('keyboardDidShow');
  ports.options.measure = async () => ({ messageIds: ['message/fresh'], readMessageIds: ['message/fresh'] });
  pending.resolve({ messageIds: ['message/stale'], readMessageIds: ['message/stale'] });
  await ports.advance(500); assert.deepEqual(ports.reads, []);
  await stable(ports); assert.deepEqual(ports.reads, [['message/fresh']]);
});

test('sheet to prompt to alert keeps ownership through nested native callbacks', async () => {
  const { ports } = mount();
  const words = [];
  ports.focus(); await ports.flush(); await ports.advance(300);
  ports.showSheet(() => ports.prompt(text => { words.push(text); ports.alert(); }));
  ports.dismissSheet();
  await stable(ports); assert.deepEqual(ports.reads, []);
  ports.dismissAlert(1, 'Keep this');
  assert.deepEqual(words, ['Keep this']);
  await stable(ports); assert.deepEqual(ports.reads, []);
  ports.dismissAlert(); await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  await ports.advance(1); assert.equal(ports.reads.length, 1);
});

test('native prompt cancellation and alert dismissal release their own leases', async () => {
  const { ports } = mount();
  ports.focus(); await ports.flush();
  ports.prompt(() => assert.fail('Cancel must not submit'));
  ports.dismissAlert(0);
  assert.equal(ports.ownedPresentations.isOccluded(), false);
  let dismissed = 0;
  ports.alert(undefined, { cancelable: true, onDismiss: () => dismissed++ });
  await stable(ports); assert.deepEqual(ports.reads, []);
  ports.cancelAlert();
  assert.equal(dismissed, 1);
  await stable(ports); assert.equal(ports.reads.length, 1);
});

test('occlusion while natural read is in flight releases an unsent automatic clear', async () => {
  const { ports } = mount();
  const edits = editable(ports);
  const read = Promise.withResolvers();
  ports.options.summary.unread_reminder = reminder('opaque/captured');
  ports.options.read = () => read.promise;
  ports.focus(); await stable(ports);
  ports.showSheet();
  read.resolve({ how: 'sent' });
  await ports.advance(500);
  assert.deepEqual(edits, []);
  ports.options.read = async () => ({ how: 'sent' });
  ports.dismissSheet(); await stable(ports);
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/captured'] }]);
});

test('native prompt callbacks default missing text while button callbacks retain credential objects', () => {
  const { ports } = mount();
  const values = [];
  ports.prompt(value => values.push(value));
  ports.dismissAlert(1);
  assert.deepEqual(values, ['']);
  assert.equal(ports.ownedPresentations.isOccluded(), false);
  const credentials = { login: 'avery', password: 'private' };
  ports.prompt([{ text: 'OK', onPress: value => values.push(value) }]);
  ports.dismissAlert(0, credentials);
  assert.equal(values[1], credentials, 'native login/password arguments pass through unchanged');
  assert.equal(ports.ownedPresentations.isOccluded(), false);
});
