import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { geometryVars as g, spaceVars as s } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DecisionCard } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/DecisionCard.tsx'
import { NarrowLayoutContext } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import type { UxTelemetry } from '../../telemetry/ux.ts'
import { useAlertAnswersStore } from './alertAnswers.ts'
import { conversationAlerts } from './alertChoices.ts'
import { AnswerAlert } from './AnswerAlert.tsx'
import { PromptAlert } from './PromptAlert.tsx'
import { useAlertAnswers, type Act, type Answer } from './useAlertAnswers.ts'

/** About 300px on an 800px window: the transcript keeps its room; the card's body scrolls, its controls do not. */
const DOCK_SHARE = 0.42
/** A phone stacks the field under the options, which needs more of a shorter column. */
const DOCK_SHARE_NARROW = 0.65
/** Below this the card starts folded to one line, unless the person unfolded it. */
const FOLD_BELOW = 480
const DOCK_MIN = 220

const isDocumentRef = (ref: string): boolean => ref.startsWith('doc/')

/** The height of the host this card docks in. */
const useRoom = (): readonly [React.RefCallback<HTMLElement>, number | undefined] => {
  const [room, setRoom] = React.useState<number>()
  const observer = React.useRef<ResizeObserver>(undefined)
  const ref = React.useCallback<React.RefCallback<HTMLElement>>((node) => {
    observer.current?.disconnect()
    // The nearest ancestor with a box of its own: a `display: contents` wrapper measures zero.
    let host = node?.parentElement
    while (host != null && getComputedStyle(host).display === 'contents') host = host.parentElement
    if (host == null || typeof ResizeObserver === 'undefined') return
    observer.current = new ResizeObserver(() => setRoom(host.clientHeight))
    observer.current.observe(host)
    setRoom(host.clientHeight)
  }, [])
  return [ref, room]
}

/**
 * The alerts of an agent's conversation, above its composer: one focused card, with a pager when there are several.
 * Answering advances to the next at once; a refused answer comes back without moving the person off what they are on.
 */
export const AgentAlerts = ({ alerts, agentRef, onAnswer, onAct, onOpenDocument }: {
  readonly alerts: readonly Native.Attention[]
  /** Keeps the record of an answer in view after the feed has dropped its alert. */
  readonly agentRef?: string
  readonly onAnswer?: Answer
  readonly onAct?: Act
  readonly onOpenDocument?: (ref: string) => void
}): React.ReactNode => {
  const store = useAlertAnswersStore()
  const narrow = React.useContext(NarrowLayoutContext)
  const [roomRef, room] = useRoom()
  const [foldChoice, setFoldChoice] = React.useState<boolean>()
  const [currentId, setCurrentId] = React.useState<string>()
  const lastIndex = React.useRef(0)
  const open = alerts.filter((alert) => !store.hidden(alert))
  const records = [...store.getSnapshot().pending.values()].filter(({ item }) =>
    alerts.some((alert) => alert.id === item.id) || (agentRef !== undefined && conversationAlerts([item], agentRef).length > 0))
  const found = open.findIndex((alert) => alert.id === currentId)
  // The alert being read stays read, wherever others come and go; a closed one gives way to the one that took its place.
  const index = found >= 0 ? found : Math.min(lastIndex.current, Math.max(open.length - 1, 0))
  const current = open[index]
  // Held by id from the moment it shows, so a refused answer returning ahead of it cannot take its place.
  if (current !== undefined && current.id !== currentId) setCurrentId(current.id)
  React.useLayoutEffect(() => { lastIndex.current = index })
  const card = React.useRef<HTMLElement>(null)
  const resolved = React.useRef<HTMLElement>(null)
  const focusFollows = React.useRef(false)
  const onBegin = () => {
    focusFollows.current = card.current?.contains(document.activeElement) === true
    if (current !== undefined) lastIndex.current = index
  }
  // The answered card unmounted with focus in it. Anywhere else, focus is the person's and stays.
  React.useLayoutEffect(() => {
    if (!focusFollows.current) return
    focusFollows.current = false
    if (document.activeElement !== null && document.activeElement !== document.body) return
    ;(card.current ?? resolved.current)?.focus()
  })
  if (open.length === 0 && records.length === 0) return null
  const folded = foldChoice ?? (room !== undefined && room < FOLD_BELOW)
  const maxBlockSize = room === undefined ? undefined : Math.max(DOCK_MIN, Math.round(room * (narrow ? DOCK_SHARE_NARROW : DOCK_SHARE)))
  const pager = open.length > 1 ? {
    index, count: open.length,
    onPrevious: () => setCurrentId(open[index - 1]?.id),
    onNext: () => setCurrentId(open[index + 1]?.id),
  } : undefined
  const shared = { density: folded ? 'row' as const : 'docked' as const, onFold: () => setFoldChoice(!folded), onBegin, focusRef: card, ...(pager === undefined ? {} : { pager }), ...(maxBlockSize === undefined ? {} : { maxBlockSize }) }
  return (
    <div ref={roomRef} {...stylex.props(styles.pinned)} data-testid="conversation-alerts">
      <div {...stylex.props(styles.strip)}>
        {records.map(({ item, label, phase }, at) => (
          <DecisionCard key={item.id} density="row" data={{ 'data-answer-phase': phase }} label={`Answered: ${item.title}`} meta={[]} heading={item.title} resolved={`You answered ${label}`}
            {...(at === 0 ? { focusRef: resolved } : {})}
            {...(phase === 'confirmed' ? {} : { status: { tone: 'progress' as const, text: phase === 'sending' ? 'Sending…' : 'Waiting for Fractal to clear it…' } })} />
        ))}
        {current === undefined ? null : current.attention_kind === 'harness-prompt'
          ? <PromptAlert key={current.id} alert={current} {...(onAnswer === undefined ? {} : { onAnswer })} {...shared} />
          : <AnswerAlert key={current.id} alert={current} {...(onAct === undefined ? {} : { onAct })} {...(onOpenDocument === undefined ? {} : { onOpenRef: onOpenDocument, opensRef: isDocumentRef })} {...shared} />}
      </div>
    </div>
  )
}

/** The agent's alerts from the active data source, answered through its ports. */
export const LiveAgentAlerts = ({ agentRef, ux, onOpenDocument }: { readonly agentRef: string; readonly ux?: UxTelemetry; readonly onOpenDocument?: (ref: string) => void }): React.ReactNode => {
  const { feed, onAct, onAnswer } = useAlertAnswers(ux)
  const alerts = React.useMemo(() => (feed._tag === 'Observed' ? conversationAlerts(feed.value, agentRef) : []), [feed, agentRef])
  return <AgentAlerts alerts={alerts} agentRef={agentRef} {...(onAct === undefined ? {} : { onAct })} {...(onAnswer === undefined ? {} : { onAnswer })} {...(onOpenDocument === undefined ? {} : { onOpenDocument })} />
}

const styles = stylex.create({
  pinned: { width: '100%', minWidth: 0, boxSizing: 'border-box', paddingInline: s.section, flexShrink: 0 },
  strip: { maxWidth: g.lane, marginInline: 'auto', display: 'flex', flexDirection: 'column', gap: s.sm, paddingBlock: s.md },
})
