import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t, interactionVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { useVisibleAttention } from '../conversation/alerts/useAlertAnswers.ts'
import { useAttentionDecode, useFleet, useMissions, useNow } from '../data/react.tsx'
import { sidebarRow } from './sidebarRow.ts'
import { missionDisplayTitle } from '../missions/missionTitle.ts'
import { credentialExpiredMessage, decodedRoster, type Feed, type RosterDecode } from '../data/source.ts'
import { alertsTabRows, type AlertsTabRow } from './alertsTab.ts'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { isAdHocMissionSlug, statuses, type AgentStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { HoverCardSurface, useHoverCardIntent } from '../../../../packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx'
import { useEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import type { EntityRef } from '../../../../packages/fractal-ui/src/assistant-ui/entity/entity.ts'
import { DecodeNotice } from './LiveRoster.tsx'

// The sidebar's Alerts tab: every open alert across conversations in Fractal's tier order, one SR-2 row
// each, opened in the main area. It reads the attention collection the conversation cards read, so the tab
// marker, this list and each conversation agree.

/** What the host knows about the agents and missions an alert names; anything it does not know is left out, never guessed. */
export interface AlertContext {
  readonly agent?: (ref: string) => { readonly name: string; readonly status: AgentStatus; readonly needsYou?: boolean } | undefined
  readonly missionTitle?: (id: string) => string | undefined
}
const noContext: AlertContext = {}
/** A mission's readable name: the host's title, else a named slug; a content hash is never shown. */
const missionName = (id: string, context: AlertContext): string | undefined => {
  const title = context.missionTitle?.(id)
  if (title !== undefined) return title
  const slug = id.split('/').at(-1) ?? id
  return isAdHocMissionSlug(slug) ? undefined : slug
}

const GLYPH_WORDS: Record<AlertsTabRow['glyph'], string> = { '?': 'asks a question', '◆': 'needs you', '!': 'fault' }

/** The Alerts tab over the active source: its attention feed, without answers on their way, and its clock. */
export const AlertsTab = ({ onOpen, selected, onSelect }: {
  readonly onOpen: (ref: string) => void
  readonly selected?: string
  readonly onSelect?: (id: string) => void
}): React.ReactNode => {
  const fleet = useFleet()
  const missions = useMissions()
  const context = React.useMemo((): AlertContext => {
    const agents = new Map(fleet._tag === 'Observed' ? fleet.value.agents.map(agent => [agent.ref, agent] as const) : [])
    const titles = new Map<string, string>(missions._tag === 'Observed' ? missions.value.map(view => [view.mission.id, view.mission.title] as const) : [])
    return {
      // The row's own glyph already says it needs you; the agent's mark says what the agent is doing (red only if it failed).
      agent: ref => { const agent = agents.get(ref); return agent === undefined ? undefined : { name: agent.name, status: sidebarRow({ agent, stale: false }).status } },
      missionTitle: id => { const title = titles.get(id); return title === undefined ? undefined : missionDisplayTitle({ title, work: undefined }) },
    }
  }, [fleet, missions])
  return <AlertsList feed={useVisibleAttention()} decode={useAttentionDecode()} now={useNow()} onOpen={onOpen} context={context} {...(selected === undefined ? {} : { selected })} {...(onSelect === undefined ? {} : { onSelect })} />
}

/**
 * The list itself, from a feed: waiting, unavailable, stale and empty each say so; unknown is never shown as none.
 * It is its own bounded scroll region, so it fills a `minHeight: 0` column and every row stays reachable.
 */
export const AlertsList = ({ feed, decode = decodedRoster, now, onOpen, selected, onSelect, context = noContext }: {
  readonly feed: Feed<readonly Native.Attention[]>
  /** Names and status for the agents and missions the alerts point at. */
  readonly context?: AlertContext
  readonly decode?: RosterDecode
  readonly now: number
  /** Opens an agent conversation or a mission by ref. */
  readonly onOpen: (ref: string) => void
  /** The alert the main area shows. */
  readonly selected?: string
  /** Opens an alert's details in the main area. */
  readonly onSelect?: (id: string) => void
}): React.ReactNode => {
  const rows = React.useMemo(() => (feed._tag === 'Observed' ? alertsTabRows(feed.value, now) : []), [feed, now])
  const shownIds = React.useMemo(() => new Set(feed._tag === 'Observed' ? feed.value.map((row) => row.id) : []), [feed])
  const region = React.useRef<HTMLDivElement>(null)
  const message = React.useRef<HTMLParagraphElement>(null)
  // The row that last held focus: its own controls, its answer, or its menu. It stays recorded when focus falls to
  // the page because a control inside the row unmounted (a row that left the feed), and is forgotten once the
  // person moves focus or points anywhere else, so a later feed update never takes focus back from other content.
  const focused = React.useRef<string | undefined>(undefined)
  const shown = React.useRef<readonly string[]>([])
  React.useEffect(() => {
    const away = (event: PointerEvent) => {
      if (!(event.target instanceof Node) || region.current?.contains(event.target) !== true) focused.current = undefined
    }
    document.addEventListener('pointerdown', away, true)
    return () => document.removeEventListener('pointerdown', away, true)
  }, [])
  // An answered alert leaves the feed and its row unmounts. If focus was in that row and nothing has taken it since,
  // it moves to the row that took its place (else the one before it), else the list's message.
  React.useLayoutEffect(() => {
    const before = shown.current
    const ids = rows.map((row) => row.id)
    shown.current = ids
    const lost = focused.current
    if (lost === undefined || ids.includes(lost)) return
    focused.current = undefined
    const active = document.activeElement
    if (active !== null && active !== document.body) return
    const at = before.indexOf(lost)
    const next = [...before.slice(at + 1), ...before.slice(0, Math.max(at, 0)).reverse()].find((id) => ids.includes(id)) ?? ids[0]
    const row = next === undefined ? undefined : [...(region.current?.querySelectorAll<HTMLElement>('[data-alert-id]') ?? [])].find((each) => each.dataset.alertId === next)
    const target = row === undefined ? message.current : row.querySelector<HTMLElement>('button[data-alert-open]')
    target?.focus()
  }, [rows])
  return (
    <div
      ref={region}
      data-alerts-scroll=""
      onBlur={(event) => {
        // Focus moving to another element forgets the row; moving within this list records it again on arrival.
        if (event.relatedTarget !== null) focused.current = undefined
      }}
      {...stylex.props(styles.scroll)}
    >
      {feed._tag === 'Observed' ? <DecodeNotice noun="alert" decode={decode} shown={shownIds} /> : null}
      {feed._tag === 'Waiting' ? <p ref={message} role="status" tabIndex={-1} {...stylex.props(styles.empty)}>Loading alerts…</p>
        : feed._tag === 'Unavailable' ? feed.reason === 'unauthorized'
          ? <SystemNotice scope="rail" kind="error" connection title={credentialExpiredMessage} />
          : (
          <p ref={message} role="alert" tabIndex={-1} {...stylex.props(styles.empty)}>
            {feed.reason === 'ungranted' ? 'Read access to alerts has not been granted.'
              : feed.reason === 'unsupported' ? 'This connection does not support alerts.'
                : 'Alerts could not be loaded; reconnect and try again.'}
          </p>
        ) : (
          <>
            {feed.freshness === 'stale' ? (
              <p role="status" {...stylex.props(styles.empty)}>
                Last known alerts{feed.readAt === undefined ? '' : `, as of ${ago(new Date(feed.readAt).toISOString(), now)} ago`}; this list may be out of date.
              </p>
            ) : null}
            {rows.length === 0 ? (
              <p ref={message} tabIndex={-1} {...stylex.props(styles.empty)}>Nothing needs you. Asks, approvals and harness questions show here.</p>
            ) : (
              <ul aria-label="Alerts" {...stylex.props(styles.list)}>
                {rows.map((row) => (
                  <AlertRow
                    key={row.id}
                    row={row}
                    selected={selected === row.id}
                    onSelect={() => onSelect?.(row.id)}
                    onFocusWithin={() => { focused.current = row.id }}
                    onOpen={onOpen}
                    context={context}
                  />
                ))}
              </ul>
            )}
          </>
        )}
    </div>
  )
}

const AlertRow = ({ row, selected, onSelect, onFocusWithin, onOpen, context }: {
  readonly row: AlertsTabRow
  readonly context: AlertContext
  readonly selected: boolean
  readonly onSelect: () => void
  /** Focus arrived in this row, including its menu, which React bubbles here from the popover. */
  readonly onFocusWithin: () => void
  readonly onOpen: (ref: string) => void
}): React.ReactNode => {
  const touch = React.useContext(NarrowLayoutContext)
  const [menu, setMenu] = React.useState(false)
  const agent = row.agent === undefined ? undefined : context.agent?.(row.agent)
  const agentName = agent?.name ?? row.agentLabel
  const mission = row.mission === undefined ? undefined : missionName(row.mission, context)
  const said = [agentName === undefined ? undefined : `from ${agentName}`, mission === undefined ? undefined : `mission ${mission}`, row.step === undefined ? undefined : `step ${row.step}`].filter(Boolean).join(', ')
  const saidId = React.useId()
  return (
    <li
      data-alert-id={row.id}
      data-alert-glyph={row.glyph}
      onContextMenu={(event) => { event.preventDefault(); setMenu(true) }}
      onFocus={onFocusWithin}
    >
      <div {...stylex.props(styles.row, selected && styles.rowSelected)}>
        <Aria.Button
          data-alert-open=""
          aria-current={selected ? 'true' : undefined}
          // The name is the row's visible words in their order, so speech input can say what it sees (label in name);
          // which of them is the agent, the mission and the step is the description.
          aria-label={[row.title, row.age, agentName, mission, row.step, row.line2 === '' ? undefined : row.line2, GLYPH_WORDS[row.glyph]].filter(Boolean).join(', ')}
          aria-describedby={said === '' ? undefined : saidId}
          onPress={onSelect}
          onKeyDown={(event) => {
            if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) setMenu(true)
            else event.continuePropagation()
          }}
          {...stylex.props(styles.main)}
        >
          <span {...stylex.props(styles.line)}>
            <span aria-hidden="true" data-alert-row-glyph={row.glyph} {...stylex.props(styles.glyph, row.glyph === '?' && styles.question, row.glyph === '!' && styles.fault)} />
            <span {...stylex.props(styles.title)}>{row.title}</span>{' '}
            <span {...stylex.props(styles.metric)}>{row.age}</span>
          </span>
          {agentName === undefined && mission === undefined && row.step === undefined && row.line2 === '' ? null : <>{' '}
            <span data-alert-context="" {...stylex.props(styles.meta, styles.context)}>
              {agentName === undefined ? null : <HoverTarget entity={row.agent === undefined ? undefined : { kind: 'agent', id: row.agent }} style={[styles.agent, mission === undefined && row.step === undefined ? styles.agentAlone : styles.agentShared]}>
                {agent === undefined ? null : <span {...stylex.props(styles.mark)}><SidebarStatus status={agent.status} statusLabel={statuses[agent.status].label} variant="Marks" needsYou={agent.needsYou === true} freshness="live" iconOnly decorative /></span>}<span data-alert-agent="" {...stylex.props(styles.ellipsis)}>{agentName}</span>
              </HoverTarget>}
              {mission === undefined && row.step === undefined ? null : <>
                {agentName === undefined ? null : <span aria-hidden="true" {...stylex.props(styles.sep)}>·</span>}
                <HoverTarget entity={row.mission === undefined ? undefined : { kind: 'mission', id: row.mission }} style={styles.missionPart}>
                  <span {...stylex.props(styles.ellipsis)}>{mission === undefined ? null : <span data-alert-mission="">{mission}</span>}{mission !== undefined && row.step !== undefined ? <span aria-hidden="true" {...stylex.props(styles.sep)}>›</span> : null}{row.step === undefined ? null : <span data-alert-step="">{row.step}</span>}</span>
                </HoverTarget>
              </>}
              {row.line2 === '' ? null : <><span aria-hidden="true" {...stylex.props(styles.sep)}>·</span><span {...stylex.props(styles.ellipsis)}>{row.line2}</span></>}
            </span>
          </>}
        </Aria.Button>
        <Aria.MenuTrigger isOpen={menu} onOpenChange={setMenu}>
          <Aria.Button aria-label={`Actions for ${row.title}`} {...stylex.props(styles.more, touch && styles.touch)}>⋯</Aria.Button>
          <Aria.Popover placement="bottom end" {...stylex.props(styles.popover)}>
            <Aria.Menu
              aria-label={`Actions for ${row.title}`}
              autoFocus="first"
              onAction={(key) => (key === 'answer' ? onSelect() : onOpen(String(key)))}
              {...stylex.props(styles.menu)}
            >
              <Aria.MenuItem id="answer" {...stylex.props(styles.item)}>Answer</Aria.MenuItem>
              {row.conversation === undefined ? null : <Aria.MenuItem id={row.conversation} {...stylex.props(styles.item)}>Open conversation</Aria.MenuItem>}
              {row.mission === undefined ? null : <Aria.MenuItem id={row.mission} {...stylex.props(styles.item)}>Open mission</Aria.MenuItem>}
            </Aria.Menu>
          </Aria.Popover>
        </Aria.MenuTrigger>
      </div>
      {said === '' ? null : <span id={saidId} hidden>{said}</span>}
    </li>
  )
}

/**
 * A pointer-only hover target inside the row's one button: it shows the entity's shared card after the hover intent, and
 * adds no focus stop (the row's name already says who asked and which mission and step, and its menu opens either).
 */
const HoverTarget = ({ entity: given, style, children }: { readonly entity: EntityRef | undefined; readonly style: stylex.StyleXStyles | readonly stylex.StyleXStyles[]; readonly children: React.ReactNode }) => {
  const anchor = React.useRef<HTMLSpanElement>(null)
  const intent = useHoverCardIntent()
  // No source knows the entity (a host without cards): no surface at all, never an empty frame.
  const card = useEntityCard(given ?? { kind: 'agent', id: '' })
  const entity = given === undefined || card === null ? undefined : given
  return <span ref={anchor} data-hover-target={entity?.kind} onPointerEnter={entity === undefined ? undefined : () => intent.intend(true)} onPointerLeave={entity === undefined ? undefined : () => intent.intend(false)} {...stylex.props(style)}>
    {children}
    {entity !== undefined && <HoverCardSurface triggerRef={anchor} isOpen={intent.isOpen} onOpenChange={intent.setOpen} placement="right top">{card}</HoverCardSurface>}
  </span>
}

const focus = { ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: g.focusInset } }
const styles = stylex.create({
  // Fills a column parent and scrolls itself; minHeight 0 lets a `minHeight: 0` flex parent bound it instead of growing.
  scroll: { flex: '1 1 0', minWidth: 0, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain' },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' },
  row: { display: 'flex', alignItems: 'stretch', borderRadius: 4, ':hover': { backgroundColor: c.rowHover } },
  rowSelected: { backgroundColor: c.rowActive, ':hover': { backgroundColor: c.rowActive } },
  main: { flexGrow: 1, minWidth: 0, minHeight: g.sidebarRow, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 2, paddingBlock: s.xs, paddingInline: s.row, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ...focus },
  line: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  glyph: { flexShrink: 0, width: g.iconSm, textAlign: 'center', color: c.attention, fontWeight: 600, '::before': { content: 'attr(data-alert-row-glyph)' } },
  // A question is attention like any other ask: amber, told apart by its glyph, not by the accent.
  question: { color: c.attention },
  fault: { color: c.danger },
  title: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metric: { flexShrink: 0, color: c.fgSoft, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums' },
  // One line: the agent keeps its name up to 45%, the mission and step take the rest, and both truncate.
  context: { display: 'flex', alignItems: 'center', minWidth: 0 },
  agent: { display: 'inline-flex', alignItems: 'center', flex: '0 1 auto', minWidth: 0, overflow: 'hidden' },
  // Beside a mission the agent keeps at most half the line, so the mission always shows; alone it takes the line.
  agentShared: { maxWidth: '50%' },
  agentAlone: { maxWidth: '100%' },
  mark: { display: 'inline-flex', marginInlineEnd: s.xs, verticalAlign: 'middle' },
  missionPart: { display: 'inline-flex', flex: '1 1 0', minWidth: 0, overflow: 'hidden' },
  ellipsis: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  sep: { flexShrink: 0, paddingInline: s.xs, color: c.fgSoft },
  meta: { paddingInlineStart: `calc(${g.iconSm} + ${s.sm})`, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  more: { flexShrink: 0, width: 24, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fgSoft, cursor: 'pointer', ':hover': { color: c.fg }, ...focus },
  touch: { minHeight: g.touchTarget, minWidth: g.touchTarget },
  popover: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6, fontFamily: t.fontSans, fontSize: t.metaSize, zIndex: 100 },
  menu: { outline: 'none', padding: s.xs, minWidth: 160 },
  item: { paddingBlock: s.xs, paddingInline: s.md, borderRadius: 4, cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { backgroundColor: c.rowHover } },
  empty: { margin: 0, padding: s.md, color: c.fgSoft, fontSize: t.metaSize, outline: 'none', ...focus },
})
