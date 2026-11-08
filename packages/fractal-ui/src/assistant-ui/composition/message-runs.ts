/** One message-like entry as the run layout sees it; `undefined` marks regular output, which ends any run. */
export interface MessageRunEntry {
  readonly id: string
  /** Who wrote it (a ref, else a label); an entry without a sender never stacks. */
  readonly sender?: string
  readonly recipient?: string
  readonly at?: string
  /** A failed entry keeps its own red edge and never folds under another head. */
  readonly failed?: boolean
}

/** How one entry of a run is drawn. */
export interface MessageRunLayout {
  /** Drawn without a head, 4px under the previous entry of the same sender and recipient. */
  readonly stacked: boolean
  /** The last entry of its stack: receipts and the closing gap belong here. */
  readonly lastInStack: boolean
  /** `→ X` on the head: only when X is not the transcript's own agent. */
  readonly showRecipient: boolean
  /** The head's time is drawn only when its minute differs from the last time shown; otherwise it waits for hover. */
  readonly showTime: boolean
}

const minuteOf = (at: string | undefined): number | undefined => {
  const ms = at === undefined ? NaN : Date.parse(at)
  return Number.isFinite(ms) ? Math.floor(ms / 60_000) : undefined
}

/**
 * Stacks consecutive same-sender, same-recipient messages under one head, drops a recipient that is the transcript's own
 * agent, and shows a head's time only when it changes from the last time shown (`shownAt`, the episode head's time).
 */
export function messageRunLayout(entries: readonly (MessageRunEntry | undefined)[], options: { /** The transcript's own agent, by ref and by name. */ readonly self?: readonly string[]; readonly shownAt?: string | number } = {}): ReadonlyMap<string, MessageRunLayout> {
  const layout = new Map<string, MessageRunLayout>()
  let lastMinute = typeof options.shownAt === 'number' ? Math.floor(options.shownAt / 60_000) : minuteOf(options.shownAt)
  const stacks = (entry: MessageRunEntry | undefined, previous: MessageRunEntry | undefined) =>
    entry !== undefined && previous !== undefined && entry.sender !== undefined && entry.sender === previous.sender && entry.recipient === previous.recipient && entry.failed !== true && previous.failed !== true
  entries.forEach((entry, index) => {
    if (entry === undefined) return
    const stacked = stacks(entry, entries[index - 1])
    const minute = minuteOf(entry.at)
    const showTime = !stacked && minute !== undefined && minute !== lastMinute
    if (showTime) lastMinute = minute
    layout.set(entry.id, {
      stacked,
      lastInStack: !stacks(entries[index + 1], entry),
      showRecipient: !stacked && entry.recipient !== undefined && !(options.self ?? []).includes(entry.recipient),
      showTime,
    })
  })
  return layout
}
