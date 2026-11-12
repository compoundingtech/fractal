import type { PresenceReport } from '@fractal/client'
import type { AvailabilityPort } from './availability-port.ts'

const MINUTE = 60_000
/** A visible page without input for this long reads as idle. */
export const IDLE_AFTER = 5 * MINUTE
/** Below the daemon's 15-minute staleness and above its 5-minute window for an unchanged report. */
export const HEARTBEAT = 10 * MINUTE
const INPUT = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart', 'focus'] as const

/**
 * Reports this page's presence for the person: `present` while the page is visible and had input within
 * `IDLE_AFTER`, else `idle` (a browser cannot see a locked screen). It reports each change, a page becoming
 * visible, and every `HEARTBEAT` while present, so a present report never goes stale. A refusal stops it.
 * Returns the stop.
 */
export const startPresenceReports = (report: AvailabilityPort['reportPresence']): (() => void) => {
  let lastInput = Date.now()
  let reported: PresenceReport['presence'] | undefined
  let reportedAt = 0
  let timer: number | undefined
  let stopped = false
  const evaluate = (force = false) => {
    if (stopped) return
    window.clearTimeout(timer)
    const now = Date.now()
    const presence = document.visibilityState === 'visible' && now - lastInput < IDLE_AFTER ? 'present' : 'idle'
    if (force || presence !== reported || (presence === 'present' && now - reportedAt >= HEARTBEAT)) {
      reported = presence
      reportedAt = now
      void report({ presence, observed_at: new Date(now).toISOString() }).then((result) => {
        if (result === 'refused') stop()
      })
    }
    if (presence === 'present') timer = window.setTimeout(() => evaluate(), Math.min(lastInput + IDLE_AFTER, reportedAt + HEARTBEAT) - now)
  }
  // Input while present only moves the idle deadline; the pending timer reads it when it fires.
  const onInput = () => {
    lastInput = Date.now()
    if (reported !== 'present') evaluate()
  }
  // Other tabs of the same pairing share this source, so returning to this one says present again.
  const onVisibility = () => {
    const visible = document.visibilityState === 'visible'
    if (visible) lastInput = Date.now()
    evaluate(visible)
  }
  const stop = () => {
    stopped = true
    window.clearTimeout(timer)
    for (const type of INPUT) window.removeEventListener(type, onInput, true)
    document.removeEventListener('visibilitychange', onVisibility)
  }
  for (const type of INPUT) window.addEventListener(type, onInput, { capture: true, passive: true })
  document.addEventListener('visibilitychange', onVisibility)
  evaluate()
  return stop
}
