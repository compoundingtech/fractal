import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { Option } from 'effect'

import {
  alertConversations,
  alertsIn,
  answerLabel,
  servedAnswers,
  servedPrompt,
  pendingQuestions,
  promptAnswers,
  promptAlertKey,
  promptCalls,
  questionAnswer,
} from '../../../../../clients/typescript/fractal-views/homeView.ts'
import { geometryVars as g, spaceVars as s } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAttention, useDataSource } from '../../data/react.tsx'
import type { UxTelemetry } from '../../telemetry/ux.ts'
import { tokens } from '../../ui-compat/tokens.stylex.ts'
import { AlertButton } from './AlertButton.tsx'
import type { AlertAction, AlertActionResult } from './alert-action-port.ts'
import { AnswerAlert } from './AnswerAlert.tsx'
import type { PromptAnswerResult } from './prompt-answer-port.ts'

const styles = stylex.create({
  pinned: { width: '100%', minWidth: 0, boxSizing: 'border-box', paddingInline: s.section, flexShrink: 0 },
  strip: {
    maxWidth: g.lane,
    marginInline: 'auto',
    display: 'flex',
    flexDirection: 'column',
    gap: '8px',
    paddingBlock: s.md,
    borderTopWidth: 1,
    borderTopStyle: 'solid',
    borderTopColor: tokens['--ds-gray-alpha-400'],
    // Many alerts keep the conversation readable: the strip scrolls, the transcript keeps its room.
    maxHeight: '40vh',
    overflowY: 'auto',
  },
  count: { fontWeight: 600, fontSize: '0.75rem', color: tokens['--ds-gray-900'] },
  folded: { display: 'flex', flexDirection: 'column', gap: '8px' },
  summary: { cursor: 'pointer', fontWeight: 600, fontSize: '0.8125rem', color: tokens['--ds-gray-1000'] },
  alert: { display: 'flex', flexDirection: 'column', gap: '4px', fontSize: '0.8125rem' },
  title: { fontWeight: 600, color: tokens['--ds-gray-1000'] },
  detail: { color: tokens['--ds-gray-900'], whiteSpace: 'pre-wrap' },
  call: {
    margin: 0,
    padding: '8px',
    borderRadius: 4,
    backgroundColor: tokens['--ds-gray-alpha-100'],
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
    fontSize: '0.75rem',
  },
  answers: { display: 'flex', flexWrap: 'wrap', gap: '8px' },
  note: { color: tokens['--ds-gray-900'], fontSize: '0.75rem' },
})

/** The native prompt alerts of one agent's conversation, open ones only. */
export const promptAlerts = (items: readonly Native.Attention[], agentRef: string): readonly Native.Attention[] =>
  items.filter(
    (item) =>
      item.attention_kind === 'harness-prompt' &&
      item.state === 'open' &&
      alertConversations(item).includes(agentRef),
  )

/**
 * Every alert this agent's conversation shows, as the Fractal terminal UI and the phone choose them (`alertsIn`):
 * open, an alert, of a kind Home shows, and naming this conversation.
 */
export const conversationAlerts = (items: readonly Native.Attention[], agentRef: string): readonly Native.Attention[] =>
  alertsIn(items, agentRef, undefined)

type Answer = (alert: Native.Attention, answer: string, sent: string) => Promise<PromptAnswerResult>
type Act = (alert: Native.Attention, action: AlertAction) => Promise<AlertActionResult>

/**
 * The alerts of an agent's conversation, above its composer. A prompt its harness waits on shows
 * with the answers Fractal can deliver: the one waiting call in full beside `allow` (or a plan beside
 * `approve`), a question's options to choose and send together, a screen prompt's own answers.
 * Anything else is answered in the seat's terminal, and an answer there still wins. An ask or a
 * review gate shows with its own answers (`AnswerAlert`).
 */
