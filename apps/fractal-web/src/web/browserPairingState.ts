import { PairingHttpError } from '../data/browserPairing.ts'
import type { BrowserPairingPort, BrowserPairingStatus, BrowserPairingStatusReply } from '../data/source.ts'
import type { SendRefusal } from './composerSend.ts'

/** One browser has one pairing: every composer dock reads the same view and shares one poll. */
export type PairingView =
  | { readonly _tag: 'Checking' }
  /** This host does not offer pairing, or its status is unreadable: the composer stays as it was. */
  | { readonly _tag: 'Hidden' }
  /** `sendRefused`: Fractal refused a send for authorization, so the browser is asked to pair whatever its initial grants said. */
  | { readonly _tag: 'Unpaired'; readonly error?: string; readonly sendRefused?: true }
  | { readonly _tag: 'Requesting' }
  | { readonly _tag: 'Pending'; readonly displayCode?: string }
  | { readonly _tag: 'Ended'; readonly outcome: 'denied' | 'expired' }
  | { readonly _tag: 'Paired'; readonly error?: string }
  | { readonly _tag: 'Forgetting' }
  /** The credential changed; the page reloads so grants and sockets use it. */
  | { readonly _tag: 'Reloading' }

export interface PairingStore {
  readonly subscribe: (listener: () => void) => () => void
  readonly snapshot: () => PairingView
  /** A mounted dock: the first one reads status; any one keeps a pending request polled. */
  readonly acquire: () => () => void
  readonly start: () => Promise<void>
  readonly forget: () => Promise<void>
  /** Fractal refused a send for authorization: re-read status, so the server drops a credential it no longer accepts. */
  readonly sendRefused: () => Promise<void>
}

export const pairingPollMs = 2000

const failure = (action: string, error: unknown) => error instanceof PairingHttpError
  ? `Could not ${action} (HTTP ${error.status}). Try again.`
  : `Could not reach Fractal to ${action}. Check the connection and try again.`

const settled = (status: Exclude<BrowserPairingStatus, 'pending'>): PairingView =>
  status === 'paired' ? { _tag: 'Paired' } : status === 'unpaired' ? { _tag: 'Unpaired' } : { _tag: 'Ended', outcome: status }

