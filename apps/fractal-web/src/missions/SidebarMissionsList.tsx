import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'
import { useAtom } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import { MissionGroupHeader, MissionListRow, missionRowAccessibleName } from '@fractal/ui/assistant-ui/shell'

import { Menu, MenuItem, MenuTrigger, Popover } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useMissionsDecode, useNow } from '../data/react.tsx'
import type { Feed, RosterDecode } from '../data/source.ts'
import { defaultMissionsListView, missionsListView, missionsShowAll, type MissionsListView } from '../state/cells/lists.ts'
import { DecodeNotice } from '../web/LiveRoster.tsx'
import { MissionFeedNotice, missionsTruncatedText } from './MissionFeedNotice.tsx'
import { groupMissionRows, missionRowData, visibleMissionRows } from './rowData.ts'
import { startOfDay, useMissionRows, type MissionRow } from './useMissionRows.ts'

/** The sidebar's Missions tab over the live feeds; grouping, sort and the fold are remembered per gateway. */
export const SidebarMissionsList = (props: { readonly current?: string; readonly onOpen: (ref: string) => void }) => {
  const { missions, rows, observedIds } = useMissionRows()
  const decode = useMissionsDecode()
  const now = useNow()
  const source = useDataSource()
  const owner = source.gateway ?? source.mode
  const [view, setView] = useAtom(missionsListView(owner))
  const [showAll, setShowAll] = useAtom(missionsShowAll(owner))
  return <MissionsList {...props} missions={missions} rows={rows} observedIds={observedIds} decode={decode} {...(decode.total === undefined ? {} : { total: decode.total })} now={now} view={view} onViewChange={setView} showAll={showAll} onShowAllChange={setShowAll} />
}

const GROUP_OPTIONS: ReadonlyArray<{ readonly id: MissionsListView['group']; readonly label: string }> = [{ id: 'state', label: 'State' }, { id: 'project', label: 'Project' }, { id: 'agent', label: 'Agent' }]
const SORT_OPTIONS: ReadonlyArray<{ readonly id: MissionsListView['sort']; readonly label: string }> = [{ id: 'activity', label: 'Recent activity' }, { id: 'eta', label: 'ETA' }, { id: 'progress', label: 'Progress' }]
const firstKey = (keys: Aria.Selection): string | undefined => keys === 'all' ? undefined : [...keys].map(String)[0]

/** Group and sort, one menu: the trigger names the current grouping so the list explains itself. */
const ViewMenu = ({ view, onViewChange }: { readonly view: MissionsListView; readonly onViewChange: (view: MissionsListView) => void }) =>
  <MenuTrigger>
    <Aria.Button aria-label={`Group by ${view.group}, sort by ${SORT_OPTIONS.find(option => option.id === view.sort)!.label}`} data-missions-view={`${view.group}/${view.sort}`} {...stylex.props(styles.viewButton)}>
      <Icon name="sliders" size={12} />
      <span>By {view.group}</span>
    </Aria.Button>
    <Popover placement="bottom end">
      <Menu aria-label="Group and sort missions">
        <Aria.MenuSection selectionMode="single" selectedKeys={[`group:${view.group}`]} onSelectionChange={keys => { const key = firstKey(keys); if (key !== undefined) onViewChange({ ...view, group: key.slice('group:'.length) as MissionsListView['group'] }) }}>
          <Aria.Header {...stylex.props(styles.menuHeader)}>Group by</Aria.Header>
          {GROUP_OPTIONS.map(option => <MenuItem key={option.id} id={`group:${option.id}`}>{option.label}</MenuItem>)}
        </Aria.MenuSection>
        <Aria.MenuSection selectionMode="single" selectedKeys={[`sort:${view.sort}`]} onSelectionChange={keys => { const key = firstKey(keys); if (key !== undefined) onViewChange({ ...view, sort: key.slice('sort:'.length) as MissionsListView['sort'] }) }}>
          <Aria.Header {...stylex.props(styles.menuHeader)}>Sort by</Aria.Header>
          {SORT_OPTIONS.map(option => <MenuItem key={option.id} id={`sort:${option.id}`}>{option.label}</MenuItem>)}
        </Aria.MenuSection>
      </Menu>
    </Popover>
  </MenuTrigger>

