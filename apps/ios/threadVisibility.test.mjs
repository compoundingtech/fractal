import assert from 'node:assert/strict';
import { test } from 'node:test';
import { intersectThreadBounds, visibleThreadMessages } from './threadVisibility.ts';

const box = (y, height) => ({ x: 0, y, width: 100, height });

test('nested clipping and keyboard bounds exclude offscreen and unmeasured messages', () => {
  const viewport = intersectThreadBounds(intersectThreadBounds(box(100, 400), box(150, 250)), box(0, 275));
  assert.deepEqual(viewport, box(150, 125));
  const rows = [
    { id: 'below-keyboard', bounds: box(275, 40) },
    { id: 'visible-part', bounds: box(250, 50) },
    { id: 'unknown' },
    { id: 'visible-whole', bounds: box(170, 25) },
    { id: 'clipped-above', bounds: box(110, 40) },
  ];
  assert.deepEqual(visibleThreadMessages(viewport, rows, 50), ['visible-part', 'visible-whole']);
  assert.deepEqual(visibleThreadMessages(viewport, rows, 1), ['visible-part']);
  assert.equal(intersectThreadBounds(box(0, 20), box(20, 20)), undefined);
});
