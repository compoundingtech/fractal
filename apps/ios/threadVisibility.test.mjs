import assert from 'node:assert/strict';
import { test } from 'node:test';
import { intersectThreadBounds, visibleThreadMessages } from './threadVisibility.ts';

const box = (y, height) => ({ x: 0, y, width: 100, height });
const row = (id, y, height, lineHeight = 12) => ({ id, bounds: box(y, height), lineHeight });

test('nested clipping and keyboard bounds exclude offscreen and unmeasured content', () => {
  const viewport = intersectThreadBounds(intersectThreadBounds(box(100, 400), box(150, 250)), box(0, 275));
  assert.deepEqual(viewport, box(150, 125));
  const rows = [row('below-keyboard', 275, 40), row('trailing-end-clipped', 250, 50),
    { id: 'unknown' }, row('visible-whole', 170, 25), row('clipped-above', 110, 40)];
  assert.deepEqual(visibleThreadMessages(viewport, rows, 50), ['visible-whole']);
  assert.equal(intersectThreadBounds(box(0, 20), box(20, 20)), undefined);
});

test('read geometry requires full width, trailing end and two lines, or the whole of a shorter body', () => {
  const viewport = box(100, 100);
  const rows = [
    row('one-pixel', 199, 100), row('single-line', 110, 18),
    row('clipped-single-line-top', 92, 12), row('clipped-single-line-tail', 195, 12),
    row('top-only', 176, 100), row('two-lines', 176, 24),
    row('trailing-two-lines', 50, 100), row('one-line-at-tail', 50, 62),
    { ...row('horizontal-clip', 110, 30), bounds: { x: -1, y: 110, width: 100, height: 30 } },
    { id: 'unknown-lines', bounds: box(110, 40) }, row('invalid-lines', 110, 40, -1),
  ];
  assert.deepEqual(visibleThreadMessages(viewport, rows, 50), ['single-line', 'two-lines', 'trailing-two-lines']);
  assert.deepEqual(visibleThreadMessages(viewport, rows, 1), ['single-line']);
});

test('a fully visible one-line body qualifies; a clipped one and one visible line of several do not', () => {
  const viewport = box(100, 100);
  assert.deepEqual(visibleThreadMessages(viewport, [row('whole-one-line', 140, 12)], 50), ['whole-one-line']);
  assert.deepEqual(visibleThreadMessages(viewport, [row('top-clipped', 95, 12), row('bottom-clipped', 194, 12)], 50), []);
  // Only the last of two lines shows: the trailing end is visible but one line is not two.
  assert.deepEqual(visibleThreadMessages(viewport, [row('last-of-two', 88, 24), row('last-of-five', 52, 60)], 50), []);
});

test('line geometry follows actual text size, not a fixed pixel heuristic', () => {
  assert.deepEqual(visibleThreadMessages(box(100, 100), [
    row('large-one-line', 164, 36, 24), row('large-clipped-line', 190, 24, 24),
    row('large-one-visible-line', 76, 48, 24), row('large-two-lines', 152, 48, 24),
    row('small-two-lines', 184, 16, 8),
  ], 50), ['large-one-line', 'large-two-lines', 'small-two-lines']);
});
