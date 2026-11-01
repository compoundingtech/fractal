import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { promptAnswers } from '../../../../clients/typescript/fractal-views/homeView.ts'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { AgentAlerts } from '../conversation/alerts/AgentAlerts.tsx'
import type { AlertAction, AlertActionResult } from '../conversation/alerts/alert-action-port.ts'
import type { PromptAnswerResult } from '../conversation/alerts/prompt-answer-port.ts'
import { useAttention, useDataSource, useNow } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import { alertsTabRows, type AlertsTabRow } from './alertsTab.ts'

// The sidebar's Alerts tab: every open alert across conversations in Fractal's tier order, one SR-2 row
// each, answered in place. It reads the attention collection the conversation strips read, so the tab
// marker, this list and each conversation agree. It offers only actions the source serves.

type Act = (alert: Native.Attention, action: AlertAction) => Promise<AlertActionResult>
type Answer = (alert: Native.Attention, answer: string, sent: string) => Promise<PromptAnswerResult>

const GLYPH_WORDS: Record<AlertsTabRow['glyph'], string> = { '?': 'asks a question', '◆': 'needs you', '!': 'fault' }

/** The Alerts tab over the active source: its attention feed, clock, and answer ports when it serves them. */
export const AlertsTab = ({ onOpen }: { readonly onOpen: (ref: string) => void }): React.ReactNode => {
  const { alertActions: actions, prompts } = useDataSource()
  return (
    <AlertsList
      feed={useAttention()}
      now={useNow()}
      onOpen={onOpen}
      onAct={actions === undefined ? undefined : (alert, action) => actions.act({ alert, action })}
      onAnswer={prompts === undefined ? undefined : (alert, _answer, sent) => {
        const prompt = promptAnswers(alert)
        return prompt === null
          ? Promise.resolve({ _tag: 'Refused', detail: 'This prompt is answered in the seat\'s terminal.' })
          : prompts.respond({ alert: { id: alert.id, revision: alert.revision, source_id: alert.source_id, target_id: prompt.target_id, episode: prompt.episode }, answer: sent })
      }}
    />
  )
}

/**
 * The list itself, from a feed: waiting, unavailable, stale and empty each say so; unknown is never shown as none.
 * It is its own bounded scroll region, so it fills a `minHeight: 0` column and every row and expanded answer stays reachable.
 */
export const AlertsList = ({ feed, now, onOpen, onAct, onAnswer }: {
  readonly feed: Feed<readonly Native.Attention[]>
  readonly now: number
  /** Opens an agent conversation or a mission by ref. */
  readonly onOpen: (ref: string) => void
  readonly onAct?: Act
  readonly onAnswer?: Answer
}): React.ReactNode => {
  const rows = React.useMemo(() => (feed._tag === 'Observed' ? alertsTabRows(feed.value, now) : []), [feed, now])
  const [expanded, setExpanded] = React.useState<string>()
  const region = React.useRef<HTMLDivElement>(null)
  const message = React.useRef<HTMLParagraphElement>(null)
  // The row that last held focus: its own controls, its answer, or its menu. It stays recorded when focus falls to
  // the page because a control inside the row unmounted (a sent answer's confirmation), and is forgotten once the
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
    if (expanded !== undefined && !ids.includes(expanded)) setExpanded(undefined)
    const lost = focused.current
    if (lost === undefined || ids.includes(lost)) return
    focused.current = undefined
    const active = document.activeElement
    if (active !== null && active !== document.body) return
    const at = before.indexOf(lost)
    const next = [...before.slice(at + 1), ...before.slice(0, Math.max(at, 0)).reverse()].find((id) => ids.includes(id)) ?? ids[0]
    const row = next === undefined ? undefined : [...(region.current?.querySelectorAll<HTMLElement>('[data-alert-id]') ?? [])].find((each) => each.dataset.alertId === next)
    const target = row === undefined ? message.current : row.querySelector<HTMLElement>('button[aria-expanded]')
    target?.focus()
  }, [rows, expanded])
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
      {feed._tag === 'Waiting' ? <p ref={message} role="status" tabIndex={-1} {...stylex.props(styles.empty)}>Loading alerts…</p>
        : feed._tag === 'Unavailable' ? (
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
                    expanded={expanded === row.id}
                    onExpand={(open) => setExpanded(open ? row.id : undefined)}
                    onFocusWithin={() => { focused.current = row.id }}
                    onOpen={onOpen}
                    onAct={onAct}
                    onAnswer={onAnswer}
                  />
                ))}
              </ul>
            )}
          </>
        )}
    </div>
  )
}

