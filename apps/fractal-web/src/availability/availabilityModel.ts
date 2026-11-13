import type { PersonAvailability } from '@fractal/client'
import { formatClockTime, formatDate, formatDayValue, formatRelative, type HumanTimeZone } from '@fractal/ui/human-time/format'
import type { OwnAvailability } from './availability-port.ts'

const DAY = 86_400_000
const dayIndex = (at: number, zone: HumanTimeZone): number => Date.parse(formatDayValue(at, zone)) / DAY

/** An instant a person plans around: `14:30` today, `tomorrow 09:00`, else its date and clock time. */
export const formatInstant = (at: number, now: number, zone: HumanTimeZone = {}): string => {
  const days = dayIndex(at, zone) - dayIndex(now, zone)
  const clock = formatClockTime(at, zone)
  return days === 0 ? clock : days === 1 ? `tomorrow ${clock}` : days === -1 ? `yesterday ${clock}` : `${formatDate(at, now, zone)} ${clock}`
}

/** How long a change lasts: until changed again, a fixed span, tomorrow at 9:00 local time, or a chosen local time. */
export type UntilPreset = 'open' | '30m' | '1h' | '2h' | 'tomorrow' | 'custom'
export const untilPresets: readonly (readonly [UntilPreset, string])[] = [
  ['open', 'Until changed'], ['30m', '30 min'], ['1h', '1 hour'], ['2h', '2 hours'], ['tomorrow', 'Tomorrow 9:00'], ['custom', 'Custom'],
]
const spans: Readonly<Record<'30m' | '1h' | '2h', number>> = { '30m': 30 * 60_000, '1h': 60 * 60_000, '2h': 120 * 60_000 }

/**
 * The end a preset names at `now`: `open` has none; `tomorrow` is 9:00 local time on the next calendar day;
 * `custom` is a `datetime-local` value read as local time and must lie in the future, else it is `invalid`.
 */
export const untilFor = (preset: UntilPreset, now: Date, custom = ''): Date | 'open' | 'invalid' => {
  switch (preset) {
    case 'open': return 'open'
    case 'tomorrow': return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9, 0, 0, 0)
    case 'custom': {
      // A `datetime-local` value has no offset, so `Date` reads it as local time.
      const at = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?$/.test(custom) ? new Date(custom).getTime() : Number.NaN
      return Number.isFinite(at) && at > now.getTime() ? new Date(at) : 'invalid'
    }
    default: return new Date(now.getTime() + spans[preset])
  }
}

export const statusLabels: Readonly<Record<PersonAvailability['status'], string>> = { available: 'Available', away: 'Away' }
export const interruptionLabels: Readonly<Record<PersonAvailability['interruptions'], string>> = {
  all: 'All asks',
  urgent: 'Urgent asks only',
  none: 'No asks',
}
export const deadlineClockLabels: Readonly<Record<PersonAvailability['deadline_clock'], string>> = {
  run: 'Running',
  'hold-routine': 'Held for routine asks',
  'hold-all': 'Held for every ask',
}
const presenceLabels: Readonly<Record<NonNullable<PersonAvailability['presence']>, string>> = {
  present: 'present',
  idle: 'idle',
  locked: 'locked',
  unknown: 'not reported',
}
const interruptionDetails: Readonly<Record<PersonAvailability['interruptions'], string | undefined>> = {
  all: undefined,
  urgent: 'Only asks with a deadline within an hour reach you.',
  none: 'No ask reaches you.',
}
const deadlineClockDetails: Readonly<Record<PersonAvailability['deadline_clock'], string | undefined>> = {
  run: undefined,
  'hold-routine': 'Routine deadlines pause; urgent asks keep running.',
  'hold-all': 'Every ask’s deadline pauses.',
}

/** One row of the availability summary: what holds now, and the words that qualify it. */
export interface AvailabilityFact {
  readonly label: string
  readonly value: string
  readonly detail?: string
}

const at = (value: string | null | undefined): number | undefined => {
  const parsed = value == null ? Number.NaN : Date.parse(value)
  return Number.isFinite(parsed) ? parsed : undefined
}
const timed = (value: string, parts: readonly (readonly [string, string | null | undefined])[], now: number, zone: HumanTimeZone): string =>
  [value, ...parts.flatMap(([word, time]) => {
    const instant = at(time)
    return instant === undefined ? [] : [`${word} ${formatInstant(instant, now, zone)}`]
  })].join(' · ')

/** The effective availability and note as the summary shows them; pure over a fixed `now`. */
export const availabilityFacts = ({ availability: a, note }: OwnAvailability, now: number, zone: HumanTimeZone = {}): readonly AvailabilityFact[] => {
  const observed = at(a.presence_observed_at)
  const presence = `${presenceLabels[a.presence ?? 'unknown']}${observed === undefined ? '' : ` ${formatRelative(observed, now, zone)}`}`
  const noteExpiry = at(note?.expires_at)
  return [
    {
      label: 'Status',
      value: timed(statusLabels[a.status], [['until', a.status_until]], now, zone),
      detail: a.status_source === 'override' ? `Set by you; presence is ${presence}.` : `Follows your presence: ${presence}.`,
    },
    { label: 'Interruptions', value: timed(interruptionLabels[a.interruptions], [['until', a.interruptions_until]], now, zone), ...detail(interruptionDetails[a.interruptions]) },
    {
      label: 'Deadline clock',
      value: timed(deadlineClockLabels[a.deadline_clock], [['since', a.deadline_clock_since], ['until', a.deadline_clock_until]], now, zone),
      ...detail(deadlineClockDetails[a.deadline_clock]),
    },
    note === undefined
      ? { label: 'Note', value: 'No note' }
      : { label: 'Note', value: note.text, ...detail(noteExpiry === undefined ? undefined : `Expires ${formatInstant(noteExpiry, now, zone)}.`) },
  ]
}
const detail = (text: string | undefined): { readonly detail?: string } => text === undefined ? {} : { detail: text }
