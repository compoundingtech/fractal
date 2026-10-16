import { test } from 'node:test'
import assert from 'node:assert/strict'
import { phaseTimings } from './kit-play-phases.mjs'

test('attributes each interval to the phase that began it, ending at storyFinished', () => {
  assert.deepEqual(phaseTimings([['preparing', 100], ['rendering', 160], ['playing', 400], ['completed', 450], ['storyFinished', 1450]]), {
    storyBootMilliseconds: 100,
    storyPhase_preparing_Milliseconds: 60,
    storyPhase_rendering_Milliseconds: 240,
    storyPhase_playing_Milliseconds: 50,
    storyPhase_completed_Milliseconds: 1000,
  })
})
test('sums a phase entered twice and never reports a negative interval', () => {
  const timings = phaseTimings([['rendering', 10], ['playing', 30], ['rendering', 50], ['storyFinished', 40]])
  assert.equal(timings.storyPhase_rendering_Milliseconds, 20)
  assert.equal(timings.storyPhase_playing_Milliseconds, 20)
})
test('reports nothing for missing or malformed marks', () => {
  for (const marks of [undefined, null, [], [['x']], [[1, 2]], [['x', Number.NaN]]]) assert.deepEqual(phaseTimings(marks), {})
})
