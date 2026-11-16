import type { PresenceReport } from '@fractal/client'
import type { AvailabilityPort } from './availability-port.ts'

/**
 * How long the browser's tabs get to tell each other. A drop to `inactive` waits this long, because a tab switch
 * blurs one tab before it focuses the next; a new reporter forgets the tabs that have not answered within it.
 */
export const SETTLE = 1_000
/** A tab tells the others of newer input at most this often, so the reported `last_input_at` lags by at most this much. */
export const SHARE_INPUT = 5_000
/** A failed report is sent again after this long, doubling with each further failure up to `RETRY_MAX`. */
export const RETRY = 60_000
export const RETRY_MAX = 15 * 60_000
/** The soonest a report repeats at `report_again_at`, so a skewed clock cannot turn it into a loop. */
export const SOONEST = 5_000
const INPUT = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart'] as const
const NAME = 'fractal.presence'

/** One tab's part of the browser's state: whether it is visible and focused, and its newest input. */
export interface TabState {
  readonly tab: string
  readonly active: boolean
  readonly lastInput: number | undefined
}

/** What the tabs of one origin tell each other over the channel. */
export type TabMessage =
  /** A tab's state, and whether it reports; `ask` asks every other tab to answer with its own. */
  | { readonly kind: 'state'; readonly state: TabState; readonly leads: boolean; readonly ask: boolean }
  /** `tab` reports from now on; every other tab answers with its state. */
  | { readonly kind: 'lead'; readonly tab: string }
  /** `tab` left: it closed, navigated away or entered the back-forward cache. */
  | { readonly kind: 'bye'; readonly tab: string }
  /** Fractal refused the pairing's reports; every tab stops. */
  | { readonly kind: 'refused' }

export interface PageEvents {
  readonly input: () => void
  /** The tab's visibility or focus changed. */
  readonly change: () => void
  /** The page is going away (`pagehide`). */
  readonly hide: () => void
  /** The page came back from the back-forward cache (`pageshow` with `persisted`). */
  readonly show: () => void
}

/** What a reporter sees of its tab and of the origin's other tabs. */
export interface PresencePage {
  /** Whether the tab is visible and focused. */
  readonly active: () => boolean
  /** Calls back on the tab's events; returns the unsubscribe. */
  readonly watch: (events: PageEvents) => () => void
  /** Tells the origin's other tabs. */
  readonly send: (message: TabMessage) => void
  /** Calls back with the other tabs' messages; returns the unsubscribe. */
  readonly heard: (onMessage: (message: TabMessage) => void) => () => void
  /**
   * Queues for the browser's one reporter role and calls `onLead` once this tab holds it. The browser hands the
   * role on when its holder releases it or dies. Returns the release, which also leaves the queue.
   */
  readonly lead: (onLead: () => void) => () => void
}

/**
 * This browser tab: the reporter role is the `fractal.presence` Web Lock, and the tabs talk over a
 * BroadcastChannel. Without Web Locks every tab reports on its own.
 */
export const browserPage = (): PresencePage => {
  const channel = typeof BroadcastChannel === 'undefined' ? undefined : new BroadcastChannel(NAME)
  return {
    active: () => document.visibilityState === 'visible' && document.hasFocus(),
    watch: ({ input, change, hide, show }) => {
      const shown = (event: PageTransitionEvent) => {
        if (event.persisted) show()
      }
      for (const type of INPUT) window.addEventListener(type, input, { capture: true, passive: true })
      document.addEventListener('visibilitychange', change)
      window.addEventListener('focus', change)
      window.addEventListener('blur', change)
      window.addEventListener('pagehide', hide)
      window.addEventListener('pageshow', shown)
      return () => {
        for (const type of INPUT) window.removeEventListener(type, input, true)
        document.removeEventListener('visibilitychange', change)
        window.removeEventListener('focus', change)
        window.removeEventListener('blur', change)
        window.removeEventListener('pagehide', hide)
        window.removeEventListener('pageshow', shown)
      }
    },
    send: (message) => channel?.postMessage(message),
    heard: (onMessage) => {
      if (channel === undefined) return () => {}
      const listener = (event: MessageEvent<TabMessage>) => onMessage(event.data)
      channel.addEventListener('message', listener)
      return () => {
        channel.removeEventListener('message', listener)
        channel.close()
      }
    },
    lead: (onLead) => {
      if (!('locks' in navigator)) {
        onLead()
        return () => {}
      }
      const queue = new AbortController()
      const held = Promise.withResolvers<void>()
      // Aborting a request that was never granted rejects it; that is the release working.
      navigator.locks.request(NAME, { signal: queue.signal }, () => {
        onLead()
        return held.promise
      }).catch(() => {})
      return () => {
        queue.abort()
        held.resolve()
      }
    },
  }
}

