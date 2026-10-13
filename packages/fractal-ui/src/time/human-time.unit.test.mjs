import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import stylex from '@stylexjs/unplugin'

const server = await createServer({ configFile: false, root: new URL('../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { formatDuration, formatRelative, formatDay, formatDayValue, formatExactDay, formatClock, formatClockTime, formatExact, instant } = await server.ssrLoadModule('/src/time/human-time.ts')
const zone = { locale: 'en-GB', timeZone: 'Europe/Berlin' }
const at = Date.parse
const now = at('2026-10-14T14:00:00Z')

test('duration ladder and compact precision', () => {
  for (const [ms, text] of [[45000, '45s'], [120000, '2m'], [125000, '2m 5s'], [3900000, '1h 5m'], [259200000, '3d'], [273600000, '3d 4h'], [999, '0s'], [-1, '0s'], [NaN, '—'], [Infinity, '—']]) assert.equal(formatDuration(ms), text)
  assert.equal(formatDuration(125000, { maxUnits: 1 }), '2m')
})
test('relative ladder with a fixed clock', () => {
  for (const [value, text] of [[now - 10000, 'just now'], [now - 44000, 'just now'], [now - 45000, 'just now'], [now - 59999, 'just now'], [now - 60000, '1m ago'], [now - 119999, '1m ago'], [now - 300000, '5m ago'], [now - 10800000, '3h ago'], [at('2026-10-13T12:02:00Z'), 'yesterday 14:02'], [at('2026-10-12T12:02:00Z'), 'Mon 14:02'], [at('2026-10-06T12:02:00Z'), '6 Oct'], [at('2025-10-06T12:02:00Z'), '6 Oct 2025'], [now + 300000, 'in 5m'], [now + 10800000, 'in 3h'], [now + 187200000, 'in 2d 4h']]) assert.equal(formatRelative(value, now, zone), text)
  for (let age = 0; age < 3_600_000; age += 1000) assert.doesNotMatch(formatRelative(now - age, now, zone), /^0m/)
})
test('civil-day ladder with a fixed clock and date-only exact text', () => {
  for (const [value, text] of [[at('2026-10-13T22:00:00Z'), 'today'], [at('2026-10-12T22:00:00Z'), 'yesterday'], [at('2026-10-11T22:00:00Z'), 'Mon'], [at('2026-10-07T22:00:00Z'), 'Thu'], [at('2026-10-06T22:00:00Z'), '7 Oct'], [at('2026-10-05T22:00:00Z'), '6 Oct'], [at('2025-10-05T22:00:00Z'), '6 Oct 2025']]) assert.equal(formatDay(value, now, zone), text)
  assert.equal(formatExactDay(at('2026-10-11T22:00:00Z'), zone), 'Mon, 12 Oct 2026')
  assert.equal(formatDay(NaN, now, zone), '—')
  assert.equal(formatExactDay(NaN, zone), 'Date unreported')
})
test('day machine values follow the formatting zone rather than UTC', async () => {
  const value = at('2026-10-11T22:00:00Z')
  assert.equal(formatDay(value, at('2026-10-12T12:00:00Z'), zone), 'today')
  assert.equal(formatDayValue(value, zone), '2026-10-12')
  assert.equal(formatDayValue(value, { ...zone, timeZone: 'America/New_York' }), '2026-10-11')
  const { HumanTime, HumanClockProvider, fixedClock } = await server.ssrLoadModule('/src/time/HumanTime.tsx')
  const html = renderToStaticMarkup(React.createElement(HumanClockProvider, { clock: fixedClock(at('2026-10-12T12:00:00Z')), zone }, React.createElement(HumanTime, { at: value, kind: 'day' })))
  assert.match(html, /dateTime="2026-10-12"/)
  assert.doesNotMatch(html, /dateTime="[^"]*T/)
})
test('civil days across Berlin DST and the New York day boundary', () => {
  assert.equal(formatDay(at('2026-03-28T23:00:00Z'), at('2026-03-29T21:30:00Z'), zone), 'today')
  assert.equal(formatDay(at('2026-03-28T23:00:00Z'), at('2026-03-29T22:30:00Z'), zone), 'yesterday')
  assert.equal(formatDay(at('2026-10-13T23:30:00Z'), at('2026-10-14T05:30:00Z'), zone), 'today')
  assert.equal(formatDay(at('2026-10-13T23:30:00Z'), at('2026-10-14T05:30:00Z'), { locale: 'en-GB', timeZone: 'America/New_York' }), 'yesterday')
})
test('calendar yesterday and real elapsed hours across both Berlin DST transitions', () => {
  assert.equal(formatRelative(at('2026-03-28T13:02:00Z'), at('2026-03-29T12:30:00Z'), zone), 'yesterday 14:02')
  assert.equal(formatRelative(at('2026-10-24T12:02:00Z'), at('2026-10-25T13:30:00Z'), zone), 'yesterday 14:02')
  assert.equal(formatRelative(at('2026-03-29T00:30:00Z'), at('2026-03-29T03:30:00Z'), zone), '3h ago')
  assert.equal(formatRelative(at('2026-10-25T00:30:00Z'), at('2026-10-25T03:30:00Z'), zone), '3h ago')
})
test('viewer locale clock and exact localized timestamp', () => {
  const value = at('2026-10-12T12:02:00Z')
  assert.equal(formatClockTime(value, zone), '14:02')
  assert.equal(formatClock(value, value, zone), '14:02')
  assert.equal(formatClock(value, now, zone), 'Mon 14:02')
  assert.equal(formatClockTime(value, { ...zone, locale: 'en-US' }), '2:02 PM')
  assert.equal(formatExact(value, { ...zone, locale: 'de-DE' }), 'Mo., 12. Okt. 2026, 14:02:00 MESZ')
})
test('viewer zone changes the calendar day', () => {
  const value = at('2026-10-13T23:30:00Z'), clock = at('2026-10-14T05:30:00Z')
  assert.equal(formatRelative(value, clock, zone), '6h ago')
  assert.equal(formatRelative(value, clock, { locale: 'en-GB', timeZone: 'America/New_York' }), 'yesterday 19:30')
})
test('instant rejects missing and unusable input while preserving machine instants', () => {
  assert.equal(instant('2026-10-12T12:02:00Z'), at('2026-10-12T12:02:00Z'))
  assert.equal(instant(42), 42)
  for (const value of [undefined, null, 'invalid', NaN, Infinity]) assert.equal(instant(value), undefined)
})
