import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import { DateTime, Option } from 'effect'
import * as React from 'react'

import { answerLabel, pendingQuestions, promptAlertKey, promptAnswers, promptCalls, questionAnswer, servedAnswers, servedPrompt } from '../../../../../clients/typescript/fractal-views/homeView.ts'
import { ago } from '../../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, radiusVars as r, spaceVars as sp, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DecisionCard, type DecisionDensity, type DecisionPosition } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/DecisionCard.tsx'
import { useAlertAnswersStore } from './alertAnswers.ts'
import { alertFrom } from './alertChoices.ts'
import { contextOf, useArmed } from './decisionText.ts'
import type { PromptAnswerResult } from './prompt-answer-port.ts'

type Answer = (alert: Native.Attention, answer: string, sent: string) => Promise<PromptAnswerResult>

/**
 * A prompt a harness waits on, in the decision card, with the answers Fractal can deliver: the one waiting call in full
 * beside `allow` (or a plan beside `approve`), a screen prompt's own answers, or a question's options, one question at
 * a time and sent together. Anything else is answered in the seat's terminal, and an answer there still wins.
 */
export const PromptAlert = ({ alert, onAnswer, now = Date.now(), density = 'docked', pager, onFold, maxBlockSize, onBegin, focusRef }: {
  readonly alert: Native.Attention
  readonly onAnswer?: Answer
  readonly now?: number
  readonly density?: DecisionDensity
  readonly pager?: DecisionPosition
  readonly onFold?: () => void
  readonly maxBlockSize?: number
  readonly onBegin?: (alert: Native.Attention) => void
  readonly focusRef?: React.Ref<HTMLElement>
}): React.ReactNode => {
  const store = useAlertAnswersStore()
  const prompt = promptAnswers(alert)
  // A new prompt or revision behind the same alert starts with nothing chosen.
  const asked = { id: alert.id, revision: promptAlertKey(alert) }
  const card = alert.presentation?.prompt
  const served = servedPrompt({ presentation: card === undefined ? undefined : { prompt: { ...card, calls: Option.getOrNull(card.calls) } } })
  const shown = prompt === null || served.calls === null ? [] : promptCalls(prompt, served.calls)
  const offered = prompt === null ? [] : servedAnswers(prompt, served)
  const live = onAnswer !== undefined
  const allQuestions = pendingQuestions(shown.filter((call) => call.complete === true))
  const gated = prompt?.answers.find((answer) => answer === 'allow' || answer === 'approve' || answer === 'answer')
  const questions = gated === 'answer' ? allQuestions : null
  const step = Math.min(store.step(asked), (questions?.length ?? 1) - 1)
  const chosenPositions = (questions ?? []).map((_, index) => (store.selected(asked, String(index)) ?? []).map(Number))
  const sendable = questions === null ? null : questionAnswer(questions, chosenPositions)
  const question = questions?.[step]
  const [armed, arm] = useArmed(alert.id)
  const send = (answer: string) => {
    const sent = answer === 'answer' ? sendable : answer
    if (sent === null || onAnswer === undefined) return
    const label = answer === 'answer' && questions !== null
      ? questions.flatMap((each, index) => (chosenPositions[index] ?? []).map((position) => each.options[position]?.label ?? '')).join(', ')
      : answer === 'deny' && gated === 'answer' ? 'Decline' : answerLabel(answer)
    store.send(alert, label, () => onAnswer(alert, answer, sent))
    onBegin?.(alert)
  }
  const main = (selected: readonly string[]) => store.setSelected(asked, selected, question === undefined ? 'main' : String(step))
  // Declining a question is the way out of it, as Dismiss is for an ask; everything else is one of the prompt's answers.
  const decline = questions !== null && offered.includes('deny') ? 'deny' : undefined
  const options = question !== undefined
    ? question.options.map((option, index) => ({ id: String(index), label: option.label, ...(option.description ? { rationale: option.description } : {}) }))
    : offered.filter((answer) => answer !== 'answer').map((answer) => ({ id: answer, label: answerLabel(answer), ...(answer === 'deny' ? { danger: true } : {}) }))
  const selected = store.selected(asked, question === undefined ? 'main' : String(step)) ?? []
  const last = questions === null || step === questions.length - 1
  const incomplete = prompt === null || served.calls === null
    ? `${served.calls === null ? `${served.why} ` : ''}Answer it in the agent's terminal.`
    : gated !== undefined && !offered.includes(gated)
      ? prompt.call_id == null ? 'Fractal does not say which call this asks about: answer it in the seat\'s terminal.' : 'The prompt details are incomplete or unsupported: answer it in the agent\'s terminal.'
      : undefined
  const failure = store.failure(alert)
  const { line, overflows } = contextOf(alert.detail ?? '')
  const heading = question === undefined ? alert.title : `${question.header ? `${question.header} · ` : ''}${question.question}${question.multiSelect ? ' (choose any)' : ''}`
  const body = question === undefined && shown.length > 0
    ? shown.map((call) => <pre key={call.id} {...stylex.props(styles.call)}>{[call.tool, ...call.lines].join('\n')}</pre>)
    : undefined
  // Each revision and each question starts the card over, so a choice made on another never lets Enter answer this one.
  return <DecisionCard
    key={`${asked.id}\u0000${asked.revision}\u0000${step}`}
    density={density}
    label={alert.title}
    data={{ 'data-alert-kind': alert.attention_kind }}
    meta={[alertFrom(alert), 'Prompt', `waited ${ago(DateTime.formatIso(alert.requested_at), now)}`]}
    heading={heading}
    headingText={heading}
    {...(line === '' || line === alert.title ? {} : { context: line })}
    {...(overflows ? { contextFull: <div {...stylex.props(styles.detail)}>{alert.detail}</div> } : {})}
    {...(body === undefined ? {} : { body })}
    options={options}
    mode={question?.multiSelect === true ? 'any' : 'one'}
    selected={selected}
    onSelect={main}
    {...(questions !== null && questions.length > 1 ? { stepper: { index: step, count: questions.length, onPrevious: () => store.setStep(asked, step - 1), onNext: () => store.setStep(asked, step + 1) } } : {})}
    {...(decline === undefined ? {} : { dismiss: { label: armed ? 'Confirm decline' : 'Decline', onPress: () => { if (armed) send(decline); else arm() } } })}
    {...(incomplete !== undefined && offered.length === 0 ? { unavailable: incomplete } : !live ? { unavailable: <>This device is read-only, so it can't answer here. Answer it in the agent's terminal.</> } : {})}
    {...(offered.length === 0 ? {} : {
      send: questions !== null
        ? { label: last ? 'Send' : 'Next', disabled: last ? sendable === null : selected.length === 0, onPress: () => { if (last) send('answer'); else store.setStep(asked, step + 1) } }
        : { label: 'Send', disabled: selected[0] === undefined, onPress: () => { if (selected[0] !== undefined) send(selected[0]) } },
    })}
    {...(failure !== undefined ? { status: { tone: 'error' as const, text: failure } } : incomplete !== undefined && offered.length > 0 ? { status: { tone: 'progress' as const, text: incomplete } } : {})}
    {...(pager === undefined ? {} : { pager })}
    {...(onFold === undefined ? {} : { onFold })}
    {...(maxBlockSize === undefined ? {} : { maxBlockSize })}
    {...(focusRef === undefined ? {} : { focusRef })}
  />
}

const styles = stylex.create({
  detail: { whiteSpace: 'pre-wrap', color: c.fgSoft },
  call: { margin: 0, padding: sp.md, borderRadius: r.sm, backgroundColor: c.codeBg, color: c.fg, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: t.metaLeading },
})