type Trigger = 'change' | 'input' | 'again' | 'settled'
type Reported = { readonly state: PresenceReport['state']; readonly lastInput: number | undefined }
const iso = (at: number) => new Date(at).toISOString()

/**
 * Reports the browser's raw presence state for the person. The daemon keys presence by the pairing, which every tab
 * of this browser shares, so exactly one tab reports: the holder of the reporter role (`PresencePage.lead`). It
 * reports the browser's state, `active` while any tab is visible and focused and `inactive` otherwise (a browser
 * cannot see a locked screen), with the newest input of any tab; the tabs tell each other their state and input
 * over the channel. It reports every change of that state, though a drop to `inactive` waits `SETTLE` so a tab
 * switch stays `active`. It reports again at the answer's `report_again_at` with the newest input, and after a null
 * one on the first newer input while active. A tab taking the role reports at once and asks the others for their
 * state, forgetting the previous holder and any tab that does not answer. A failed report is retried with
 * exponential backoff. A refusal (403: no `write.presence` grant or no person; 404: no such route) stops every tab
 * and releases the role, so a tab loaded later, say after pairing again, takes it up. Returns the stop, which hands
 * the role on.
 */
export const startPresenceReports = (report: AvailabilityPort['reportPresence'], page: PresencePage = browserPage()): (() => void) => {
  const tab = Math.random().toString(36).slice(2)
  const others = new Map<string, TabState>()
  let active = page.active()
  // Opening or switching to a tab is the person's input.
  let lastInput = active ? Date.now() : undefined
  let shared: number | undefined
  let leaderTab: string | undefined
  let release: (() => void) | undefined
  let stopped = false
  // The reporter's own state; a new term forgets a report still in flight.
  let leading = false
  let term = 0
  let reported: Reported | undefined
  let againAt: number | null | undefined
  let inFlight = false
  let failures = 0
  let answered: Set<string> | undefined
  let againTimer: number | undefined
  let settleTimer: number | undefined
  let rollTimer: number | undefined

  const tell = (ask = false) => {
    shared = lastInput
    page.send({ kind: 'state', state: { tab, active, lastInput }, leads: leading, ask })
  }
  const aggregate = (): Reported => {
    let anyActive = active
    let newest = lastInput
    for (const other of others.values()) {
      anyActive ||= other.active
      if (other.lastInput !== undefined && (newest === undefined || other.lastInput > newest)) newest = other.lastInput
    }
    return { state: anyActive ? 'active' : 'inactive', lastInput: newest }
  }
  const clearTimers = () => {
    window.clearTimeout(againTimer)
    window.clearTimeout(settleTimer)
    againTimer = undefined
    settleTimer = undefined
  }
  const send = () => {
    if (inFlight) return
    clearTimers()
    const now = aggregate()
    reported = now
    inFlight = true
    const sentIn = term
    void report({ state: now.state, ...(now.lastInput === undefined ? {} : { last_input_at: iso(now.lastInput) }), observed_at: iso(Date.now()) }).then((answer) => {
      if (sentIn !== term) return
      inFlight = false
      if (answer._tag === 'Refused') {
        page.send({ kind: 'refused' })
        return refuse()
      }
      if (answer._tag === 'Failed') {
        againAt = Date.now() + Math.min(RETRY * 2 ** failures, RETRY_MAX)
        failures += 1
      } else {
        failures = 0
        const next = answer.reportAgainAt === null ? null : Date.parse(answer.reportAgainAt)
        againAt = next === null || Number.isFinite(next) ? next : Date.now() + RETRY
      }
      if (againAt !== null) againTimer = window.setTimeout(() => evaluate('again'), Math.max(againAt - Date.now(), SOONEST))
      // Input can arrive while the old numeric deadline still suppresses an input-triggered send.
      // Always reconcile the latest state against the answer's schedule, not just a queued send.
      evaluate('input')
    })
  }
  const evaluate = (trigger: Trigger) => {
    if (!leading) return
    const now = aggregate()
    if (reported !== undefined && now.state !== reported.state) {
      if (now.state === 'active' || trigger === 'settled') return send()
      settleTimer ??= window.setTimeout(() => {
        settleTimer = undefined
        evaluate('settled')
      }, SETTLE)
      return
    }
    window.clearTimeout(settleTimer)
    settleTimer = undefined
    const newerInput = now.lastInput !== undefined && (reported?.lastInput === undefined || now.lastInput > reported.lastInput)
    if (reported === undefined || trigger === 'again' || (trigger === 'input' && againAt === null && now.state === 'active' && newerInput)) send()
  }
  const takeLead = () => {
    if (stopped || leading) return
    leading = true
    term += 1
    reported = undefined
    againAt = undefined
    inFlight = false
    failures = 0
    // The previous reporter gave the role up because it left or died; the others answer the call below.
    if (leaderTab !== undefined) others.delete(leaderTab)
    leaderTab = tab
    answered = new Set()
    rollTimer = window.setTimeout(() => {
      for (const other of others.keys()) if (!answered?.has(other)) others.delete(other)
      answered = undefined
      evaluate('change')
    }, SETTLE)
    page.send({ kind: 'lead', tab })
    evaluate('change')
  }
  const onInput = () => {
    lastInput = Date.now()
    if (shared === undefined || lastInput - shared >= SHARE_INPUT) tell()
    evaluate('input')
  }
  const onChange = () => {
    const next = page.active()
    if (next === active) return
    active = next
    if (active) lastInput = Date.now()
    tell()
    evaluate(active ? 'input' : 'change')
  }
  const onMessage = (message: TabMessage) => {
    switch (message.kind) {
      case 'state':
        others.set(message.state.tab, message.state)
        answered?.add(message.state.tab)
        if (message.leads) leaderTab = message.state.tab
        if (message.ask) tell()
        return evaluate('input')
      case 'lead':
        leaderTab = message.tab
        return tell()
      case 'bye':
        others.delete(message.tab)
        return evaluate('change')
      case 'refused':
        return refuse()
    }
  }
  // Gives the role up and tells the others; the browser hands the role to the next tab in line.
  const leave = () => {
    if (release === undefined) return
    leading = false
    term += 1
    clearTimers()
    window.clearTimeout(rollTimer)
    page.send({ kind: 'bye', tab })
    release()
    release = undefined
  }
  // Back from the back-forward cache: the others' states are stale, so ask them again and queue for the role.
  const rejoin = () => {
    if (stopped || release !== undefined) return
    others.clear()
    active = page.active()
    if (active) lastInput = Date.now()
    tell(true)
    release = page.lead(takeLead)
  }
  // Stops this tab for good.
  const halt = () => {
    stopped = true
    leading = false
    term += 1
    clearTimers()
    window.clearTimeout(rollTimer)
    unwatch()
    unhear()
  }
  // Fractal refused the pairing: this tab stops and gives up the role or its place in line.
  const refuse = () => {
    halt()
    release?.()
    release = undefined
  }
  const unwatch = page.watch({ input: onInput, change: onChange, hide: leave, show: rejoin })
  const unhear = page.heard(onMessage)
  tell(true)
  release = page.lead(takeLead)
  return () => {
    if (!stopped) {
      leave()
      halt()
    }
    release?.()
    release = undefined
  }
}

/**
 * Starts the page's presence reports once Fractal says this session may report: its capabilities grant
 * `write.presence` and its actor is a person. Returns the stop, which also cancels a start still waiting for that answer.
 */
export const reportPresenceWhenGranted = (port: Pick<AvailabilityPort, 'mayReportPresence' | 'reportPresence'>, page: () => PresencePage = browserPage): (() => void) => {
  let stop: (() => void) | undefined
  let stopped = false
  void port.mayReportPresence().then((granted) => {
    if (granted && !stopped) stop = startPresenceReports(port.reportPresence, page())
  })
  return () => {
    stopped = true
    stop?.()
  }
}
