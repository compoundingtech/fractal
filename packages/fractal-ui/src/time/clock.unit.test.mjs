import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const server = await createServer({ configFile: false, root: new URL('../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { formatClock, formatDay, formatDuration, formatRelative, nextCivilChange, nextCountdownChange, nextDurationChange, nextRelativeChange, earliestChange } = await server.ssrLoadModule('/src/time/human-time.ts')
const { syncLine, syncLineNextChange } = await server.ssrLoadModule('/src/assistant-ui/sync-presentation/sync-line.ts')

const zone = { locale: 'en-GB', timeZone: 'Europe/Berlin' }
const base = Date.parse('2026-10-14T21:47:13.250Z')
// Deterministic pseudo-random offsets so failures reproduce.
let seed = 7
const random = () => { seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648; return seed / 2_147_483_648 }

/** No label may change strictly before its computed next instant, sampled at every second and just before `next`. */
const assertStableUntilNext = (name, label, next, at, now) => {
  const until = next(now)
  if (until === undefined) return
  assert.ok(until > now, `${name}: next ${until} must follow now ${now}`)
  const text = label(now)
  const end = Math.min(until, now + 3_600_000)
  for (let time = now; time < end; time += 1_000) assert.equal(label(time), text, `${name} at=${at} now=${now} t=${time} next=${until}`)
  if (until <= now + 3_600_000) assert.equal(label(until - 1), text, `${name} just before next at=${at} now=${now}`)
}

test('relative, elapsed, countdown and civil labels never change before their next instant', () => {
  const ages = [0, 999, 29_999, 30_000, 44_999, 59_999, 60_000, 61_000, 119_999, 3_599_999, 3_600_000, 5_400_000, 86_399_000, 90_000_000, 6 * 86_400_000]
  for (let index = 0; index < 24; index++) ages.push(Math.floor(random() * 3 * 86_400_000))
  for (const age of ages) {
    const now = base + Math.floor(random() * 1_000)
    const at = now - age
    assertStableUntilNext('relative-past', time => formatRelative(at, time, zone), time => nextRelativeChange(at, time), at, now)
    assertStableUntilNext('relative-future', time => formatRelative(now + age + 31_000, time, zone), time => nextRelativeChange(now + age + 31_000, time), now + age + 31_000, now)
    assertStableUntilNext('elapsed-2', time => formatDuration(time - at), time => nextDurationChange(at, time), at, now)
    assertStableUntilNext('elapsed-1', time => formatDuration(time - at, { maxUnits: 1 }), time => nextDurationChange(at, time, { maxUnits: 1 }), at, now)
    assertStableUntilNext('countdown-1', time => formatDuration(now + age - time, { maxUnits: 1 }), time => nextCountdownChange(now + age, time, { maxUnits: 1 }), now + age, now)
    assertStableUntilNext('clock', time => formatClock(at, time, zone), time => nextCivilChange(time), at, now)
    assertStableUntilNext('day', time => formatDay(at, time, zone), time => nextCivilChange(time), at, now)
  }
})

test('minute-granular labels wake exactly at their minute boundary, not every second', () => {
  const at = base - 5 * 60_000 - 12_345
  const now = base
  assert.equal(formatRelative(at, now, zone), '5m ago')
  const next = nextRelativeChange(at, now)
  assert.equal(next, at + 6 * 60_000)
  assert.equal(formatRelative(at, next - 1, zone), '5m ago')
  assert.equal(formatRelative(at, next, zone), '6m ago')
  assert.equal(nextDurationChange(at, now, { maxUnits: 1 }), at + 6 * 60_000)
  assert.equal(nextDurationChange(base - 12_345, now), base - 12_345 + 13_000)
  assert.equal(earliestChange(now, [undefined, now, now - 1, NaN, now + 5, now + 2]), now + 2)
})

test('sync lines never change before syncLineNextChange', () => {
  const observedAt = base
  const statuses = [
    { _tag: 'Connecting', attempt: 1, since: observedAt },
    { _tag: 'Requested', since: observedAt },
    { _tag: 'Progress', stage: 'queued', elapsedMs: 200, reportedAt: observedAt },
    { _tag: 'Progress', stage: 'reading', elapsedMs: 0, reportedAt: observedAt, done: 3, total: 9 },
    { _tag: 'Stale', reason: { _tag: 'Unknown' } },
    { _tag: 'Stale', reason: { _tag: 'Unknown' }, lastLiveAt: observedAt - 50_000 },
    { _tag: 'Stale', reason: { _tag: 'Quiet', lastFrameAt: observedAt - 3_000 } },
    { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: observedAt + 9_500, issue: 'transport lost' } },
    { _tag: 'Stale', reason: { _tag: 'Resync', code: 'internal' } },
  ]
  for (const status of statuses) for (const socket of [false, true]) {
    const text = time => JSON.stringify(syncLine({ status, label: 'conversation', now: time, observedAt, socket, gateway: 'gw' }) ?? null)
    for (const now of [observedAt, observedAt + 399, observedAt + 1_500, observedAt + 4_999, observedAt + 9_999, observedAt + 61_000]) {
      const until = syncLineNextChange({ status, label: 'conversation', now, observedAt, socket })
      const end = Math.min(until ?? now + 120_000, now + 120_000)
      for (let time = now; time < end; time += 50) assert.equal(text(time), text(now), `${status._tag}/${status.reason?._tag ?? status.stage ?? ''} socket=${socket} now=${now} t=${time} next=${until}`)
    }
  }
})