/** Missions grouped (by state, project or agent) and sorted. Pressing one opens it; the host decides where. */
export const MissionsList = ({ missions, rows, observedIds, decode, now, current, onOpen, view = defaultMissionsListView, onViewChange, showAll: showAllProp, onShowAllChange, total }: {
  readonly missions: Feed<readonly Fractal.Mission[]>
  readonly rows: readonly MissionRow[]
  readonly observedIds: ReadonlySet<string>
  readonly decode: RosterDecode
  readonly now: number
  readonly current?: string
  readonly onOpen: (ref: string) => void
  readonly view?: MissionsListView
  readonly onViewChange?: (view: MissionsListView) => void
  readonly showAll?: boolean
  readonly onShowAllChange?: (all: boolean) => void
  /** The server's pre-truncation count, when it states one. */
  readonly total?: number
}) => {
  // Stories and tests without a store keep the choice locally.
  const [localView, setLocalView] = React.useState(view)
  const [localAll, setLocalAll] = React.useState(false)
  const activeView = onViewChange === undefined ? localView : view
  const setView = onViewChange ?? setLocalView
  const showAll = showAllProp ?? localAll
  const setShowAll = onShowAllChange ?? setLocalAll
  // A mission the workspace is showing never folds out from under the reader.
  const shown = visibleMissionRows({ rows, showAll, current, startOfToday: startOfDay(now) })
  const hidden = rows.length - shown.length
  const groups = groupMissionRows(shown, activeView)
  const collapsed = new Set(activeView.collapsed)
  const toggle = (key: string) => setView({ ...activeView, collapsed: collapsed.has(key) ? activeView.collapsed.filter(item => item !== key) : [...activeView.collapsed, key] })
  const notice = shown.length === 0 || missions._tag !== 'Observed' ? <MissionFeedNotice missions={missions} total={rows.length} shown={shown.length} /> : null
  const truncated = missions._tag === 'Observed' && decode.hasMore === true
  const stale = missions._tag === 'Observed' && missions.freshness === 'stale'
  return <div data-testid="sidebar-missions" {...stylex.props(styles.root)}>
    {missions._tag === 'Observed' && rows.length > 0 ? <div {...stylex.props(styles.toolbar)}>
      <span data-missions-count {...stylex.props(styles.count)}>{total !== undefined && total > rows.length ? `${rows.length} of ${total} missions` : hidden > 0 ? `${shown.length} of ${rows.length} shown` : `${rows.length} missions`}</span>
      {stale ? <span data-missions-stale {...stylex.props(styles.stale)}>last observed</span> : null}
      <ViewMenu view={activeView} onViewChange={setView} />
    </div> : null}
    <div {...stylex.props(styles.scroller)}>
      {notice ?? groups.map(group => {
        const closed = collapsed.has(group.key)
        return <section key={group.key} aria-label={group.label} data-mission-group-key={group.key}>
          <MissionGroupHeader label={group.label} count={group.rows.length} {...(group.tone === undefined ? {} : { tone: group.tone })} collapsed={closed} onToggle={() => toggle(group.key)} />
          {closed ? null : <Aria.GridList aria-label={group.label} selectionMode="none" onAction={key => onOpen(String(key))} {...stylex.props(styles.list)}>
            {group.rows.map(row => {
              const data = missionRowData(row)
              return <Aria.GridListItem key={row.mission.id} id={row.mission.id} textValue={data.title} aria-label={missionRowAccessibleName(data, now)} data-current={row.mission.id === current || undefined} {...stylex.props(styles.item)}>
                <MissionListRow mission={data} now={now} active={row.mission.id === current} frozen={stale} />
              </Aria.GridListItem>
            })}
          </Aria.GridList>}
        </section>
      })}
      {hidden > 0 || showAll ? <Aria.Button onPress={() => setShowAll(!showAll)} {...stylex.props(styles.more)}>{showAll ? 'Show fewer' : `Show all (${rows.length})`}</Aria.Button> : null}
    </div>
    {missions._tag === 'Observed' ? <DecodeNotice noun="mission" decode={decode} shown={observedIds} /> : null}
    {truncated ? <p role="status" data-missions-truncated {...stylex.props(styles.truncated)}>{missionsTruncatedText(missions.value.length, total)}</p> : null}
  </div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  toolbar: { display: 'flex', alignItems: 'center', gap: s.sm, flexShrink: 0, height: g.controlMd, paddingInlineStart: s.md, paddingInlineEnd: s.xs, fontFamily: t.fontSans, fontSize: t.denseSize, color: ink.fgSoft },
  count: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  stale: { flexShrink: 0, color: ink.fgMuted },
  viewButton: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  menuHeader: { paddingInline: s.md, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontFamily: t.fontSans },
  scroller: { flex: '1 1 0', minHeight: 0, overflowY: 'auto' },
  list: { display: 'flex', flexDirection: 'column', outlineStyle: 'none' },
  item: { display: 'block', outlineStyle: 'none' },
  more: { alignSelf: 'stretch', width: '100%', minHeight: g.controlMd, paddingInline: s.md, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover } },
  truncated: { flexShrink: 0, margin: 0, padding: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading },
})
