import type { PtyState } from '@st3/sdk/effect'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  gridFor,
  makeResizeDebouncer,
  resizeSettleMillis,
  terminalChip,
  terminalPresentation,
  type RawPreparation,
} from './rawTerminalState.ts'

const session = (state: PtyState): RawPreparation => ({ _tag: 'Session', state })
const chip = (raw: RawPreparation, typing = false) => terminalChip({ raw, presentation: terminalPresentation(raw), typing }).label

describe('fallback selection', () => {
  it('keeps the raw path while it connects, is live, reconnects or re-resolves a stale fence', () => {
    for (const state of [
      { _tag: 'Connecting', attempt: 1 },
      { _tag: 'Live', rows: 24, cols: 80 },
      { _tag: 'Reconnecting', attempt: 2, delayMs: 1000, issue: 'lost' },
      { _tag: 'Stale', message: 'stale fence' },
      { _tag: 'Exited', code: 0 },
    ] satisfies PtyState[])
      expect(terminalPresentation(session(state))).toEqual({ _tag: 'Raw' })
    expect(terminalPresentation({ _tag: 'Preparing' })).toEqual({ _tag: 'Raw' })
  })

  it('falls back to the read-only projected screen when ATTACH is refused', () => {
    expect(terminalPresentation(session({ _tag: 'Refused', message: 'forbidden', status: 403, code: 'forbidden' }))).toEqual({
      _tag: 'ReadOnly',
      reason: 'This device may watch this terminal but not type into it.',
    })
  })

  it('falls back when the socket keeps failing, the emulator cannot load, or the source has no stream', () => {
    expect(terminalPresentation(session({ _tag: 'Failed', message: 'PTY socket error' }))).toEqual({
      _tag: 'ReadOnly',
      reason: 'The live terminal stream failed (PTY socket error). Showing the read-only screen.',
    })
    expect(terminalPresentation({ _tag: 'Unprepared', message: 'The terminal emulator could not be loaded' })).toEqual({
      _tag: 'ReadOnly',
      reason: 'The terminal emulator could not be loaded',
    })
    expect(terminalPresentation({ _tag: 'Unsupported' })._tag).toBe('ReadOnly')
  })

  it('names the state in the chip: typing, read-only, reconnecting', () => {
    expect(chip(session({ _tag: 'Live', rows: 24, cols: 80 }), true)).toBe('Typing')
    expect(chip(session({ _tag: 'Live', rows: 24, cols: 80 }))).toBe('Click to type')
    expect(chip(session({ _tag: 'Refused', message: 'forbidden' }), true)).toBe('Read-only')
    expect(chip({ _tag: 'Unsupported' })).toBe('Read-only')
    expect(chip(session({ _tag: 'Reconnecting', attempt: 1, delayMs: 500, issue: 'lost' }))).toBe('Reconnecting…')
    expect(chip(session({ _tag: 'Connecting', attempt: 2 }))).toBe('Reconnecting…')
    expect(chip(session({ _tag: 'Connecting', attempt: 1 }))).toBe('Connecting…')
    expect(chip(session({ _tag: 'Exited', code: 130 }))).toBe('Exited (130)')
  })
})

describe('element geometry', () => {
  it('fits whole cells and waits for a measured font', () => {
    expect(gridFor({ width: 800, height: 400, cellWidth: 7.8, cellHeight: 17 })).toEqual({ cols: 102, rows: 23 })
    expect(gridFor({ width: 800, height: 400, cellWidth: 0, cellHeight: 17 })).toBeUndefined()
    expect(gridFor({ width: 0, height: 400, cellWidth: 7.8, cellHeight: 17 })).toBeUndefined()
    expect(gridFor({ width: 5, height: 5, cellWidth: 7.8, cellHeight: 17 })).toEqual({ cols: 2, rows: 1 })
  })
})

describe('resize debounce', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('sends one RESIZE for the settled size of a burst and none for an unchanged size', () => {
    const sent: unknown[] = []
    const debouncer = makeResizeDebouncer({ send: (size) => sent.push(size) })
    debouncer.observe({ cols: 100, rows: 30 })
    vi.advanceTimersByTime(resizeSettleMillis - 1)
    debouncer.observe({ cols: 90, rows: 30 })
    vi.advanceTimersByTime(resizeSettleMillis - 1)
    debouncer.observe({ cols: 80, rows: 24 })
    expect(sent).toEqual([])
    vi.advanceTimersByTime(resizeSettleMillis)
    expect(sent).toEqual([{ cols: 80, rows: 24 }])
    debouncer.observe({ cols: 80, rows: 24 })
    vi.advanceTimersByTime(resizeSettleMillis)
    expect(sent).toHaveLength(1)
    debouncer.observe({ cols: 81, rows: 24 })
    vi.advanceTimersByTime(resizeSettleMillis)
    expect(sent).toEqual([{ cols: 80, rows: 24 }, { cols: 81, rows: 24 }])
  })

  it('skips the attach size and sends nothing after disposal', () => {
    const sent: unknown[] = []
    const debouncer = makeResizeDebouncer({ send: (size) => sent.push(size), initial: { cols: 80, rows: 24 } })
    debouncer.observe({ cols: 80, rows: 24 })
    vi.advanceTimersByTime(resizeSettleMillis)
    debouncer.observe({ cols: 120, rows: 40 })
    debouncer.dispose()
    vi.advanceTimersByTime(resizeSettleMillis)
    expect(sent).toEqual([])
  })
})
