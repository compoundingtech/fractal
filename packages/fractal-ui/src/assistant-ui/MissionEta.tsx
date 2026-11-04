/**
 * The served mission ETA: the p25–p75 remaining wall duration exactly as the run summary reports it at `as_of`. The
 * client never counts it down; the label states the served range and explains how old it is, relative to the host's now.
 * Absent or null ETA renders nothing and reserves no space; hosts hide it for finished runs.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria-components'
import { useClockLabel } from '../time/clock'
import { formatExact, formatRelative } from '../time/human-time'

/** Client-v0 `MissionRunSummary.eta`; `as_of` is the publication cut in Unix ms, not a finish time. */
export type MissionEta = {
  readonly remaining_low_ms: number
  readonly remaining_high_ms: number
  readonly basis: { readonly method: 'per-step' | 'critical-path'; readonly samples: number }
  readonly as_of: number
}

const SECOND = 1_000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const unitOf = (ms: number) => ms >= HOUR ? { size: HOUR, name: 'hr' } : ms >= MINUTE ? { size: MINUTE, name: 'min' } : { size: SECOND, name: 'sec' }
const amount = (ms: number) => { const unit = unitOf(ms); return { value: Math.round(ms / unit.size), unit: unit.name } }

/**
 * `~12–18 min left`; `~12 min left` when both bounds read the same; `~40 sec–2 min left` when the bounds need different
 * units; a served zero reads `~0 sec left`. Each bound takes the largest of sec/min/hr it fills and rounds to it. Null only
 * when there is no ETA.
 */
export const formatMissionEta = (eta: MissionEta | null | undefined): string | null => {
  if (eta === null || eta === undefined) return null
  const low = amount(eta.remaining_low_ms)
  const high = amount(eta.remaining_high_ms)
  if (low.unit !== high.unit) return `~${low.value} ${low.unit}–${high.value} ${high.unit} left`
  return low.value === high.value ? `~${high.value} ${high.unit} left` : `~${low.value}–${high.value} ${high.unit} left`
}

const methodLabel = { 'per-step': 'Per-step', 'critical-path': 'Critical-path' } as const
const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? '' : 's'}`

/** `Estimated 12–18 min left, served as of <servedAs>. Critical-path basis, 12 retained samples.` Null only without an ETA. */
export const describeMissionEta = (eta: MissionEta | null | undefined, servedAs: string): string | null => {
  const text = formatMissionEta(eta)
  if (text === null || eta === null || eta === undefined) return null
  return `Estimated ${text.slice(1)}, served as of ${servedAs}. ${methodLabel[eta.basis.method]} basis, ${plural(eta.basis.samples, 'retained sample')}.`
}

/**
 * The compact served range, described by its basis and its exact `as_of` with a relative age. `now` fixes the age;
 * omitted, the age follows the context clock. Either way the range itself never changes until the server serves a new one.
 * `variant="description"` renders only the visually hidden description: hosts too narrow for the range (the sidebar row)
 * keep it for assistive tech without taking any layout space.
 */
export const MissionEtaLabel = React.memo(function MissionEtaLabel({ eta, now, variant = 'inline' }: { readonly eta: MissionEta | null | undefined; readonly now?: number; readonly variant?: 'inline' | 'description' }) {
  const text = formatMissionEta(eta)
  const asOf = eta?.as_of ?? 0
  const age = useClockLabel((current, zone) => text === null ? '' : `${formatExact(asOf, zone)}, ${formatRelative(asOf, current, zone)}`, now)
  const description = describeMissionEta(eta, age)
  if (text === null || description === null || eta === null || eta === undefined) return null
  if (variant === 'description') return <VisuallyHidden elementType="span" data-mission-eta={text} data-mission-eta-method={eta.basis.method}>{description}</VisuallyHidden>
  return <span data-mission-eta={text} data-mission-eta-method={eta.basis.method} title={description} {...stylex.props(styles.eta)}>
    <span aria-hidden="true">{text}</span>
    <VisuallyHidden elementType="span">{description}</VisuallyHidden>
  </span>
})

const styles = stylex.create({
  eta: { flexShrink: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
})
