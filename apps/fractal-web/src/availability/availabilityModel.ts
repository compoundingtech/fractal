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

/** Looks a response word up in a table of the words this build knows; Fractal may send words it does not know yet. */
const lookup = <V>(table: Readonly<Record<string, V>>, word: string): V | undefined => Object.hasOwn(table, word) ? table[word] : undefined
/** A response word as the person reads it: its label when known, else the word itself. */
const label = (labels: Readonly<Record<string, string>>, word: string): string => lookup(labels, word) ?? word

const statusLabels: Readonly<Record<string, string>> = { available: 'Available', away: 'Away' }
const interruptionLabels: Readonly<Record<string, string>> = {
  all: 'All asks',
  urgent: 'Urgent asks only',
  none: 'No asks',
}
const deadlineClockLabels: Readonly<Record<string, string>> = {
  run: 'Running',
  'hold-routine': 'Held for routine asks',
  'hold-all': 'Held for every ask',
}
const presenceLabels: Readonly<Record<string, string>> = {
  present: 'present',
  idle: 'idle',
  locked: 'locked',
  unreported: 'not reported',
}
const interruptionDetails: Readonly<Record<string, string>> = {
  urgent: 'Only asks with priority critical reach you.',
  none: 'No ask reaches you.',
}
const deadlineClockDetails: Readonly<Record<string, string>> = {
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

/**
 * The effective availability and note as the summary shows them; pure over a fixed `now`. A word this build
 * does not know shows as itself. With `status_source: unreported` no device reports presence, so the status is
 * the default: available, and agents ask as usual.
 */
export const availabilityFacts = ({ availability: a, note }: OwnAvailability, now: number, zone: HumanTimeZone = {}): readonly AvailabilityFact[] => {
  const observed = a.presence === null || a.presence === 'unreported' ? undefined : at(a.presence_observed_at)
  const presence = `${label(presenceLabels, a.presence ?? 'unreported')}${observed === undefined ? '' : ` ${formatRelative(observed, now, zone)}`}`
  const noteExpiry = at(note?.expires_at)
  const unreported = a.status_source === 'unreported'
  return [
    {
      label: 'Status',
      value: timed(unreported ? 'Available' : label(statusLabels, a.status), [['until', a.status_until]], now, zone),
      detail: a.status_source === 'override' ? `Set by you; presence is ${presence}.`
        : a.status_source === 'presence' ? `Follows your presence: ${presence}.`
          : unreported ? 'No device is reporting your presence, so agents ask as usual.'
            : `Comes from ${a.status_source}; presence is ${presence}.`,
    },
    { label: 'Interruptions', value: timed(label(interruptionLabels, a.interruptions), [['until', a.interruptions_until]], now, zone), ...detail(lookup(interruptionDetails, a.interruptions)) },
    {
      label: 'Deadline clock',
      value: timed(label(deadlineClockLabels, a.deadline_clock), [['since', a.deadline_clock_since], ['until', a.deadline_clock_until]], now, zone),
      ...detail(lookup(deadlineClockDetails, a.deadline_clock)),
    },
    note === undefined
      ? { label: 'Note', value: 'No note' }
      : { label: 'Note', value: note.text, ...detail(noteExpiry === undefined ? undefined : `Expires ${formatInstant(noteExpiry, now, zone)}.`) },
  ]
}
const detail = (text: string | undefined): { readonly detail?: string } => text === undefined ? {} : { detail: text }