export const AgentAlerts = ({
  alerts,
  onAnswer,
  onAct,
}: {
  readonly alerts: readonly Native.Attention[]
  readonly onAnswer?: Answer
  readonly onAct?: Act
}): React.ReactNode => {
  if (alerts.length === 0) return null
  const one = (alert: Native.Attention) =>
    alert.attention_kind === 'harness-prompt'
      // A new prompt or revision behind the same alert starts with nothing chosen.
      ? <PromptAlert key={promptAlertKey(alert)} alert={alert} onAnswer={onAnswer} />
      : <AnswerAlert key={alert.id} alert={alert} onAct={onAct} />
  const [first, ...rest] = alerts
  return (
    <div {...stylex.props(styles.pinned)} data-testid="conversation-alerts">
      <div {...stylex.props(styles.strip)}>
        {alerts.length > 1 ? <div {...stylex.props(styles.count)}>◆ {alerts.length} alerts</div> : null}
        {one(first!)}
        {/* The first alert is open; the others open on a click, as on the phone. */}
        {rest.map((alert) => (
          <details key={alert.id} {...stylex.props(styles.folded)}>
            <summary {...stylex.props(styles.summary)}>◆ {alert.title}</summary>
            {one(alert)}
          </details>
        ))}
      </div>
    </div>
  )
}

const PromptAlert = ({
  alert,
  onAnswer,
}: {
  readonly alert: Native.Attention
  readonly onAnswer?: Answer
}): React.ReactNode => {
  const prompt = promptAnswers(alert)
  const card = alert.presentation?.prompt
  const served = servedPrompt({ presentation: card === undefined ? undefined : { prompt: { ...card, calls: Option.getOrNull(card.calls) } } })
  const shown = prompt === null || served.calls === null ? [] : promptCalls(prompt, served.calls)
  const [chosen, setChosen] = React.useState<number[][]>([])
  const [status, setStatus] = React.useState<string>()
  const [sending, setSending] = React.useState(false)
  const offered = prompt === null || onAnswer === undefined ? [] : servedAnswers(prompt, served)
  const questions = pendingQuestions(shown.filter(call => call.complete === true))
  const gated = prompt?.answers.find((answer) => answer === 'allow' || answer === 'approve' || answer === 'answer')
  const sendable = questions === null ? null : questionAnswer(questions, chosen)
  const choose = (question: number, option: number, multi: boolean) =>
    setChosen((previous) => {
      const next = [...previous]
      const picked = next[question] ?? []
      next[question] = multi
        ? picked.includes(option) ? picked.filter((each) => each !== option) : [...picked, option].sort()
        : [option]
      return next
    })
  const send = (answer: string) => {
    const sent = answer === 'answer' ? sendable : answer
    if (sent === null || onAnswer === undefined) return
    setSending(true)
    setStatus(`Answering ${answerLabel(answer).toLowerCase()}…`)
    void onAnswer(alert, answer, sent).then((result) => {
      setSending(false)
      setStatus(result._tag === 'Sent' ? 'Answered. The alert clears once the harness moves on.' : result.detail)
    })
  }
  return (
    <section {...stylex.props(styles.alert)} aria-label={alert.title}>
      <div {...stylex.props(styles.title)}>◆ {alert.title}</div>
      {alert.detail ? <div {...stylex.props(styles.detail)}>{alert.detail}</div> : null}
      {prompt === null || served.calls === null ? (
        <div {...stylex.props(styles.note)}>{served.calls === null ? `${served.why} ` : ''}Answer it in the agent's terminal.</div>
      ) : questions !== null && gated === 'answer' ? (
        questions.map((question, index) => (
          <div key={index} {...stylex.props(styles.alert)}>
            <div {...stylex.props(styles.title)}>
              {question.header ? `${question.header} · ` : ''}
              {question.question}
              {question.multiSelect ? ' (choose any)' : ''}
            </div>
            <div {...stylex.props(styles.answers)}>
              {question.options.map((option, choice) => {
                const on = (chosen[index] ?? []).includes(choice)
                return (
                  <AlertButton key={choice} tone={on ? 'primary' : 'secondary'} aria-pressed={on} isDisabled={sending}
                    onPress={() => choose(index, choice, question.multiSelect)}>
                    {on ? `✓ ${option.label}` : option.label}
                  </AlertButton>
                )
              })}
            </div>
          </div>
        ))
      ) : (
        shown.map((call) => (
          <pre key={call.id} {...stylex.props(styles.call)}>{[call.tool, ...call.lines].join('\n')}</pre>
        ))
      )}
      {prompt !== null && gated !== undefined && !offered.includes(gated) ? (
        <div {...stylex.props(styles.note)}>
          {prompt.call_id == null
            ? 'Fractal does not say which call this asks about: answer it in the seat\'s terminal.'
            : 'The prompt details are incomplete or unsupported: answer it in the agent\'s terminal.'}
        </div>
      ) : null}
      {offered.length > 0 ? (
        <div {...stylex.props(styles.answers)}>
          {offered.map((answer) => (
            <AlertButton key={answer} tone={answer === 'deny' ? 'danger' : answer === gated ? 'primary' : 'secondary'}
              isDisabled={sending || (answer === 'answer' && sendable === null)} onPress={() => send(answer)}>
              {answer === 'deny' && gated === 'answer' ? 'Decline' : answerLabel(answer)}
            </AlertButton>
          ))}
        </div>
      ) : null}
      {status ? <div role="status" {...stylex.props(styles.note)}>{status}</div> : null}
    </section>
  )
}

