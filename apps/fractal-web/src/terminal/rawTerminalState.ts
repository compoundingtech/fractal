/**
 * Which terminal path a pane shows, the chip that names it, and the grid an element can hold.
 * The raw PTY stream is the interactive path; the projected screen remains as a read-only
 * fallback when the stream is refused, keeps failing, or the source does not offer it.
 */

import { ptyFallback, type PtyOutput, type PtySession, type PtySize, type PtyState, type PtyTarget } from '@st3/sdk/effect'
import type * as Effect from 'effect/Effect'
import type * as Scope from 'effect/Scope'

/**
 * A source's raw PTY stream: one scoped session per pane, bound to one terminal incarnation.
 * Closing the scope detaches. Absent when the source has no raw stream.
 */
export interface RawTerminalPort {
  readonly open: (options: {
    readonly target: PtyTarget
    readonly size: PtySize
    readonly onOutput: (output: PtyOutput) => void
  }) => Effect.Effect<PtySession, never, Scope.Scope>
}

/** The pane's terminal path. */
export type TerminalPresentation =
  /** The raw session owns the screen and the keyboard. */
  | { readonly _tag: 'Raw' }
  /** The projected screen is shown; nothing typed reaches the terminal. */
  | { readonly _tag: 'ReadOnly'; readonly reason: string }

/** What the raw path can say about itself. */
export type RawPreparation =
  | { readonly _tag: 'Unsupported' }
  | { readonly _tag: 'Preparing' }
  | { readonly _tag: 'Unprepared'; readonly message: string }
  | { readonly _tag: 'Session'; readonly state: PtyState }

/** Fallback selection: everything that is not a usable raw session reads the projected screen. */
export const terminalPresentation = (raw: RawPreparation): TerminalPresentation => {
  if (raw._tag === 'Unsupported')
    return { _tag: 'ReadOnly', reason: 'This gateway does not offer a live terminal stream.' }
  if (raw._tag === 'Unprepared') return { _tag: 'ReadOnly', reason: raw.message }
  if (raw._tag === 'Preparing') return { _tag: 'Raw' }
  const fallback = ptyFallback(raw.state)
  if (fallback === undefined) return { _tag: 'Raw' }
  return {
    _tag: 'ReadOnly',
    reason:
      fallback.reason === 'refused'
        ? 'This device may watch this terminal but not type into it.'
        : `The live terminal stream failed (${fallback.message}). Showing the read-only screen.`,
  }
}

/** The pane's state chip. */
export type TerminalChip = { readonly label: string; readonly tone: 'live' | 'pending' | 'readonly' | 'ended' }

export const terminalChip = ({
  raw,
  presentation,
  typing,
}: {
  readonly raw: RawPreparation
  readonly presentation: TerminalPresentation
  readonly typing: boolean
}): TerminalChip => {
  if (presentation._tag === 'ReadOnly') return { label: 'Read-only', tone: 'readonly' }
  if (raw._tag !== 'Session') return { label: 'Connecting…', tone: 'pending' }
  switch (raw.state._tag) {
    case 'Live':
      return typing ? { label: 'Typing', tone: 'live' } : { label: 'Click to type', tone: 'live' }
    case 'Connecting':
      return raw.state.attempt === 1 ? { label: 'Connecting…', tone: 'pending' } : { label: 'Reconnecting…', tone: 'pending' }
    case 'Reconnecting':
    case 'Stale':
      return { label: 'Reconnecting…', tone: 'pending' }
    case 'Exited':
      return { label: `Exited (${raw.state.code})`, tone: 'ended' }
    default:
      return { label: 'Closed', tone: 'ended' }
  }
}

/**
 * The grid that fits an element's content box. `undefined` until the font has a measured cell,
 * so an unmeasured element never shrinks the shared PTY.
 */
export const gridFor = ({
  width,
  height,
  cellWidth,
  cellHeight,
}: {
  readonly width: number
  readonly height: number
  readonly cellWidth: number
  readonly cellHeight: number
}): PtySize | undefined => {
  if (!(cellWidth > 0) || !(cellHeight > 0) || !(width > 0) || !(height > 0)) return undefined
  return {
    cols: Math.max(2, Math.min(65535, Math.floor(width / cellWidth))),
    rows: Math.max(1, Math.min(65535, Math.floor(height / cellHeight))),
  }
}

/** Element resizes settle for this long before one RESIZE frame is sent. */
export const resizeSettleMillis = 120

/** A pending RESIZE frame that settles after the element stops changing size. */
export interface ResizeDebouncer {
  readonly observe: (size: PtySize) => void
  readonly dispose: () => void
}

/**
 * Debounce element sizes into RESIZE frames: only the last size of a burst is sent, and a size
 * equal to the last one sent is not sent again. The server applies min-wins over writable clients.
 */
export const makeResizeDebouncer = ({
  send,
  settleMillis = resizeSettleMillis,
  initial,
}: {
  readonly send: (size: PtySize) => void
  readonly settleMillis?: number
  /** The size the session attached with. */
  readonly initial?: PtySize | undefined
}): ResizeDebouncer => {
  let sent = initial
  let pending: PtySize | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  const flush = () => {
    timer = undefined
    const size = pending
    pending = undefined
    if (size === undefined || (sent !== undefined && sent.cols === size.cols && sent.rows === size.rows)) return
    sent = size
    send(size)
  }
  return {
    observe: (size: PtySize) => {
      pending = size
      clearTimeout(timer)
      timer = setTimeout(flush, settleMillis)
    },
    dispose: () => {
      clearTimeout(timer)
      pending = undefined
    },
  }
}
