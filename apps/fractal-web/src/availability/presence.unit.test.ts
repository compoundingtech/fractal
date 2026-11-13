// @vitest-environment jsdom
import type { PresenceReport } from '@fractal/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { HANDOFF, HEARTBEAT, IDLE_AFTER, startPresenceReports, type PresencePage, type TabClaim } from './presence.ts'

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
  expect(reports).toEqual([['present', 0], ['idle', 1 + HANDOFF / MINUTE], ['present', 31]])
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

/** Tabs of one origin sharing one presence source (the sink); each has its own visibility and input, and hears the others' claims a moment later. */
const origin = () => {
  const sink: (readonly [string, PresenceReport['presence'], number])[] = []
  const listeners = new Set<(claim: TabClaim) => void>()
  const open = (name: string, shown = true) => {
    let visible = shown
    let onInput = () => {}
    let onVisibility = () => {}
    let own: ((claim: TabClaim) => void) | undefined
    const page: PresencePage = {
      visible: () => visible,
      watch: (input, visibility) => {
        onInput = input
        onVisibility = visibility
        return () => {}
      },
      claim: (claim) => {
        for (const hear of listeners) if (hear !== own) queueMicrotask(() => hear(claim))
      },
      heard: (onClaim) => {
        own = onClaim
        listeners.add(onClaim)
        return () => listeners.delete(onClaim)
      },
    }
    const stop = startPresenceReports(async ({ presence, observed_at }) => {
      sink.push([name, presence, (Date.parse(observed_at) - start) / MINUTE])
      return 'sent'
    }, page)
    const show = (next: boolean) => {
      visible = next
      onVisibility()
    }
    return { input: () => onInput(), show, stop }
  }
  return { sink, open }
}

it('keeps a tab without input from calling the person idle while they work in another tab', async () => {
  const { sink, open } = origin()
  const a = open('a')
  await vi.advanceTimersByTimeAsync(MINUTE)
  // A second window, say on another monitor: the person works there and leaves the first one untouched.
  const b = open('b')
  for (let minute = 2; minute <= 20; minute += 1) {
    await vi.advanceTimersByTimeAsync(MINUTE)
    b.input()
  }
  expect(sink).toEqual([['a', 'present', 0], ['b', 'present', 1], ['b', 'present', 1 + HEARTBEAT / MINUTE]])
  // Leaving the working tab says idle; input back in the first one takes over and says present.
  b.show(false)
  await vi.advanceTimersByTimeAsync(HANDOFF)
  expect(sink.at(-1)).toEqual(['b', 'idle', 20 + HANDOFF / MINUTE])
  a.input()
  await vi.advanceTimersByTimeAsync(IDLE_AFTER - MINUTE)
  expect(sink.slice(3)).toEqual([['b', 'idle', 20 + HANDOFF / MINUTE], ['a', 'present', 20 + HANDOFF / MINUTE]])
  a.stop()
  b.stop()
})

it.each([['the hidden tab', true], ['the shown tab', false]])('ends a tab switch on present when %s hears of it first', async (_, hiddenFirst) => {
  const { sink, open } = origin()
  const a = open('a')
  const b = open('b', false)
  await vi.advanceTimersByTimeAsync(MINUTE)
  if (hiddenFirst) {
    a.show(false)
    b.show(true)
  } else {
    b.show(true)
    a.show(false)
  }
  await vi.advanceTimersByTimeAsync(2 * HANDOFF)
  expect(sink).toEqual([['a', 'present', 0], ['b', 'present', 1]])
  a.stop()
  b.stop()
})
