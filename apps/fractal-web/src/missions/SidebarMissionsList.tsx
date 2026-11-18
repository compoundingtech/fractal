import { markdownPlainText } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdown-source.ts'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'
import type * as Fractal from '@fractal/client/schema'
import { DateTime } from 'effect'

import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, sidebarRowSize, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useMissionsDecode, useNow } from '../data/react.tsx'
import type { Feed, RosterDecode } from '../data/source.ts'
import { DecodeNotice } from '../web/LiveRoster.tsx'
import { MissionFeedNotice, missionsTruncatedText } from './MissionFeedNotice.tsx'
import { missionWord } from './missionWords.ts'
import { foldedInSidebar, missionFaulted, missionReaderTitle } from './presentation.ts'
import { startOfDay, useMissionRows, type MissionRow } from './useMissionRows.ts'

const MissionListRow = ({ row, now, current }: { readonly row: MissionRow; readonly now: number; readonly current: boolean }) => {
  const narrow = React.useContext(NarrowLayoutContext)
  const { mission, presentation } = row
  const word = missionWord(presentation.word)
  const title = missionReaderTitle(mission)
  const facts = [
    presentation.who.kind === 'nobody' ? undefined : presentation.who.label,
    presentation.progress.total > 0 ? `${presentation.progress.done}/${presentation.progress.total}` : undefined,
    presentation.last_progress === null ? undefined : markdownPlainText(presentation.last_progress),
  ].filter(fact => fact !== undefined)
  return <Aria.GridListItem id={mission.id} textValue={title} aria-label={`${title}, ${word.name}`} data-mission-row={mission.id} data-current={current || undefined} {...stylex.props(styles.row, narrow && styles.rowTouch, current && styles.current)} style={{ height: sidebarRowSize }}>
    <span {...stylex.props(styles.line)}>
      <span title={word.name} aria-hidden="true" {...stylex.props(styles.glyph, presentation.word === 'decision' && styles.person, missionFaulted(presentation) && styles.fault)}>{word.glyph}</span>
      <span {...stylex.props(styles.title)}>{title}</span>
      <span {...stylex.props(styles.age)}>{ago(DateTime.formatIso(mission.updated_at), now)}</span>
    </span>
    <span {...stylex.props(styles.facts)}>{facts.length === 0 ? word.name : facts.join(' · ')}</span>
  </Aria.GridListItem>
}

/** The sidebar's Missions tab over the live feeds. */
export const SidebarMissionsList = (props: { readonly current?: string; readonly onOpen: (ref: string) => void }) => {
  const { missions, rows, observedIds } = useMissionRows()
  const decode = useMissionsDecode()
  const now = useNow()
  return <MissionsList {...props} missions={missions} rows={rows} observedIds={observedIds} decode={decode} now={now} />
}

/** Words, who holds them and how far along, in the contract's order. Pressing one focuses it in the Missions screen. */
export const MissionsList = ({ missions, rows, observedIds, decode, now, current, onOpen }: {
  readonly missions: Feed<readonly Fractal.Mission[]>
  readonly rows: readonly MissionRow[]
  readonly observedIds: ReadonlySet<string>
  readonly decode: RosterDecode
  readonly now: number
  readonly current?: string
  readonly onOpen: (ref: string) => void
}) => {
  const [all, setAll] = React.useState(false)
  const today = startOfDay(now)
  // A mission the screen is showing never folds out from under the reader.
  const shown = all ? rows : rows.filter(row => row.mission.id === current || !foldedInSidebar({ ...row, startOfToday: today }))
  const hidden = rows.length - shown.length
  const notice = shown.length === 0 || missions._tag !== 'Observed' ? <MissionFeedNotice missions={missions} total={rows.length} shown={shown.length} /> : null
  return <div data-testid="sidebar-missions" {...stylex.props(styles.root)}>
    <div {...stylex.props(styles.scroller)}>
      {notice ?? <Aria.GridList aria-label="Missions" selectionMode="none" onAction={key => onOpen(String(key))} {...stylex.props(styles.list)}>
        {shown.map(row => <MissionListRow key={row.mission.id} row={row} now={now} current={row.mission.id === current} />)}
      </Aria.GridList>}
      {hidden > 0 || all ? <Aria.Button onPress={() => setAll(!all)} {...stylex.props(styles.more)}>{all ? 'Show fewer' : `Show all (${rows.length})`}</Aria.Button> : null}
    </div>
    {missions._tag === 'Observed' ? <DecodeNotice noun="mission" decode={decode} shown={observedIds} /> : null}
    {missions._tag === 'Observed' && decode.hasMore === true ? <p role="status" data-missions-truncated {...stylex.props(styles.truncated)}>{missionsTruncatedText(missions.value.length)}</p> : null}
  </div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  scroller: { flex: '1 1 0', minHeight: 0, overflowY: 'auto' },
  list: { display: 'flex', flexDirection: 'column', outlineStyle: 'none' },
  row: { display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: s.xs2, boxSizing: 'border-box', flexShrink: 0, minWidth: 0, paddingInline: s.md, cursor: 'pointer', outlineStyle: 'none', fontFamily: t.fontSans, ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  rowTouch: { minHeight: g.touchTarget },
  current: { backgroundColor: surface.rowActive, ':hover': { backgroundColor: surface.rowActive } },
  line: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, color: ink.fg },
  glyph: { flexShrink: 0, width: g.icon, textAlign: 'center', color: ink.fgMuted },
  person: { color: tone.attention },
  fault: { color: tone.dangerFg },
  title: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  age: { flexShrink: 0, color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  facts: { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', paddingInlineStart: `calc(${g.icon} + ${s.sm})`, color: ink.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading },
  more: { alignSelf: 'stretch', width: '100%', minHeight: g.controlMd, paddingInline: s.md, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover } },
  truncated: { flexShrink: 0, margin: 0, padding: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading },
})