export const createPairingStore = ({ port, deviceLabel, credentialChanged, pollMs = pairingPollMs }: {
  readonly port: BrowserPairingPort
  readonly deviceLabel: () => string
  /** Grants and the socket were authorized with the previous credential; reloading adopts the new one. */
  readonly credentialChanged: () => void
  readonly pollMs?: number
}): PairingStore => {
  let view: PairingView = { _tag: 'Checking' }
  let checked = false
  // Sticky for the page: initial grants no longer show whether this browser can send.
  let refused = false
  let holders = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  // Each action supersedes replies that were in flight when it began.
  let generation = 0
  const listeners = new Set<() => void>()
  const schedule = () => {
    if (timer !== undefined || holders === 0 || view._tag !== 'Pending') return
    timer = setTimeout(() => { timer = undefined; void poll() }, pollMs)
  }
  const set = (next: PairingView) => {
    view = next
    for (const listener of listeners) listener()
    schedule()
  }
  const changed = () => {
    generation += 1
    set({ _tag: 'Reloading' })
    credentialChanged()
  }
  const poll = async () => {
    const at = generation
    let reply: BrowserPairingStatusReply
    try { reply = await port.status() } catch {
      // A missed poll is not an outcome; the server still decides when the code expires.
      if (at === generation) schedule()
      return
    }
    if (at !== generation || view._tag !== 'Pending') return
    if (reply === undefined) set({ _tag: 'Hidden' })
    else if (reply.status === 'pending') set({ _tag: 'Pending', displayCode: reply.displayCode ?? view.displayCode })
    else if (reply.status === 'paired') changed()
    // A request that vanished without an answer can only be asked again, like an expired one.
    else set(reply.status === 'unpaired' ? { _tag: 'Ended', outcome: 'expired' } : settled(reply.status))
  }
  const check = async () => {
    checked = true
    const at = generation
    let reply: BrowserPairingStatusReply
    try { reply = await port.status() } catch {
      if (at !== generation) return
      // The next mounted dock asks again.
      checked = false
      set({ _tag: 'Hidden' })
      return
    }
    if (at !== generation) return
    if (reply?.credentialChanged === true) { changed(); return }
    set(reply === undefined ? { _tag: 'Hidden' }
      : reply.status === 'pending' ? { _tag: 'Pending', ...(reply.displayCode === undefined ? {} : { displayCode: reply.displayCode }) }
      : reply.status === 'unpaired' && refused ? { _tag: 'Unpaired', sendRefused: true }
      : settled(reply.status))
  }
  return {
    subscribe: (listener) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    snapshot: () => view,
    acquire: () => {
      holders += 1
      if (!checked) void check()
      schedule()
      return () => {
        holders -= 1
        if (holders === 0 && timer !== undefined) {
          clearTimeout(timer)
          timer = undefined
        }
      }
    },
    start: async () => {
      generation += 1
      const at = generation
      set({ _tag: 'Requesting' })
      try {
        const started = await port.start(deviceLabel())
        if (at === generation) set({ _tag: 'Pending', displayCode: started.displayCode })
      } catch (error) {
        if (at === generation) set({ _tag: 'Unpaired', error: failure('request a pairing code', error) })
      }
    },
    forget: async () => {
      generation += 1
      const at = generation
      set({ _tag: 'Forgetting' })
      try {
        await port.forget()
        if (at === generation) changed()
      } catch (error) {
        if (at === generation) set({ _tag: 'Paired', error: failure('forget this browser', error) })
      }
    },
    sendRefused: async () => {
      refused = true
      // A first read still in flight applies the refusal; a request or outcome on screen already offers pairing;
      // a host without pairing stays as it was.
      if (view._tag !== 'Unpaired' && view._tag !== 'Paired') return
      generation += 1
      await check()
    },
  }
}

/** What the person will recognise in Fractal's approval prompt, e.g. "Firefox on Linux". */
export const browserDeviceLabel = (userAgent: string) => {
  const browser = /Edg\//.test(userAgent) ? 'Edge'
    : /Firefox\//.test(userAgent) ? 'Firefox'
    : /Chrome\//.test(userAgent) ? 'Chrome'
    : /Safari\//.test(userAgent) ? 'Safari'
    : 'Web browser'
  // iOS and Android agents also name macOS and Linux; test the specific platform first.
  const platform = /iPhone|iPad|iPod/.test(userAgent) ? 'iOS'
    : /Android/.test(userAgent) ? 'Android'
    : /Mac OS X/.test(userAgent) ? 'macOS'
    : /Windows/.test(userAgent) ? 'Windows'
    : /Linux|CrOS/.test(userAgent) ? 'Linux'
    : undefined
  return platform === undefined ? browser : `${browser} on ${platform}`
}

/** The composer's refusal handler, also re-pairing this browser when a send was refused for authorization. */
export const pairingSendRefusal = (store: PairingStore | undefined, onRefused: (refusal: SendRefusal) => void) => (refusal: SendRefusal) => {
  onRefused(refusal)
  if (refusal.reason === 'ungranted') void store?.sendRefused()
}

const stores = new WeakMap<BrowserPairingPort, PairingStore>()
/** The page's store for its source's port; one per port, however many docks mount. */
export const pairingStoreFor = (port: BrowserPairingPort) => {
  let store = stores.get(port)
  if (store === undefined) {
    store = createPairingStore({ port, deviceLabel: () => browserDeviceLabel(navigator.userAgent), credentialChanged: () => window.location.reload() })
    stores.set(port, store)
  }
  return store
}
