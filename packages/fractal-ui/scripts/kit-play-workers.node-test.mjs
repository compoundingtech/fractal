import { test } from 'node:test'
import assert from 'node:assert/strict'
import { playWorkers, MAX_WORKERS } from './kit-play-workers.mjs'

test('without a cap the gate fills the machine, up to six workers, as before', () => {
  assert.equal(playWorkers({}, 4), 4)
  assert.equal(playWorkers({}, 16), MAX_WORKERS)
  assert.equal(playWorkers({ KIT_PLAY_WORKERS: '' }, 16), MAX_WORKERS, 'an empty value is unset')
})

test('a cap lowers the worker count and never raises it', () => {
  assert.equal(playWorkers({ KIT_PLAY_WORKERS: '2' }, 16), 2)
  assert.equal(playWorkers({ KIT_PLAY_WORKERS: '8' }, 16), MAX_WORKERS)
  assert.equal(playWorkers({ KIT_PLAY_WORKERS: '8' }, 4), 4)
})

test('a cap that is not a positive whole number is refused', () => {
  for (const value of ['0', '-1', '1.5', 'two', ' 2']) {
    assert.throws(() => playWorkers({ KIT_PLAY_WORKERS: value }, 16), /KIT_PLAY_WORKERS/, value)
  }
})
