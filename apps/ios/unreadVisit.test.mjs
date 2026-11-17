import assert from 'node:assert/strict';
import { test } from 'node:test';
import { anchorPresentation, reminderFromSummary, unreadVisit, visitControl } from './unreadVisit.ts';
import { applyVisitEffect, pendingUnreadAPI, UnreadVisitController } from './unreadActions.ts';

const mark = (mark_id, anchor_message_id) => ({ mark_id, anchor_message_id });
const summary = (...marks) => ({ unread: 0, unread_reminder: marks.length ? { marks, anchor_message_id: marks[0].anchor_message_id } : null });
const start = (marks = [], canRead = true, canEdit = true) => {
  const control = new UnreadVisitController(unreadVisit(canRead, canEdit));
  control.observe(summary(...marks));
  control.send({ kind: 'start', foreground: true, visible: true });
  return control;
};
const view = (control, messageIds, deliberate = true) => control.send({ kind: 'meaningful-content-visible', messageIds, deliberate });
const clears = result => result.effects.filter(effect => effect.kind === 'edit').flatMap(effect => effect.edit.observed_marks);

test('background, unfocused, invisible, ended and empty views never advance or clear', () => {
  for (const foreground of [false, true]) for (const visible of [false, true]) {
    const control = start([mark('opaque/id', 'message/a')]);
    control.send({ kind: 'foreground', value: foreground });
    control.send({ kind: 'visibility', value: visible });
    const effects = view(control, ['message/a']).effects;
    assert.equal(effects.length > 0, foreground && visible);
    control.send({ kind: 'end' });
    assert.deepEqual(view(control, ['message/a']).effects, []);
  }
  assert.deepEqual(view(start(), []).effects, []);
  const idle = unreadVisit(true, true);
  assert.deepEqual(visitControl(idle, { kind: 'meaningful-content-visible', messageIds: ['message/a'], deliberate: true }).effects, []);
});

test('a mark remains sticky for the current visit, including refocus notifications and foreground changes', () => {
  const control = start();
  view(control, ['message/a']);
  const intent = control.send({ kind: 'mark-unread', anchor: { message_id: 'message/a', state: 'current' } });
  assert.deepEqual(intent.effects, [{ kind: 'edit', edit: { kind: 'mark', anchor_message: 'message/a', observed_marks: [] } }]);
  assert.equal(control.state.reminder, undefined, 'never optimistically invent a mark');
  control.observe(summary(mark('signed-operation/opaque', 'message/a')));
  control.send({ kind: 'foreground', value: false });
  control.send({ kind: 'foreground', value: true });
  control.send({ kind: 'start', foreground: true, visible: true });
  assert.deepEqual(clears(view(control, ['message/a'])), []);
  control.send({ kind: 'end' });
  control.send({ kind: 'start', foreground: true, visible: true });
  assert.deepEqual(clears(view(control, ['message/a'])), ['signed-operation/opaque']);
});

test('later visits clear only captured IDs whose own anchor was deliberately viewed', () => {
  const a = mark('op/a', 'message/a'), b = mark('op/b', 'message/b');
  const control = start([a, b]);
  control.observe(summary(a, b, mark('op/concurrent', 'message/a')));
  assert.deepEqual(clears(view(control, ['message/a'], false)), []);
  assert.deepEqual(clears(view(control, ['message/a'])), ['op/a']);
  assert.deepEqual(clears(view(control, ['message/a'])), [], 'one in-flight clear per ID');
  assert.deepEqual(clears(view(control, ['message/b'])), ['op/b']);
  assert.equal(control.state.reminder.marks.length, 3, 'only a summary can remove marks');
});

test('manual clear and new mark supersede all observed IDs, not concurrent unseen IDs', () => {
  const control = start([mark('op/a', 'message/a'), mark('op/b', 'message/b')]);
  view(control, ['message/c']);
  const marked = control.send({ kind: 'mark-unread', anchor: { message_id: 'message/c', state: 'current' } });
  assert.deepEqual(marked.effects[0].edit, { kind: 'mark', anchor_message: 'message/c', observed_marks: ['op/a', 'op/b'] });
  assert.deepEqual(control.send({ kind: 'mark-read' }).effects, [{ kind: 'edit', edit: { kind: 'clear', observed_marks: ['op/a', 'op/b'] } }]);
  assert.deepEqual(control.state.reminder, summary(mark('op/a', 'message/a'), mark('op/b', 'message/b')).unread_reminder);
});

test('unavailable anchors keep the reminder without a body, and cannot be marked again', () => {
  const control = start([mark('op/missing', 'message/missing')]);
  assert.deepEqual(clears(view(control, ['message/other'])), []);
  assert.equal(anchorPresentation({ message_id: 'message/missing', state: 'unavailable' }).body, false);
  assert.ok(control.send({ kind: 'mark-unread', anchor: { message_id: 'message/missing', state: 'unavailable' } }).refusal);
  assert.equal(control.state.reminder.marks[0].mark_id, 'op/missing');
  assert.deepEqual(clears(control.send({ kind: 'mark-read' })), ['op/missing'], 'manual clear does not require an available anchor');
});

