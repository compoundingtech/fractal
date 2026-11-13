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
