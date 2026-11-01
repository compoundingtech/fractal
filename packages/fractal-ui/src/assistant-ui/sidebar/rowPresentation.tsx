import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { matchPositions, type SidebarAgentRow as Row } from './model'
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

const styles = stylex.create({
  mark: { backgroundColor: surface.rowActive, color: ink.fg, fontWeight: t.weightSemibold },
})