test('pending seek intent exposes no body and never creates a reminder or badge', () => {
  const pending = { message_id: 'message/unknown-intent', state: 'pending' };
  assert.equal(anchorPresentation(pending).body, false);
  assert.equal(reminderFromSummary({ unread: 0, anchor: pending }), undefined);
  const control = start();
  assert.ok(control.send({ kind: 'mark-unread', anchor: pending }).refusal);
  assert.equal(control.state.reminder, undefined);
  assert.equal(reminderFromSummary({ unread: 42 }), undefined, 'natural unread is not the reminder');
});

test('read-only refuses all write intents; capability loss also fences adapters', async () => {
  const control = start([mark('op/a', 'message/a')], false, false);
  assert.deepEqual(view(control, ['message/a']).effects, []);
  assert.ok(control.send({ kind: 'mark-read' }).refusal);
  assert.ok(control.send({ kind: 'mark-unread', anchor: { message_id: 'message/a', state: 'current' } }).refusal);
  const api = { state: 'ready', actions: {
    read: async () => assert.fail('read-only read reached transport'),
    edit: async () => assert.fail('read-only edit reached transport'),
    seek: async () => assert.fail('unexpected seek'),
  } };
  for (const effect of [{ kind: 'read', messageIds: ['message/a'] }, { kind: 'edit', edit: { kind: 'clear', observed_marks: ['op/a'] } }])
    assert.equal((await applyVisitEffect(effect, api, { canRead: false, canEdit: false })).how, 'failed');
  assert.equal((await applyVisitEffect({ kind: 'edit', edit: { kind: 'clear', observed_marks: [] } }, pendingUnreadAPI, { canRead: true, canEdit: true })).how, 'failed');
});

test('IDs come only from summary, never from an action-result operation_id or local identity source', () => {
  const control = start();
  view(control, ['message/a']);
  const before = control.state;
  control.send({ kind: 'edit-finished', operation_id: 'receipt/not-a-mark', mark_id: 'also-not-a-mark' });
  assert.equal(control.state, before);
  assert.equal(control.state.reminder, undefined);
  const opaque = 'not-a-claim:retry#1/é/0000';
  control.observe(summary(mark(opaque, 'message/a')));
  assert.deepEqual(clears(control.send({ kind: 'mark-read' })), [opaque]);
  assert.ok(!JSON.stringify(control.state).includes('receipt/not-a-mark'));
});

test('failed reads and clears can be deliberately attempted again without new IDs', () => {
  const control = start([mark('op/a', 'message/a')]);
  view(control, ['message/a']);
  control.send({ kind: 'read-failed', messageIds: ['message/a'] });
  control.send({ kind: 'edit-failed', observed_marks: ['op/a'] });
  assert.deepEqual(view(control, ['message/a']).effects, [
    { kind: 'read', messageIds: ['message/a'] },
    { kind: 'edit', edit: { kind: 'clear', observed_marks: ['op/a'] } },
  ]);
});

test('own-message anchors can be viewed without advancing the natural read history', () => {
  const control = start([mark('op/own', 'message/own')]);
  const transition = control.send({ kind: 'meaningful-content-visible', messageIds: ['message/own'], readMessageIds: [], deliberate: true });
  assert.deepEqual(transition.effects, [{ kind: 'edit', edit: { kind: 'clear', observed_marks: ['op/own'] } }]);
});

test('property: every subset/permutation clears exactly the captured, still-active, viewed anchors', () => {
  const marks = Array.from({ length: 5 }, (_, i) => mark('opaque/' + i, 'message/' + (i % 3)));
  for (let captured = 0; captured < 32; captured++) for (let active = 0; active < 32; active++) for (let viewed = 0; viewed < 8; viewed++) {
    const capturedMarks = marks.filter((_, i) => captured & (1 << i));
    const activeMarks = marks.filter((_, i) => active & (1 << i)).reverse();
    const control = start(capturedMarks);
    const concurrent = mark('concurrent/never-captured', 'message/0');
    control.observe(summary(...activeMarks, concurrent));
    const ids = [0, 1, 2].filter(i => viewed & (1 << i)).map(i => 'message/' + i);
    const actual = clears(view(control, [...ids, ...ids]));
    const expected = activeMarks.filter(mark => capturedMarks.some(captured => captured.mark_id === mark.mark_id) && ids.includes(mark.anchor_message_id)).map(mark => mark.mark_id);
    assert.deepEqual(actual, expected);
    assert.ok(!actual.includes(concurrent.mark_id));
  }
});

test('Unknown entry stays uncaptured until a real navigation exit, while start restores visibility', () => {
  const control = new UnreadVisitController(unreadVisit(true, true));
  control.send({ kind: 'start', foreground: true, visible: true });
  control.observe(summary(mark('opaque/late', 'message/a')));
  control.send({ kind: 'visibility', value: false });
  control.send({ kind: 'start', foreground: true, visible: true });
  assert.deepEqual(clears(view(control, ['message/a'])), []);
  control.send({ kind: 'end' });
  control.send({ kind: 'start', foreground: true, visible: true });
  assert.deepEqual(clears(view(control, ['message/a'])), ['opaque/late']);
});
