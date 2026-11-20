import assert from 'node:assert/strict';
import { test } from 'node:test';
import { threadTitleFromAsk, utf8Bytes } from '@fractal/views';

test('an ask title is trimmed and kept up to the byte limit without splitting a character', () => {
  assert.equal(threadTitleFromAsk('  Review this change  '), 'Review this change');
  assert.equal(threadTitleFromAsk('a'.repeat(512) + 'b'), 'a'.repeat(512));
  const mixed = 'a'.repeat(509) + '🙂' + 'b';
  assert.equal(threadTitleFromAsk(mixed), 'a'.repeat(509));
  assert.equal(utf8Bytes(threadTitleFromAsk('🙂'.repeat(129))), 512);
  assert.equal(threadTitleFromAsk('a'.repeat(511) + ' b'), 'a'.repeat(511));
});

test('an ask without a usable title gets a default', () => {
  assert.equal(threadTitleFromAsk(' \n\0 '), 'Follow-up');
  assert.equal(threadTitleFromAsk('An\0 ask'), 'An ask');
});
