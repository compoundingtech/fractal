import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { matchPositions, usageScopeLabel, type SidebarActions, type SidebarAgentRow as Row } from './model'
import { surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/** Search and accessible names belong to the eager roster, not its optional overlays. */
/**
 * The host's sidebar search query. Only `Highlight` reads it, so a keystroke re-renders the
 * highlighted titles while memoized rows with unchanged props skip rendering.
 */
export const SidebarQueryContext = React.createContext('')
/** Fuzzy highlighting uses the upstream matcher, not a second search implementation. An explicit `query` takes precedence over `SidebarQueryContext`. */
export function Highlight({ value, query }: { readonly value: string; readonly query?: string }) {
  const contextQuery = React.useContext(SidebarQueryContext)
  const matched = matchPositions({ query: query ?? contextQuery, text: value })
  if (matched.size === 0) return <>{value}</>
  return <>{Array.from(value, (letter, index) => matched.has(index) ? <mark key={index} {...stylex.props(styles.mark)}>{letter}</mark> : letter)}</>
}
/** Concise label-in-name for the tree host, which cannot attach a row description. */
export const sidebarRowAccessibleName = (row: Row): string => {
  const parts = [row.title, row.statusLabel]
  if (row.parentRef === undefined && row.subagentObservation !== undefined && row.subagentObservation._tag !== 'NotReported' && row.subagentCount !== 0) {
    parts.splice(1, 0, row.subagentCount === undefined ? '? subagents' : `${row.subagentCount} ${row.subagentCount === 1 ? 'subagent' : 'subagents'}`)
  }
  if (row.missionContext?._tag === 'Mission') {
    const { slug, progress, step } = row.missionContext
    parts.push(`Mission ${slug}`)
    if (progress._tag === 'Known') parts.push(`${progress.done} of ${progress.total} steps`)
    if (step._tag === 'Step') parts.push(`Step ${step.name}`)
    if (step._tag === 'Waiting') parts.push(`Waiting for you: ${step.handle === undefined ? '' : `${step.handle} `}${step.question}`)
    if (row.missionInfo !== undefined && row.missionInfo._tag !== 'Live') parts.push('mission info may be out of date')
  }
  if (row.missionContext?._tag === 'Unavailable') parts.push('mission info unavailable')
  return parts.join(', ')
}
export const tokenCount = Intl.NumberFormat('en', { notation: 'compact', maximumSignificantDigits: 2 })
export const spendCount = Intl.NumberFormat('en', { style: 'currency', currency: 'USD', notation: 'compact', maximumSignificantDigits: 2 })
export function usageText(usage: Extract<Row['usage'], { _tag: 'Known' }>): string {
  return `${usage.usd === undefined ? '' : `$${usage.usd} / `}${usage.tokens} tokens; ${usageScopeLabel(usage.scope).toLowerCase()}`
}
export const sidebarRowTitle = (row: Row): string => row.title
/** Cause comes from the projection, never from matching a displayed harness name. */
export const sidebarSubagentObservationText = (observation: Row['subagentObservation']): string => {
  if (observation?._tag === 'NotReported') {
    return observation.reason._tag === 'Harness' ? 'Not reported by this harness' : 'Not reported by this daemon version'
  }
  return observation?._tag === 'Unavailable' ? 'Not reported yet' : 'Subagent observations unavailable'
}
/** Opening selects the host workspace (with its parent, for subagents) and notifies the host. */
export const openRow = (row: Row, actions: SidebarActions | undefined, onOpen: ((row: Row) => void) | undefined) => { actions?.select?.(row.id, row.parentRef?.replace(/^agent\//, '')); onOpen?.(row) }

const styles = stylex.create({
  mark: { backgroundColor: surface.rowActive, color: ink.fg, fontWeight: t.weightSemibold },
})
