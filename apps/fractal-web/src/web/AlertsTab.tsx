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

/** The list itself, from a feed: waiting, unavailable, stale and empty each say so; unknown is never shown as none. */
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
  if (feed._tag === 'Waiting') return <p role="status" {...stylex.props(styles.empty)}>Loading alerts…</p>
  if (feed._tag === 'Unavailable') {
    return (
      <p role="alert" {...stylex.props(styles.empty)}>
        {feed.reason === 'ungranted' ? 'Read access to alerts has not been granted.'
          : feed.reason === 'unsupported' ? 'This connection does not support alerts.'
            : 'Alerts could not be loaded; reconnect and try again.'}
      </p>
    )
  }
  return (
    <>
      {feed.freshness === 'stale' ? (
        <p role="status" {...stylex.props(styles.empty)}>
          Last known alerts{feed.readAt === undefined ? '' : `, as of ${ago(new Date(feed.readAt).toISOString(), now)} ago`}; this list may be out of date.
        </p>
      ) : null}
      {rows.length === 0 ? (
        <p {...stylex.props(styles.empty)}>Nothing needs you. Asks, approvals and harness questions show here.</p>
      ) : (
        <ul aria-label="Alerts" {...stylex.props(styles.list)}>
          {rows.map((row) => (
            <AlertRow key={row.id} row={row} expanded={expanded === row.id} onExpand={(open) => setExpanded(open ? row.id : undefined)} onOpen={onOpen} onAct={onAct} onAnswer={onAnswer} />
          ))}
        </ul>
      )}
    </>
  )
}

const AlertRow = ({ row, expanded, onExpand, onOpen, onAct, onAnswer }: {
  readonly row: AlertsTabRow
  readonly expanded: boolean
  readonly onExpand: (open: boolean) => void
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
          aria-label={`${row.title}, ${GLYPH_WORDS[row.glyph]}, waited ${row.age}`}
          onPress={() => onExpand(!expanded)}
          onKeyDown={(event) => {
            if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) setMenu(true)
            else event.continuePropagation()
          }}
          {...stylex.props(styles.main)}
        >
          <span {...stylex.props(styles.line)}>
            <span aria-hidden="true" {...stylex.props(styles.glyph, row.glyph === '?' && styles.question, row.glyph === '!' && styles.fault)}>{row.glyph}</span>
            <span {...stylex.props(styles.title)}>{row.title}</span>
            <span {...stylex.props(styles.metric)}>{row.age}</span>
          </span>
          {row.line2 === '' ? null : <span {...stylex.props(styles.meta)}>{row.line2}</span>}
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
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' },
  row: { display: 'flex', alignItems: 'stretch', borderRadius: 4, ':hover': { backgroundColor: c.rowHover } },
  main: { flexGrow: 1, minWidth: 0, minHeight: g.sidebarRow, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 2, paddingBlock: s.xs, paddingInline: s.row, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ...focus },
  line: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  glyph: { flexShrink: 0, width: g.iconSm, textAlign: 'center', color: c.attention, fontWeight: 600 },
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
  empty: { margin: 0, padding: s.md, color: c.fgMuted, fontSize: t.metaSize },
})
