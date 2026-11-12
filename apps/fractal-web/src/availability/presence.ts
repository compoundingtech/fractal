import type { PresenceReport } from '@fractal/client'
import type { AvailabilityPort } from './availability-port.ts'

const MINUTE = 60_000
/** A visible page without input for this long reads as idle. */
export const IDLE_AFTER = 5 * MINUTE
/** Below the daemon's 15-minute staleness and above its 5-minute window for an unchanged report. */
export const HEARTBEAT = 10 * MINUTE
/** A reporting tab that is hidden waits this long before saying idle, so a tab being shown can take over first. */
export const HANDOFF = 1_000
const INPUT = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart', 'focus'] as const
const CHANNEL = 'fractal.presence'

/** A tab saying that, from `at` on, it reports for the pairing; the newest claim wins and `tab` breaks ties. */
export interface TabClaim {
  readonly tab: number
  readonly at: number
}

/** What a reporter sees of its tab: whether it is visible, its input and visibility changes, and the origin's other tabs. */
export interface PresencePage {
  readonly visible: () => boolean
  /** Calls back on input and on visibility changes; returns the unsubscribe. */
  readonly watch: (onInput: () => void, onVisibility: () => void) => () => void
  /** Tells the other tabs of this origin about a claim. */
  readonly claim: (claim: TabClaim) => void
  /** Calls back with the other tabs' claims; returns the unsubscribe. */
  readonly heard: (onClaim: (claim: TabClaim) => void) => () => void
}

/** This browser tab; the origin's other tabs hear its claims over a BroadcastChannel. */
export const browserPage = (): PresencePage => {
  const channel = typeof BroadcastChannel === 'undefined' ? undefined : new BroadcastChannel(CHANNEL)
  return {
    visible: () => document.visibilityState === 'visible',
    watch: (onInput, onVisibility) => {
      for (const type of INPUT) window.addEventListener(type, onInput, { capture: true, passive: true })
      document.addEventListener('visibilitychange', onVisibility)
      return () => {
        for (const type of INPUT) window.removeEventListener(type, onInput, true)
        document.removeEventListener('visibilitychange', onVisibility)
      }
    },
    claim: (claim) => channel?.postMessage(claim),
    heard: (onClaim) => {
      if (channel === undefined) return () => {}
      channel.onmessage = (event: MessageEvent<TabClaim>) => onClaim(event.data)
      return () => channel.close()
    },
  }
}

/**
 * Reports the person's presence from this tab: `present` while the tab is visible and had input within
 * `IDLE_AFTER`, else `idle` (a browser cannot see a locked screen). Every tab of the pairing shares one presence
 * source where the newest report wins, so only the tab that last had input or was last shown reports; it tells the
 * others, which fall quiet. The reporting tab reports each change, becoming visible, and every `HEARTBEAT` while
 * present, so a present report never goes stale. A tab opened in the background reports nothing until shown.
 * A refusal stops it. Returns the stop.
 */
export const startPresenceReports = (report: AvailabilityPort['reportPresence'], page: PresencePage = browserPage()): (() => void) => {
  const tab = Math.random()
  let claimedAt: number | undefined
  let lastInput = Date.now()
  let reported: PresenceReport['presence'] | undefined
  let reportedAt = 0
  let timer: number | undefined
  let stopped = false
  const evaluate = (force = false) => {
    if (stopped || claimedAt === undefined) return
    window.clearTimeout(timer)
    const now = Date.now()
    const presence = page.visible() && now - lastInput < IDLE_AFTER ? 'present' : 'idle'
    if (force || presence !== reported || (presence === 'present' && now - reportedAt >= HEARTBEAT)) {
      reported = presence
      reportedAt = now
      void report({ presence, observed_at: new Date(now).toISOString() }).then((result) => {
        if (result === 'refused') stop()
      })
    }
    if (presence === 'present') timer = window.setTimeout(() => evaluate(), Math.min(lastInput + IDLE_AFTER, reportedAt + HEARTBEAT) - now)
  }
  const claim = () => {
    if (claimedAt !== undefined) return
    claimedAt = Date.now()
    reported = undefined
    page.claim({ tab, at: claimedAt })
  }
  // A newer claim means another tab now speaks for the person; this one stops until it has input or is shown again.
  const onClaim = (other: TabClaim) => {
    if (claimedAt === undefined || other.at < claimedAt || (other.at === claimedAt && other.tab < tab)) return
    claimedAt = undefined
    reported = undefined
    window.clearTimeout(timer)
  }
  // Input while present only moves the idle deadline; the pending timer reads it when it fires.
  const onInput = () => {
    lastInput = Date.now()
    if (page.visible()) claim()
    if (reported !== 'present') evaluate()
  }
  const onVisibility = () => {
    if (page.visible()) {
      lastInput = Date.now()
      claim()
      evaluate(true)
    } else if (claimedAt !== undefined) {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => evaluate(), HANDOFF)
    }
  }
  const unwatch = page.watch(onInput, onVisibility)
  const unhear = page.heard(onClaim)
  const stop = () => {
    stopped = true
    window.clearTimeout(timer)
    unwatch()
    unhear()
  }
  if (page.visible()) claim()
  evaluate()
  return stop
}
