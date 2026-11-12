// @vitest-environment jsdom
import type { PresenceReport } from '@fractal/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { HEARTBEAT, IDLE_AFTER, startPresenceReports } from './presence.ts'

const start = Date.parse('2026-11-12T09:00:00Z')
const MINUTE = 60_000
let visibility: DocumentVisibilityState = 'visible'
beforeEach(() => {
  vi.useFakeTimers({ now: start })
  visibility = 'visible'
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility })
})
afterEach(() => {
  vi.useRealTimers()
  Reflect.deleteProperty(document, 'visibilityState')
})

const reporting = (answer: 'sent' | 'refused' = 'sent') => {
  const reports: (readonly [PresenceReport['presence'], number])[] = []
  const stop = startPresenceReports(async ({ presence, observed_at }) => {
    reports.push([presence, (Date.parse(observed_at) - start) / MINUTE])
    return answer
  })
  return { reports, stop }
}
const setVisibility = (next: DocumentVisibilityState) => {
  visibility = next
  document.dispatchEvent(new Event('visibilitychange'))
}

it('reports present at once, keeps it fresh below the staleness window, and idles without input', async () => {
  const { reports, stop } = reporting()
  // Input every minute keeps the page present; only the heartbeat reports again.
  for (let minute = 1; minute <= 12; minute += 1) {
    await vi.advanceTimersByTimeAsync(MINUTE)
    window.dispatchEvent(new Event('pointermove'))
  }
  expect(reports).toEqual([['present', 0], ['present', HEARTBEAT / MINUTE]])
  await vi.advanceTimersByTimeAsync(IDLE_AFTER)
  expect(reports.at(-1)).toEqual(['idle', 12 + IDLE_AFTER / MINUTE])
  window.dispatchEvent(new Event('keydown'))
  expect(reports.at(-1)).toEqual(['present', 17])
  stop()
})

it('reports idle when the page is hidden and present when it returns, then stops for good', async () => {
  const { reports, stop } = reporting()
  await vi.advanceTimersByTimeAsync(MINUTE)
  setVisibility('hidden')
  await vi.advanceTimersByTimeAsync(30 * MINUTE)
  setVisibility('visible')
  expect(reports).toEqual([['present', 0], ['idle', 1], ['present', 31]])
  stop()
  setVisibility('hidden')
  await vi.advanceTimersByTimeAsync(30 * MINUTE)
  expect(reports).toHaveLength(3)
})

it('stops reporting once Fractal refuses this session', async () => {
  const { reports } = reporting('refused')
  await vi.advanceTimersByTimeAsync(0)
  setVisibility('hidden')
  setVisibility('visible')
  await vi.advanceTimersByTimeAsync(2 * HEARTBEAT)
  expect(reports).toEqual([['present', 0]])
})
