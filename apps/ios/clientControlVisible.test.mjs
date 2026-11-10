import assert from 'node:assert/strict';
import { test } from 'node:test';
import { committedVisible, publishVisible, subscribeVisible } from './clientControlVisible.ts';

test('committed state is isolated by navigator route key and removed on cleanup', () => {
  const state = { route: 'Attention', screen: 'Attention', visible_item_ids: ['ask/one'], opened_item: { id: 'ask/one', title: 'A request', body: 'Question rendered by the normal request view' } };
  assert.equal(committedVisible('not-committed'), undefined);
  publishVisible('committed-attention', state);
  assert.equal(committedVisible('committed-attention'), state);
  assert.equal(committedVisible('another-attention-instance'), undefined);
  publishVisible('committed-attention', undefined);
  assert.equal(committedVisible('committed-attention'), undefined);
});

test('viewability publishes only renderer-observed IDs, including an empty list', () => {
  let notifications = 0;
  const stop = subscribeVisible(() => notifications++);
  publishVisible('now', { route: 'NowRoot', screen: 'NowRoot', visible_item_ids: ['ask/visible'], opened_item: null });
  assert.deepEqual(committedVisible('now').visible_item_ids, ['ask/visible']);
  publishVisible('now', { route: 'NowRoot', screen: 'NowRoot', visible_item_ids: [], opened_item: null });
  assert.deepEqual(committedVisible('now').visible_item_ids, []);
  publishVisible('now', undefined);
  assert.equal(notifications, 3);
  stop();
  publishVisible('now', undefined);
  assert.equal(notifications, 3);
});
