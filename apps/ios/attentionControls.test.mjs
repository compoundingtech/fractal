import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dismissalKind } from './attentionControls.ts';
const ask = { state: 'open', attention_kind: 'person-step', requester_id: 'agent/release', actions: ['work.done'], request: { answers: [{ id: 'approve' }] } };
test('dismiss preserves its own outcome instead of choosing an approval', () => {
  assert.equal(dismissalKind(ask), 'dismiss');
  assert.equal(dismissalKind({ ...ask, request: { answers: [{ id: 'dismiss' }] } }), 'answer');
});
test('authored work, closed asks, and gate-only decisions never gain dismissal', () => {
  assert.equal(dismissalKind({ ...ask, requester_id: undefined }), null);
  assert.equal(dismissalKind({ ...ask, closedElsewhere: true }), null);
  assert.equal(dismissalKind({ ...ask, actions: ['review.approve'] }), null);
});
test('updates and messages retain their read actions', () => {
  assert.equal(dismissalKind({ ...ask, update: {} }), 'read');
  assert.equal(dismissalKind({ ...ask, attention_kind: 'message', actions: ['message.read'] }), 'message');
});
