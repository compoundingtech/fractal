import { markdownInline } from '@fractal/client/markdown'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DecisionCard } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DecisionCard.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { InlineMarkdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { useAlertAnswersStore } from '../conversation/alerts/alertAnswers.ts'
import { AnswerAlert } from '../conversation/alerts/AnswerAlert.tsx'
import { PromptAlert } from '../conversation/alerts/PromptAlert.tsx'
import { useAlertAnswers } from '../conversation/alerts/useAlertAnswers.ts'
import { useNow } from '../data/react.tsx'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { alertsTabRows } from './alertsTab.ts'

const isOpenableRef = (ref: string): boolean => /^(?:doc|agent|mission)\//.test(ref)

/**
 * One alert as the main area's content: the same decision card at its full density. Answering advances to the next
 * alert at once, before the server replies, as the Fractal terminal UI does; a refused answer returns to the list
 * without taking the person off what they moved on to.
 */
export const AlertDetailsPane = ({ id, ux, onOpenAlert, onOpenRef, onClose, onBack }: {
  readonly id: string
  readonly ux?: UxTelemetry
  readonly onOpenAlert: (id: string) => void
  /** Opens a conversation or a mission by ref. */
  readonly onOpenRef: (ref: string) => void
  readonly onClose: () => void
  /** Back to the alerts list, where the list is a drawer. */
  readonly onBack: () => void
}): React.ReactNode => {
  const touch = React.useContext(NarrowLayoutContext)
  const now = useNow()
  const store = useAlertAnswersStore()
  const { feed, onAct, onAnswer } = useAlertAnswers(ux)
  const rows = React.useMemo(() => (feed._tag === 'Observed' ? alertsTabRows(feed.value, now) : []), [feed, now])
  const open = rows.filter((row) => !store.hidden(row.item))
  const row = open.find((each) => each.id === id)
  const records = [...store.getSnapshot().pending.values()]
  const answeredHere = store.getSnapshot().pending.has(id)
  const card = React.useRef<HTMLElement>(null)
  const advance = () => {
    const at = open.findIndex((each) => each.id === id)
    const next = open[at + 1] ?? open[at - 1]
    if (next !== undefined) onOpenAlert(next.id)
  }
  // Opening a row brings the keyboard here; a draft being written anywhere else keeps it.
  React.useEffect(() => {
    const active = document.activeElement
    if (active instanceof HTMLTextAreaElement && card.current?.contains(active) !== true) return
    // The pane starts at its top, with the way back in view.
    card.current?.focus({ preventScroll: true })
  }, [id])
  const next = open[0]
  const iconButton = stylex.props(styles.iconButton, touch && styles.touch)
  const textButton = stylex.props(styles.textButton, touch && styles.touch)
  return (
    <div data-testid="alert-details" {...stylex.props(styles.pane)}>
      <div {...stylex.props(styles.lane)}>
        <div {...stylex.props(styles.bar)}>
          {touch ? <Aria.Button aria-label="Back to alerts" onPress={onBack} {...textButton}><span {...stylex.props(styles.flip)}><Icon name="chevron-right" size={12} /></span>Alerts</Aria.Button> : null}
          <span {...stylex.props(styles.grow)} />
          {row?.conversation === undefined ? null : <Aria.Button onPress={() => onOpenRef(row.conversation!)} {...textButton}>Open conversation</Aria.Button>}
          {row?.mission === undefined ? null : <Aria.Button onPress={() => onOpenRef(row.mission!)} {...textButton}>Open mission</Aria.Button>}
          <Aria.Button aria-label="Close alert" onPress={onClose} {...iconButton}><Icon name="x" size={12} /></Aria.Button>
        </div>
        {records.map((record) => <DecisionCard key={record.item.id} density="row" data={{ 'data-answer-phase': record.phase }} label={`Answered: ${record.item.title}`} meta={[]} heading={record.item.title} resolved={`You answered ${record.label}`}
          {...(record.phase === 'confirmed' ? {} : { status: { tone: 'progress' as const, text: record.phase === 'sending' ? 'Sending…' : 'Waiting for Fractal to clear it…' } })} />)}
        {feed._tag === 'Waiting' ? <SystemNotice kind="loading" title="Loading alert" />
          : feed._tag === 'Unavailable' ? <SystemNotice kind="empty" title="Alerts could not be loaded" detail="Reconnect and try again" />
            : row !== undefined
              ? row.item.attention_kind === 'harness-prompt'
                ? <PromptAlert key={row.id} alert={row.item} density="full" now={now} onBegin={advance} focusRef={card} {...(onAnswer === undefined ? {} : { onAnswer })} />
                : <AnswerAlert key={row.id} alert={row.item} density="full" now={now} onBegin={advance} focusRef={card} onOpenRef={onOpenRef} opensRef={isOpenableRef} {...(onAct === undefined ? {} : { onAct })} />
              : <div {...stylex.props(styles.gone)}>
                <h2 {...stylex.props(styles.goneTitle)}>{answeredHere ? 'All answered' : 'No longer open'}</h2>
                {next === undefined
                  ? <p {...stylex.props(styles.goneText)}>Nothing else needs you.</p>
                  : <Aria.Link onPress={() => onOpenAlert(next.id)} {...stylex.props(styles.goneLink)}>Next: <InlineMarkdown text={markdownInline(next.title)} /></Aria.Link>}
              </div>}
      </div>
    </div>
  )
}

const styles = stylex.create({
  pane: { position: 'absolute', inset: 0, zIndex: 2, overflowY: 'auto', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, paddingInline: s.section, paddingBlockEnd: s.section },
  lane: { maxWidth: g.lane, marginInline: 'auto', display: 'flex', flexDirection: 'column', gap: s.md },
  bar: { display: 'flex', alignItems: 'center', gap: s.xs, minHeight: g.band },
  grow: { flexGrow: 1 },
  textButton: { display: 'inline-flex', alignItems: 'center', gap: s.xs, minHeight: g.controlMd, paddingInline: s.md, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg, backgroundColor: c.rowHover } },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlMd, height: g.controlMd, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgMuted, cursor: 'pointer', ':hover': { color: c.fg, backgroundColor: c.rowHover } },
  touch: { minHeight: g.touchTarget, minWidth: g.touchTarget },
  flip: { display: 'inline-flex', transform: 'rotate(180deg)' },
  gone: { display: 'flex', flexDirection: 'column', gap: s.md, paddingBlock: s.section },
  goneTitle: { margin: 0, fontSize: g.dialogTitle, fontWeight: t.weightSemibold },
  goneText: { margin: 0, color: c.fgMuted },
  goneLink: { alignSelf: 'flex-start', color: c.primary, cursor: 'pointer' },
})
