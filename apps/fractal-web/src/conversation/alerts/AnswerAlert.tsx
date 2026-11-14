import { markdownBlock, markdownInline } from '@fractal/client/markdown'
import { markdownSource, optionalSource } from '@fractal/client/markdown'
import type * as Native from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import { DateTime, Option } from 'effect'
import * as React from 'react'

import { cleanMessageText } from '../../../../../clients/typescript/fractal-views/conversationView.ts'
import { askDeadline, isRequest, report, spaced } from '../../../../../clients/typescript/fractal-views/requestView.ts'
import { ago } from '../../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DecisionCard, type DecisionDensity, type DecisionPosition } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/DecisionCard.tsx'
import { DecisionMore, type DecisionSubject } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/DecisionMore.tsx'
import type { IconName } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { InlineMarkdown, Markdown } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import type { AlertAction, AlertActionResult } from './alert-action-port.ts'
import { useAlertAnswersStore } from './alertAnswers.ts'
import { KIND_WORDS, alertChoices, alertFrom, answeredLabel, canSend, effectiveChoice, partitionChoices, webLink } from './alertChoices.ts'
import { contextOf, firstLine, isPlainHeading, markdownLine, repeatsHeading, sameWords, useArmed } from './decisionText.ts'

// Asks, review gates and launch approvals, in the one decision card: choosing an answer sends nothing, Send is its own
// deliberate act, so a stray click answers nothing.

/** A request's question: a JSON report as its telling fields, anything else as Markdown. */
const Question = ({ text }: { readonly text: string }): React.ReactNode => {
  const shown = report(text)
  if (shown === null) return <div {...stylex.props(styles.prose)}><Markdown text={markdownBlock(spaced(text))} /></div>
  return (
    <div {...stylex.props(styles.prose)}>
      {shown.before ? <Markdown text={markdownBlock(shown.before)} /> : null}
      <dl {...stylex.props(styles.report)}>
        {shown.rows.map((row) => (
          <React.Fragment key={row.key}>
            <dt {...stylex.props(styles.soft)}>{row.key}</dt>
            <dd {...stylex.props(styles.value, row.tone === 'fault' ? styles.fault : row.tone === 'soft' ? styles.soft : null)}>{row.value}</dd>
          </React.Fragment>
        ))}
      </dl>
      {shown.more > 0 ? <div {...stylex.props(styles.soft)}>… {shown.more} more fields</div> : null}
    </div>
  )
}