const AlertRow = ({ row, expanded, onExpand, onFocusWithin, onOpen, onAct, onAnswer }: {
  readonly row: AlertsTabRow
  readonly expanded: boolean
  readonly onExpand: (open: boolean) => void
  /** Focus arrived in this row, including its menu, which React bubbles here from the popover. */
  readonly onFocusWithin: () => void
  readonly onOpen: (ref: string) => void
  readonly onAct: Act | undefined
  readonly onAnswer: Answer | undefined
}): React.ReactNode => {
  const touch = React.useContext(NarrowLayoutContext)
  const [menu, setMenu] = React.useState(false)
  const trigger = React.useRef<HTMLButtonElement>(null)
  const panel = React.useRef<HTMLDivElement>(null)
  const panelId = React.useId()
  // Opening moves focus to the first answer (else the answer region), so Enter, then Enter again, reaches the alert's own confirmation.
  React.useEffect(() => {
    if (expanded) (panel.current?.querySelector<HTMLElement>('textarea:not([disabled]), button:not([disabled]), a[href]') ?? panel.current)?.focus()
  }, [expanded])
  return (
    <li
      data-alert-id={row.id}
      data-alert-glyph={row.glyph}
      onContextMenu={(event) => { event.preventDefault(); setMenu(true) }}
      onFocus={onFocusWithin}
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || !expanded || event.defaultPrevented) return
        event.preventDefault()
        onExpand(false)
        trigger.current?.focus()
      }}
    >
      <div {...stylex.props(styles.row)}>
        <Aria.Button
          ref={trigger}
          aria-expanded={expanded}
          aria-controls={expanded ? panelId : undefined}
          aria-label={`${row.title}, ${row.age}${row.line2 === '' ? '' : `, ${row.line2}`}, ${GLYPH_WORDS[row.glyph]}`}
          onPress={() => onExpand(!expanded)}
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
          {row.line2 === '' ? null : <>{' '}<span {...stylex.props(styles.meta)}>{row.line2}</span></>}
        </Aria.Button>
        <Aria.MenuTrigger isOpen={menu} onOpenChange={setMenu}>
          <Aria.Button aria-label={`Actions for ${row.title}`} {...stylex.props(styles.more, touch && styles.touch)}>⋯</Aria.Button>
          <Aria.Popover placement="bottom end" {...stylex.props(styles.popover)}>
            <Aria.Menu
              aria-label={`Actions for ${row.title}`}
              autoFocus="first"
              onAction={(key) => (key === 'answer' ? onExpand(true) : onOpen(String(key)))}
              {...stylex.props(styles.menu)}
            >
              <Aria.MenuItem id="answer" {...stylex.props(styles.item)}>Answer</Aria.MenuItem>
              {row.conversation === undefined ? null : <Aria.MenuItem id={row.conversation} {...stylex.props(styles.item)}>Open conversation</Aria.MenuItem>}
              {row.mission === undefined ? null : <Aria.MenuItem id={row.mission} {...stylex.props(styles.item)}>Open mission</Aria.MenuItem>}
            </Aria.Menu>
          </Aria.Popover>
        </Aria.MenuTrigger>
      </div>
      {expanded ? (
        <div ref={panel} id={panelId} role="region" tabIndex={-1} aria-label={`Answer ${row.title}`} {...stylex.props(styles.panel)}>
          <AgentAlerts alerts={[row.item]} calls={[]} served onAct={onAct} onAnswer={onAnswer} />
        </div>
      ) : null}
    </li>
  )
}

const focus = { ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: '-2px' } }
const styles = stylex.create({
  // Fills a column parent and scrolls itself; minHeight 0 lets a `minHeight: 0` flex parent bound it instead of growing.
  scroll: { flex: '1 1 0', minWidth: 0, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain' },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' },
  row: { display: 'flex', alignItems: 'stretch', borderRadius: 4, ':hover': { backgroundColor: c.rowHover } },
  main: { flexGrow: 1, minWidth: 0, minHeight: g.sidebarRow, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 2, paddingBlock: s.xs, paddingInline: s.row, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ...focus },
  line: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  glyph: { flexShrink: 0, width: g.iconSm, textAlign: 'center', color: c.attention, fontWeight: 600, '::before': { content: 'attr(data-alert-row-glyph)' } },
  question: { color: c.primary },
  fault: { color: c.danger },
  title: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metric: { flexShrink: 0, color: c.fgMuted, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums' },
  meta: { paddingInlineStart: `calc(${g.iconSm} + ${s.sm})`, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  more: { flexShrink: 0, width: 24, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fgMuted, cursor: 'pointer', ':hover': { color: c.fg }, ...focus },
  touch: { minHeight: g.touchTarget, minWidth: g.touchTarget },
  popover: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6, fontFamily: t.fontSans, fontSize: t.metaSize, zIndex: 100 },
  menu: { outline: 'none', padding: s.xs, minWidth: 160 },
  item: { paddingBlock: s.xs, paddingInline: s.md, borderRadius: 4, cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { backgroundColor: c.rowHover } },
  panel: { paddingInline: s.row, paddingBlockEnd: s.md, outline: 'none' },
  empty: { margin: 0, padding: s.md, color: c.fgMuted, fontSize: t.metaSize, outline: 'none', ...focus },
})
