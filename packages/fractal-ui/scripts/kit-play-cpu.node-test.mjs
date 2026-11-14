import { test } from 'node:test'
import assert from 'node:assert/strict'
import { kitCpuDelta, parseProcStat } from './kit-play-cpu.mjs'

const zeroTicks = { user: 0, nice: 0, system: 0, idle: 0, iowait: 0, irq: 0, softirq: 0, steal: 0 }
const snapshot = (procStat = { ...zeroTicks }, busyMilliseconds = 0) => ({ busyMilliseconds, procStat })

test('parses the Linux aggregate line as raw ticks, excluding guest double counting', () => {
  const text = 'cpu  3357 34 4313 1362393 17 7 92 41 120 3\n'
    + 'cpu0 100 0 200 300 4 5 6 7 8 9\n'
    + 'intr 12345 0 0 0\nbtime 1710000000\n'
  assert.deepEqual(parseProcStat(text), {
    user: 3357, nice: 34, system: 4313, idle: 1362393, iowait: 17, irq: 7, softirq: 92, steal: 41,
  })
  assert.deepEqual(parseProcStat('cpu\t0 0 0 0 0 0 0 0\n'), zeroTicks)
})

test('refuses malformed, absent, or inexact Linux aggregate counters', () => {
  for (const text of [
    '',
    'cpu0 1 2 3 4 5 6 7 8\n',
    'cpu 1 2 3 4 5 6 7\n',
    'cpu 1 2 3 4 5 6 7 -8\n',
    'cpu 1 2 3 4 5 6 7 1.5\n',
    'cpu 1 2 3 4 5 6 7 NaN\n',
    'cpu 1 2 3 4 5 6 7 Infinity\n',
    'cpu 1 2 3 4 5 6 7 8e2\n',
    'cpu 1 2 3 4 5 6 7 9007199254740992\n',
    'cpu 1 2 3 4 5 6 7 8 -1 0\n',
    'cpu 1 2 3 4 5 6 7 8 0 invalid\n',
    'cpu 1 2 3 4 5 6 7 8 0 0 0\n',
  ]) {
    assert.throws(() => parseProcStat(text), /\/proc\/stat/, text)
  }
  assert.throws(() => parseProcStat(null), /must be text/)
})

test('returns machine busy core-minutes and each raw tick delta', () => {
  const before = snapshot(parseProcStat('cpu 100 200 300 400 500 600 700 800 10 20'), 120000)
  const after = snapshot(parseProcStat('cpu 101 202 303 404 505 606 707 808 30 40'), 210000)
  assert.deepEqual(kitCpuDelta(before, after), {
    busyCoreMinutes: 1.5,
    procStat: {
      available: true,
      ticks: { user: 1, nice: 2, system: 3, idle: 4, iowait: 5, irq: 6, softirq: 7, steal: 8 },
      stealTicks: 8,
      stealFraction: 8 / 36,
    },
  })
})

test('steal fraction includes steal itself and all eight fields in the denominator', () => {
  const after = snapshot({ user: 10, nice: 10, system: 10, idle: 10, iowait: 10, irq: 10, softirq: 10, steal: 30 })
  assert.equal(kitCpuDelta(snapshot(), after).procStat.stealFraction, 0.3)
  assert.equal(kitCpuDelta(snapshot(), snapshot({ ...zeroTicks, steal: 20 })).procStat.stealFraction, 1)
  assert.equal(kitCpuDelta(snapshot(), snapshot()).procStat.stealFraction, 0)
})

test('explicitly reports unavailable non-Linux snapshots without fake zero steal', () => {
  for (const [before, after] of [
    [snapshot(null), snapshot(null, 60000)],
    [snapshot(null), snapshot(zeroTicks, 60000)],
    [snapshot(zeroTicks), snapshot(null, 60000)],
  ]) {
    assert.deepEqual(kitCpuDelta(before, after), {
      busyCoreMinutes: 1,
      procStat: { available: false },
    })
  }
})

test('rejects regressing counters rather than clamping deltas', () => {
  assert.throws(() => kitCpuDelta(snapshot(zeroTicks, 2), snapshot(zeroTicks, 1)), /milliseconds delta/)
  for (const field of Object.keys(zeroTicks)) {
    assert.throws(() => kitCpuDelta(snapshot({ ...zeroTicks, [field]: 1 }), snapshot()), new RegExp(`${field} ticks delta`))
  }
})

test('rejects invalid counter snapshots even when another endpoint is unavailable', () => {
  for (const value of [-1, 0.5, NaN, Infinity, undefined, Number.MAX_SAFE_INTEGER + 1]) {
    const invalidBusy = { busyMilliseconds: value, procStat: zeroTicks }
    assert.throws(() => kitCpuDelta(invalidBusy, snapshot()), /milliseconds/)
    assert.throws(() => kitCpuDelta(snapshot(), invalidBusy), /milliseconds/)
    for (const field of Object.keys(zeroTicks)) {
      const invalid = snapshot({ ...zeroTicks, [field]: value })
      assert.throws(() => kitCpuDelta(invalid, snapshot()), /ticks/)
      assert.throws(() => kitCpuDelta(snapshot(), invalid), /ticks/)
      assert.throws(() => kitCpuDelta(snapshot(null), invalid), /ticks/)
    }
  }
  assert.throws(() => kitCpuDelta({ busyMilliseconds: 0 }, snapshot()), /ticks/)
})
