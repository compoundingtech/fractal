import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { ThingRow, ThingCard, compareThings, nextAgeChange, type Observed, type ThingFace } from '@fractal/ui/assistant-ui/things'
import type { DetailsSection } from '@fractal/ui/assistant-ui'
import { useEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import { DetailsReadFailureLine } from '../../../../packages/fractal-ui/src/assistant-ui/details/DetailsReadState.tsx'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useClockDeadline, useNowSnapshot } from '../data/react.tsx'
import { useEntityDrag } from '@fractal/ui/workspace/react'
import { paneKey, subjectOfRef } from '../workspace/subject.ts'

/** The host adds destination behavior; the kit owns every pixel of the row. */
export interface PanelItem {
  readonly key: string
  readonly observed: Observed<ThingFace>
  readonly ref?: string
  readonly hoverCard?: React.ReactNode
}
export const PanelThingRow = ({ item, onOpen }: { readonly item: PanelItem; readonly onOpen: (ref: string, split: boolean) => void }) => {
  const now = useNowSnapshot()
  useClockDeadline(nextAgeChange(item.observed, now))
  return item.ref === undefined ? <ThingRow observed={item.observed} now={now} /> : <OpenablePanelRow item={item} subjectRef={item.ref} now={now} onOpen={onOpen} />
}
const OpenablePanelRow = ({ item, subjectRef, now, onOpen }: { readonly item: PanelItem; readonly subjectRef: string; readonly now: number; readonly onOpen: (ref: string, split: boolean) => void }) => {
  const drag = useEntityDrag({ ref: paneKey(subjectOfRef(subjectRef)) })
  const card = useEntityCard({ kind: 'resource', id: subjectRef })
  return <ThingRow observed={item.observed} now={now} openable={{
    onPointerDown: drag.onPointerDown,
    onClick: event => onOpen(subjectRef, event.metaKey || event.ctrlKey),
    onKeyDown: event => { if (event.key === 'Enter') { event.preventDefault(); onOpen(subjectRef, event.metaKey || event.ctrlKey || event.altKey) } },
  }} hoverCard={item.hoverCard ?? card ?? <ThingCard observed={item.observed} now={now} />} />
}

/** Each reader keeps its recovery focus target even after an empty or refused result. */
export const PanelSection = ({ title, items, onOpen, section }: { readonly title: string; readonly items: readonly PanelItem[]; readonly onOpen: (ref: string, split: boolean) => void; readonly section?: DetailsSection | undefined }) => {
  const [readSeen, setReadSeen] = React.useState(section?.failure !== undefined)
  if (section?.failure !== undefined && !readSeen) setReadSeen(true)
  if (items.length === 0 && section?.pending !== true && !readSeen) return null
  const sorted = [...items].sort((a, b) => title === 'Declared bindings' && a.observed._tag === 'Known' && b.observed._tag === 'Known' ? a.observed.value.title.localeCompare(b.observed.value.title) : compareThings(a.observed, b.observed))
  return <section aria-label={title}>
    <h2 tabIndex={-1} data-details-section-control {...stylex.props(styles.sectionTitle)}>{title}</h2>
    <DetailsReadFailureLine failure={section?.failure} pending={section?.pending ?? false} />
    {section?.pending === true && items.length === 0 ? <p role="status" {...stylex.props(styles.note)}>Loading {title.toLowerCase()}…</p> : null}
    {readSeen && section?.failure === undefined && section?.pending !== true && section?.items._tag === 'Unknown' ? <p {...stylex.props(styles.note)}>{section.unavailableReason}</p> : null}
    <ul {...stylex.props(styles.plain)}>{sorted.map(item => <li key={item.key}><PanelThingRow item={item} onOpen={onOpen} /></li>)}</ul>
  </section>
}
const styles = stylex.create({
  sectionTitle: { margin: 0, marginBottom: s.md, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted },
  note: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgSoft, overflowWrap: 'anywhere' },
  plain: { listStyle: 'none', margin: 0, padding: 0 },
})
