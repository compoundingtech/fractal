import assert from 'node:assert/strict';
import { test } from 'node:test';
import { nativeVisitPorts } from './unreadVisitTestPorts.mjs';

const mount = () => { const ports = nativeVisitPorts(); const visit = ports.useUnreadVisit(ports.options); return { ports, visit }; };
test('mounting a background tab is not a visit; focus and measured content are both required', async () => {
  const { ports } = mount();
  await ports.flush();
  assert.deepEqual(ports.reads, []);
  ports.focus(); await ports.flush();
  assert.deepEqual(ports.reads, [['message/visible']]);
  ports.scroll(); await ports.flush();
  assert.equal(ports.reads.length, 1, 'same visible content is acknowledged only once');
  ports.blur(); ports.scroll(); await ports.flush();
  assert.equal(ports.reads.length, 1);
});

test('background and invisible native content never auto marks read', async () => {
  const { ports } = mount();
  ports.app('background'); ports.focus(); await ports.flush();
  assert.deepEqual(ports.reads, []);
  ports.options.measure = async () => ({ messageIds: [], readMessageIds: [] });
  ports.app('active'); await ports.flush();
  assert.deepEqual(ports.reads, []);
});

test('native measurements are fenced across blur and background/foreground even when they finish later', async () => {
  for (const hide of ['blur', 'background']) {
    const { ports } = mount();
    const pending = Promise.withResolvers();
    ports.options.measure = () => pending.promise;
    ports.focus(); await ports.flush();
    if (hide === 'blur') ports.blur();
    else { ports.app('background'); ports.app('active'); }
    pending.resolve({ messageIds: ['message/old-visibility'], readMessageIds: ['message/old-visibility'] });
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(ports.reads, [], hide);
  }
});

test('read-only native clients do not mark content or expose an executable reminder action', async () => {
  const { ports, visit } = mount();
  ports.options.canRead = false; ports.options.canEdit = false;
  ports.focus(); await ports.flush();
  assert.deepEqual(ports.reads, []);
  assert.equal((await visit.measure('unread')).how, 'failed');
});

test('explicit Mark unread preserves natural reads and supersedes observed IDs only through the ready adapter', async () => {
  const { ports, visit } = mount();
  const edits = [];
  ports.options.canEdit = true;
  ports.options.summary.unread_reminder = { marks: [{ mark_id: 'opaque/from-summary', anchor_message_id: 'message/visible' }], anchor_message_id: 'message/visible' };
  ports.options.api = { state: 'ready', actions: {
    read: async () => assert.fail('existing natural read port should be reused'),
    edit: async edit => { edits.push(edit); return { how: 'sent', operation_id: 'receipt/not-a-mark' }; },
    seek: async () => assert.fail('measuring is not seeking'),
  } };
  ports.focus();
  assert.equal((await visit.measure('unread')).how, 'sent');
  assert.deepEqual(ports.reads, [], 'the unread intent does not alter natural history');
  assert.deepEqual(edits, [{ kind: 'mark', anchor_message: 'message/visible', observed_marks: ['opaque/from-summary'] }]);
});

test('a failed natural read does not reserve a clear that was never sent', async () => {
  const { ports } = mount();
  const edits = [];
  let failed = false;
  ports.options.canEdit = true;
  ports.options.summary.unread_reminder = { marks: [{ mark_id: 'opaque/captured', anchor_message_id: 'message/visible' }], anchor_message_id: 'message/visible' };
  ports.options.api = { state: 'ready', actions: {
    edit: async edit => { edits.push(edit); return { how: 'sent' }; },
  } };
  ports.options.read = async () => {
    if (!failed) { failed = true; return { how: 'failed', error: 'Read refused' }; }
    return { how: 'sent' };
  };
  ports.focus(); await ports.flush();
  assert.deepEqual(edits, []);
  ports.scroll(); await ports.flush();
  assert.deepEqual(edits, [{ kind: 'clear', observed_marks: ['opaque/captured'] }]);
});