/** The agent's prompt alerts from the active data source, answered through its prompt port. */
export const LiveAgentAlerts = ({
  agentRef,
  ux,
}: {
  readonly agentRef: string
  readonly ux?: UxTelemetry
}): React.ReactNode => {
  const source = useDataSource()
  const attention = useAttention()
  const alerts = React.useMemo(
    () => (attention._tag === 'Observed' ? conversationAlerts(attention.value, agentRef) : []),
    [attention, agentRef],
  )
  // From Fractal taking an answer to the paint that no longer shows its alert (Fractal's `alert_landed`).
  const answered = React.useRef(new Map<string, ReturnType<UxTelemetry['beginAlertAnswer']>>())
  const landing = React.useRef<Array<() => void>>([])
  React.useLayoutEffect(() => {
    const shown = new Set<string>(alerts.map((alert) => alert.id))
    for (const [id, operation] of answered.current) {
      if (shown.has(id)) continue
      answered.current.delete(id)
      landing.current.push(operation.landed())
    }
  }, [alerts])
  React.useEffect(() => () => {
    for (const operation of answered.current.values()) operation.cancel()
    for (const cancel of landing.current) cancel()
  }, [])
  const actions = source.alertActions
  const onAct: Act | undefined =
    actions === undefined
      ? undefined
      : (alert, action) => {
          const operation = ux?.beginAlertAnswer()
          return actions.act({ alert, action }).then((result) => {
            if (operation === undefined) return result
            if (result._tag === 'Sent') answered.current.set(alert.id, operation)
            else operation.cancel()
            return result
          })
        }
  const prompts = source.prompts
  const onAnswer: Answer | undefined =
    prompts === undefined
      ? undefined
      : (alert, _answer, sent) => {
          const prompt = promptAnswers(alert)
          return prompt === null
            ? Promise.resolve({ _tag: 'Refused', detail: 'This prompt is answered in the seat\'s terminal.' })
            : prompts.respond({
                alert: { id: alert.id, revision: alert.revision, source_id: alert.source_id, target_id: prompt.target_id, episode: prompt.episode },
                answer: sent,
              })
        }
  return <AgentAlerts alerts={alerts} onAnswer={onAnswer} onAct={onAct} />
}
