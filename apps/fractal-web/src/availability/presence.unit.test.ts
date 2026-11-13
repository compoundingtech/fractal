// @vitest-environment jsdom
import type { PresenceReport } from '@fractal/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { PresenceAnswer } from './availability-port.ts'
import { SETTLE, startPresenceReports, type PageEvents, type PresencePage, type TabMessage } from './presence.ts'

const start = Date.parse('2026-11-12T09:00:00Z')
const MINUTE = 60_000
/** The daemon's idle threshold for an active device without newer input. */
const IDLE = 10 * MINUTE
const minutes = (at: string) => (Date.parse(at) - start) / MINUTE
const advance = (ms: number) => vi.advanceTimersByTimeAsync(ms)

beforeEach(() => {
  vi.useFakeTimers({ now: start })
})
afterEach(() => {
  vi.useRealTimers()
})

/** A report as the sink records it: the tab that sent it, the raw state, and last_input_at and observed_at in minutes. */
type Sent = readonly [tab: string, state: PresenceReport['state'], lastInput: number | undefined, at: number]

/**
 * One browser pairing: its tabs report into one sink, hear each other's messages a moment later, and queue for one
 * reporter lock that the browser grants a moment after it is free. The sink answers like the daemon: an active
 * device with input inside the idle threshold reports again when it would turn idle, anything else on its next change.
 */
const browser = () => {
  const sink: Sent[] = []
  let refusing = false
  const listeners = new Map<string, (message: TabMessage) => void>()
  const queue: { readonly name: string; readonly onLead: () => void; granted: boolean }[] = []
  const grant = () => {
    const head = queue[0]
    if (head === undefined || head.granted) return
    head.granted = true
    head.onLead()
  }
  const free = (name: string) => {
    const at = queue.findIndex((entry) => entry.name === name)
    if (at >= 0) queue.splice(at, 1)
    queueMicrotask(grant)
  }
  const daemon = ({ state, last_input_at }: PresenceReport): PresenceAnswer => {
    if (refusing) return { _tag: 'Refused' }
    const idleAt = last_input_at === undefined ? undefined : Date.parse(last_input_at) + IDLE
    return { _tag: 'Sent', reportAgainAt: state === 'active' && idleAt !== undefined && idleAt > Date.now() ? new Date(idleAt).toISOString() : null }
  }
  const open = (name: string, focused = true) => {
    let active = focused
    let gone = false
    let events: PageEvents | undefined
    const page: PresencePage = {
      active: () => active,
      watch: (on) => {
        events = on
        return () => {
          events = undefined
        }
      },
      send: (message) => {
        if (gone) return
        for (const [other, hear] of listeners) if (other !== name) queueMicrotask(() => hear(message))
      },
      heard: (onMessage) => {
        listeners.set(name, onMessage)
        return () => listeners.delete(name)
      },
      lead: (onLead) => {
        queue.push({ name, onLead, granted: false })
        queueMicrotask(grant)
        return () => free(name)
      },
    }
    const stop = startPresenceReports(async (report) => {
      if (gone) return { _tag: 'Failed' }
      sink.push([name, report.state, report.last_input_at === undefined ? undefined : minutes(report.last_input_at), minutes(report.observed_at)])
      return daemon(report)
    }, page)
    const leave = () => {
      gone = true
      listeners.delete(name)
    }
    return {
      input: () => events?.input(),
      focus: (next: boolean) => {
        active = next
        events?.change()
      },
      /** The tab closes: `pagehide`, then nothing. */
      close: () => {
        events?.hide()
        leave()
      },
      /** The tab's process dies: no `pagehide` and no goodbye; the browser frees its lock. */
      crash: () => {
        leave()
        free(name)
      },
      stop,
    }
  }
  return { sink, open, refuse: () => { refusing = true } }
}

it('reports once while the person works in tab B and tab A sits untouched', async () => {
  const { sink, open } = browser()
  const a = open('a')
  await advance(MINUTE)
  // A second window, say on another monitor, takes the focus; the person works there.
  a.focus(false)
  const b = open('b')
  for (let minute = 2; minute < 10; minute += 1) {
    await advance(MINUTE)
    b.input()
  }
  expect(sink).toEqual([['a', 'active', 0, 0]])
  a.stop()
  b.stop()
})