/** What a launch would start, as its planner previewed it: goal, steps, agents and gates. */
const LaunchPreview = ({ alert }: { readonly alert: Native.Attention }): React.ReactNode => {
  if (Option.isNone(alert.preview)) {
    return <div {...stylex.props(styles.soft)}>There is no valid preview of this launch yet, so it cannot be approved.{alert.detail ? <> <InlineMarkdown text={markdownInline(cleanMessageText(markdownSource(alert.detail)))} /></> : ''}</div>
  }
  const preview = alert.preview.value
  return (
    <>
      {preview.request_excerpt ? <div {...stylex.props(styles.soft)}>Asked for: <InlineMarkdown text={markdownInline(markdownSource(preview.request_excerpt))} /></div> : null}
      {preview.steps.length > 0 ? (
        <ol {...stylex.props(styles.list)} aria-label="Steps">
          {preview.steps.map((step) => (
            <li key={step.path}>
              <InlineMarkdown text={markdownInline(step.title)} />
              {Option.isSome(step.assignee) ? <span {...stylex.props(styles.soft)}> · {step.assignee.value.replace(/^agent\//, '')}</span> : null}
              {step.depends.length > 0 ? <span {...stylex.props(styles.soft)}> · after {step.depends.join(', ')}</span> : null}
            </li>
          ))}
        </ol>
      ) : null}
      {preview.agents.length > 0 ? (
        <div {...stylex.props(styles.soft)}>
          Agents: {preview.agents.map((agent) => `${agent.id.replace(/^agent\//, '')}${Option.isSome(agent.host) ? ` on ${agent.host.value}` : ''}${Option.isSome(agent.worktree) ? ` in ${agent.worktree.value}` : ''}`).join('; ')}
        </div>
      ) : null}
      <div {...stylex.props(styles.soft)}>
        Gates: {preview.gates.human} for a person, {preview.gates.automated} automated
        {preview.diagnostics_count > 0 ? ` · ${preview.diagnostics_count} diagnostics` : ''}
      </div>
    </>
  )
}

const SUBJECT_ICONS: Readonly<Record<string, IconName>> = { pull_request: 'pull-request', issue: 'alert', document: 'pencil', mission: 'checklist', run: 'gauge', step: 'checklist', agent: 'person', host: 'folder', commit: 'dot', link: 'globe' }
const SUBJECT_KINDS: Readonly<Record<string, string>> = { pull_request: 'Pull request', issue: 'Issue', document: 'Document', mission: 'Mission', run: 'Run', step: 'Step', agent: 'Agent', host: 'Host', commit: 'Commit', link: 'Link' }

/** A request's subjects as rows: a checked web link opens out, a ref the host can open opens in, and any other ref is shown to copy. */
const subjectRows = (alert: Native.Attention, onOpenRef: ((ref: string) => void) | undefined, opensRef: (ref: string) => boolean): readonly DecisionSubject[] =>
  (alert.request?.subjects ?? []).map((subject, index) => {
    const href = webLink(subject.url)
    const ref = subject.ref
    const inner = href === undefined && ref && onOpenRef !== undefined && opensRef(ref)
    const kind = SUBJECT_KINDS[subject.kind]
    return {
      key: `${index}`,
      label: subject.label,
      icon: SUBJECT_ICONS[subject.kind] ?? 'info',
      ...(kind === undefined ? {} : { kind }),
      ...(href === undefined ? {} : { href }),
      ...(inner ? { onOpen: () => onOpenRef!(ref!) } : {}),
      ...(href === undefined && !inner && ref ? { rawRef: ref } : {}),
    }
  })


const Command = ({ children }: { readonly children: React.ReactNode }) => <code {...stylex.props(styles.command)}>{children}</code>

/** Why an alert cannot be answered here and where it can be, with the exact command when there is one. */
const elsewhere = (alert: Native.Attention, step: string | undefined): React.ReactNode => {
  switch (alert.attention_kind) {
    case 'harness-login': return 'Sign in from the seat\'s terminal. One sign-in answers every seat that shares it.'
    case 'model-drift': return 'Nothing to answer: these agents keep working on another model. This clears by itself once each runs its declared model again.'
    case 'revision-approval': return <>A revision isn't approved from the web yet. Run <Command>fractal work revision approve</Command> or <Command>cancel</Command>, as in the terminal app.</>
    case 'fault': return step === undefined
      ? <>This is a fault to act on, not a question. Run <Command>fractal alerts</Command> to see what it offers.</>
      : <>This is a fault to act on, not a question. Retry it with <Command>fractal work retry {step}</Command>.</>
    default: return <>The web can't answer this kind of alert yet. Answer it with <Command>fractal alerts</Command>.</>
  }
}

const READ_ONLY = <>This device is read-only, so it can't answer here. Answer it from a device with control access, or with <Command>fractal alerts</Command>.</>

type Act = (alert: Native.Attention, action: AlertAction) => Promise<AlertActionResult>

/** One ask, gate or launch in the decision card. Answers always show; only a device that can send them can choose. */
export const AnswerAlert = ({
  alert,
  onAct,
  now = Date.now(),
  density = 'docked',
  pager,
  onFold,
  maxBlockSize,
  onBegin,
  focusRef,
  onOpenRef,
  opensRef = () => true,
}: {
  readonly alert: Native.Attention
  readonly onAct?: Act
  readonly now?: number
  readonly density?: DecisionDensity
  readonly pager?: DecisionPosition
  readonly onFold?: () => void
  readonly maxBlockSize?: number
  /** The answer left the card: called as it is sent, before any reply. */
  readonly onBegin?: (alert: Native.Attention) => void
  readonly focusRef?: React.Ref<HTMLElement>
  /** Opens a subject's ref inside the app. */
  readonly onOpenRef?: (ref: string) => void
  /** Which refs that can open; any other stays a chip to copy. */
  readonly opensRef?: (ref: string) => boolean
}): React.ReactNode => {
  const store = useAlertAnswersStore()
  const choices = alertChoices(alert)
  const parts = choices === null ? { options: [] } : partitionChoices(choices)
  const words = store.words(alert.id)
  const chosen = store.selected(alert)
  const selected = chosen ?? parts.options.filter((choice) => choice.recommended === true).map((choice) => choice.key)
  const answer = choices === null ? undefined : effectiveChoice(choices, selected[0], words)
  const from = alertFrom(alert)
  const send = (choice: NonNullable<typeof answer>) => {
    if (onAct === undefined) return
    store.send(alert, answeredLabel(choice, words), () => onAct(alert, choice.action(words)))
    onBegin?.(alert)
  }
  const [armed, arm] = useArmed(alert.id)
  const step = alert.blocked?.step ?? alert.step_run_id?.split('/').pop()
  const deadline = askDeadline(
    { deadline_at: alert.deadline_at === undefined ? undefined : DateTime.formatIso(alert.deadline_at), default_answer: alert.default_answer },
    now,
  )
  const request = alert.request
  const question = optionalSource(request?.question)
  const plain = question !== undefined && isPlainHeading(question)
  const headingText = plain ? question : alert.attention_kind === 'launch-approval' && Option.isSome(alert.preview) ? markdownSource(alert.preview.value.goal) : alert.title
  const detail = cleanMessageText(optionalSource(alert.detail) ?? '')
  const contextSource = optionalSource(request?.summary) ?? (request === undefined && alert.attention_kind !== 'launch-approval' && !(isRequest(alert.attention_kind) && report(detail) !== null) ? detail : '')
  const { line, overflows } = contextOf(contextSource)
  const title = plain && !repeatsHeading(alert.title, question) ? alert.title : undefined
  const whyText = optionalSource(request?.why_person)?.replace(/^why (you|now):\s*/i, '').replace(/^why (you|now):\s*/i, '')
  const why = whyText === undefined || whyText === '' || sameWords(whyText, contextSource, true) || sameWords(whyText, question ?? '', true) ? undefined : whyText
  const evidence = [...(alert.because ? [`Because ${markdownSource(alert.because)}`] : []), ...(request?.reasons ?? []).map(markdownSource)]
  const related = subjectRows(alert, onOpenRef, opensRef)
  const full = density === 'full'
  const contextFull = overflows ? <div {...stylex.props(styles.prose)}><Markdown text={markdownBlock(spaced(contextSource))} /></div> : undefined
  // A differing title is context the heading does not carry. Full density shows the long context beside More's parts already.
  const shownContext = title === undefined && (full || contextFull === undefined) ? undefined : <>{title === undefined ? null : <p {...stylex.props(styles.titleLine)}><InlineMarkdown text={markdownInline(title)} /></p>}{full ? null : contextFull}</>
  const more = why !== undefined || evidence.length > 0 || related.length > 0 || shownContext !== undefined
    ? <DecisionMore clamp={!full} evidence={evidence} related={related} {...(why === undefined ? {} : { why })} {...(shownContext === undefined ? {} : { context: shownContext })} />
    : undefined
  const body = alert.attention_kind === 'launch-approval' ? <LaunchPreview alert={alert} />
    : question !== undefined && !plain ? <Question text={question} />
      : request === undefined && isRequest(alert.attention_kind) && report(detail) !== null ? <Question text={detail} /> : undefined
  const failure = store.failure(alert)
  const options = parts.options
  const shownSelected = answer !== undefined && options.some((choice) => choice.key === answer.key) ? [answer.key] : []
  const movedToWords = words.trim() !== '' && selected.length > 0 && answer?.key !== selected[0]
  const stuck = words.trim() !== '' && answer !== undefined && answer.words === 'none'
  const status = failure !== undefined ? { tone: 'error' as const, text: failure }
    : stuck ? { tone: 'progress' as const, text: 'This asks you to pick one of its answers, so your words can\'t be sent. Clear them to send.' }
      : movedToWords ? { tone: 'progress' as const, text: answer?.key === 'words' ? 'Your words will be sent as the answer.' : `Your words go with “${answer?.label}”.` }
        : undefined
  // A new revision may ask something new: the card starts over, so a choice made on the old one never lets Enter answer this one.
  return <DecisionCard
    key={`${alert.id}\u0000${alert.revision}`}
    density={density}
    label={firstLine(alert.title)}
    data={{ 'data-alert-kind': alert.attention_kind }}
    meta={[from, KIND_WORDS[alert.attention_kind] ?? 'Alert', ...(step ? [`step ${step}`] : []), `waited ${ago(DateTime.formatIso(alert.requested_at), now)}`, ...(deadline ? [deadline] : [])]}
    heading={<InlineMarkdown text={markdownInline(headingText)} />}
    headingText={markdownLine(headingText)}
    {...(line === '' ? {} : { context: line })}
    {...(contextFull === undefined ? {} : { contextFull })}
    {...(more === undefined ? {} : { more })}
    {...(body === undefined ? {} : { body })}
    options={options.map((choice) => ({
      id: choice.key,
      label: choice.label,
      ...(choice.consequence ? { rationale: choice.consequence } : {}),
      ...(choice.recommended === true ? { recommended: request?.recommendation?.reason ? { reason: request.recommendation.reason } : {} } : {}),
      ...(choice.tone === 'danger' ? { danger: true } : {}),
    }))}
    selected={shownSelected}
    onSelect={(ids) => store.setSelected(alert, ids)}
    {...(choices === null ? {} : {
      words: {
        value: words,
        label: `Your answer to ${from}`,
        placeholder: answer?.words === 'required' ? 'What should it do instead?' : parts.words !== undefined ? 'Or tell it something else…' : choices.some((choice) => choice.words === 'optional') ? 'Add a note (optional)…' : 'Or tell it something else…',
        onChange: (value: string) => store.setWords(alert.id, value),
      },
      send: { label: 'Send', disabled: !canSend(answer, words), onPress: () => { if (answer !== undefined) send(answer) } },
    })}
    {...(parts.dismiss === undefined ? {} : {
      dismiss: { label: armed ? `Confirm ${parts.dismiss.label.toLowerCase()}` : parts.dismiss.label, onPress: () => { if (armed) send(parts.dismiss!); else arm() } },
    })}
    {...(choices === null ? { unavailable: elsewhere(alert, step) } : onAct === undefined ? { unavailable: READ_ONLY } : {})}
    {...(status === undefined ? {} : { status })}
    {...(pager === undefined ? {} : { pager })}
    {...(onFold === undefined ? {} : { onFold })}
    {...(maxBlockSize === undefined ? {} : { maxBlockSize })}
    {...(focusRef === undefined ? {} : { focusRef })}
  />
}

const styles = stylex.create({
  prose: { color: c.fg, fontSize: t.uiSize, lineHeight: t.uiLeading, overflowWrap: 'anywhere' },
  report: { margin: 0, display: 'grid', gridTemplateColumns: 'max-content 1fr', columnGap: s.md, fontSize: t.metaSize },
  fault: { color: c.dangerFg },
  soft: { color: c.fgSoft },
  value: { margin: 0 },
  list: { margin: 0, paddingInlineStart: s.xxl },
  titleLine: { margin: 0 },
  command: { fontFamily: t.fontMono, color: c.fg },
})
