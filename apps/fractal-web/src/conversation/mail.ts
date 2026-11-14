// Who a Fractal message is from, and how machine mail reads as one quiet row.

/** The participant fields of a timeline `message` body this module reads. */
export interface MailParticipants {
  readonly author?: string | undefined
  readonly from?: string | undefined
  readonly to?: string | undefined
}

/**
 * The writer: `author` when the read negotiated `conversation-message-participants.v1`, else the
 * legacy `from`. A runtime-authored wake has author `daemon/runtime` and never a `from`.
 */
export const mailAuthor = (body: MailParticipants): string | undefined => body.author ?? body.from

/**
 * `person`: a person wrote it, the only mail that earns the person's blue bar. `machine`: Fractal
 * itself (a wake, a restart handoff). An unnegotiated read drops a daemon's `from` without naming an
 * author, so a header that names a recipient and no writer is machine too, never a person. `agent`:
 * any other named writer (an agent, a step, an assistant principal).
 */
export type MailOrigin = 'person' | 'machine' | 'agent'
export const mailOrigin = (body: MailParticipants): MailOrigin => {
  const author = mailAuthor(body)
  if (author === undefined || author.startsWith('daemon/')) return 'machine'
  return author.startsWith('person/') ? 'person' : 'agent'
}

const oneLine = (text: string): string => text.replace(/[\s\p{Cc}]+/gu, ' ').trim()
const bounded = (text: string, max: number): string => {
  const chars = [...text]
  return chars.length <= max ? text : `${chars.slice(0, max - 1).join('').trimEnd()}…`
}

/** Row titles and excerpts are one line of at most this many characters; the body stays behind Read message. */
export const wakeTitleMax = 120
export const wakeExcerptMax = 160

/**
 * "Woken by <trigger> · <step>" from a daemon message's title. Known daemon titles name their trigger;
 * any other title is kept after "Fractal", so nothing the daemon said is lost from the row.
 */
export const wakeTitle = (title: string | undefined): string => {
  const line = oneLine(title ?? '')
  const step = /^Mission step ready:\s*(.+)$/i.exec(line)?.[1]
  const label = step !== undefined ? `Woken by mission step · ${step}`
    : /^Prepare for restart\b/i.test(line) ? 'Woken by restart notice'
      : /^Session restarted\b/i.test(line) ? 'Woken by restart handoff'
        : line === '' ? 'Woken by Fractal' : `Woken by Fractal · ${line}`
  return bounded(label, wakeTitleMax)
}

/** The body's first non-blank line, normalized and bounded; Markdown markers stay as written. */
export const wakeExcerpt = (body: string): string => {
  const first = body.split('\n').find(line => line.trim() !== '') ?? ''
  return bounded(oneLine(first), wakeExcerptMax)
}