it('re-reports at report_again_at with the newer input of another tab', async () => {
  const { sink, open } = browser()
  const a = open('a')
  await advance(MINUTE)
  a.focus(false)
  const b = open('b')
  for (let minute = 2; minute < 10; minute += 1) {
    await advance(MINUTE)
    b.input()
  }
  // Tab A's first report would turn idle at minute 10; it reports again with tab B's input from minute 9.
  await advance(MINUTE)
  expect(sink).toEqual([['a', 'active', 0, 0], ['a', 'active', 9, 10]])
  await advance(IDLE)
  expect(sink.slice(2)).toEqual([['a', 'active', 9, 19]])
  a.stop()
  b.stop()
})

it.each([['the blur', true], ['the focus', false]])('keeps a tab switch active when %s arrives first', async (_, blurFirst) => {
  const { sink, open } = browser()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  if (blurFirst) {
    a.focus(false)
    b.focus(true)
  } else {
    b.focus(true)
    a.focus(false)
  }
  await advance(2 * SETTLE)
  expect(sink).toEqual([['a', 'active', 0, 0]])
  a.stop()
  b.stop()
})

it('reports inactive once no tab is visible and focused, and active when one is again', async () => {
  const { sink, open } = browser()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  a.focus(false)
  await advance(SETTLE)
  expect(sink).toEqual([['a', 'active', 0, 0], ['a', 'inactive', 0, 1 + SETTLE / MINUTE]])
  await advance(4 * MINUTE - SETTLE)
  b.focus(true)
  await advance(0)
  expect(sink.slice(2)).toEqual([['a', 'active', 5, 5]])
  a.stop()
  b.stop()
})

it('reports the first input after an idle stretch once report_again_at was null', async () => {
  const { sink, open } = browser()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  a.focus(false)
  b.focus(true)
  // The person stops at minute 1: the daemon asks again at minute 10 and 11, then turns the browser idle.
  await advance(IDLE)
  expect(sink).toEqual([['a', 'active', 0, 0], ['a', 'active', 1, 10], ['a', 'active', 1, 11]])
  await advance(4 * MINUTE)
  b.input()
  await advance(0)
  b.input()
  await advance(MINUTE)
  expect(sink.slice(3)).toEqual([['a', 'active', 15, 15]])
  a.stop()
  b.stop()
})

it('hands the role on when the reporting tab closes', async () => {
  const { sink, open } = browser()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  a.focus(false)
  b.focus(true)
  await advance(MINUTE)
  a.close()
  await advance(0)
  // Tab B reports at once on taking the role; tab A reports nothing after it closed.
  expect(sink).toEqual([['a', 'active', 0, 0], ['b', 'active', 1, 2]])
  b.focus(false)
  await advance(SETTLE)
  expect(sink.slice(2)).toEqual([['b', 'inactive', 1, 2 + SETTLE / MINUTE]])
  b.stop()
})

it('hands the role on when the reporting tab dies without pagehide, forgetting its state', async () => {
  const { sink, open } = browser()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  // Tab A was the focused one; with it gone no tab is active, which tab B reports at once.
  a.crash()
  await advance(SETTLE)
  expect(sink).toEqual([['a', 'active', 0, 0], ['b', 'inactive', undefined, 1]])
  b.focus(true)
  await advance(0)
  expect(sink.slice(2)).toEqual([['b', 'active', 1 + SETTLE / MINUTE, 1 + SETTLE / MINUTE]])
  b.stop()
})

it('stops every tab once Fractal refuses the pairing', async () => {
  const { sink, open, refuse } = browser()
  refuse()
  const a = open('a')
  const b = open('b', false)
  await advance(MINUTE)
  b.focus(true)
  a.focus(false)
  b.input()
  // Not even the refused tab giving its role up lets another tab report.
  a.stop()
  await advance(30 * MINUTE)
  b.focus(false)
  await advance(30 * MINUTE)
  expect(sink).toEqual([['a', 'active', 0, 0]])
  b.stop()
})
