import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Checkbox, CheckboxGroup, Input, Label, Radio, RadioGroup, Tab, TabList, TabPanel, Tabs, TextField, VisuallyHidden } from 'react-aria-components'
import { surfaceVars as surface, textVars as text, borderVars as border, accentVars as accent, geometryVars as g, geometryNumbers, radiusVars as r, spaceVars as s, typeVars as t } from '../composition-tokens.stylex'
import { Button, IconButton, Tooltip } from '../composition/Controls'
import { Icon } from '../composition/Icons'
import { Markdown } from '../composition/Markdown'
import type { AnswerDraft, Answers, OptionView, Preview, QuestionFormProps, QuestionState, QuestionView } from './model'

/** The automatic free-text row every `allowOther` question receives, worded as the omp ask tool words it. */
export const OTHER_LABEL = 'Other (type your own)'

type FocusTarget = { readonly _tag: 'options' } | { readonly _tag: 'option'; readonly index: number } | { readonly _tag: 'other' }
interface FocusRequest { readonly questionId: string; readonly target: FocusTarget }
type StyleArg = Parameters<typeof stylex.props>[number]

const EMPTY_DRAFT: AnswerDraft = { selected: [] }
const OPTIONS_TARGET: FocusTarget = { _tag: 'options' }
const OTHER_VALUE = 'other'
const optionValue = (index: number) => `option:${index}`
const optionIndexOf = (value: string) => value.startsWith('option:') ? Number(value.slice('option:'.length)) : undefined
const draftFor = (answers: Answers, id: string) => answers[id] ?? EMPTY_DRAFT
const filled = (value: string | undefined) => {
  const trimmed = value?.trim()
  return trimmed === undefined || trimmed === '' ? undefined : trimmed
}
const usesCheckboxes = (question: QuestionView) => question.kind === 'multi'
const className = (...styles: StyleArg[]) => stylex.props(...styles).className ?? ''
const clock = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })

/** A question counts as answered once it has a chosen option or non-empty typed text. */
function isQuestionAnswered(question: QuestionView, draft: AnswerDraft): boolean {
  if (filled(draft.other) !== undefined) return true
  return question.kind !== 'text' && draft.selected.length > 0
}

/** The submitted payload: chosen ids, trimmed Other text and trimmed notes; empty text is omitted. */
function settle(questions: readonly QuestionView[], answers: Answers): Answers {
  return Object.fromEntries(questions.map(question => {
    const draft = draftFor(answers, question.id)
    const other = filled(draft.other)
    const note = question.allowNote ? filled(draft.note) : undefined
    const answer: AnswerDraft = {
      selected: question.kind === 'text' ? [] : draft.selected,
      ...(other === undefined ? {} : { other }),
      ...(note === undefined ? {} : { note }),
      ...(draft.attachments === undefined ? {} : { attachments: draft.attachments }),
    }
    return [question.id, answer]
  }))
}

function findFocusTarget(root: HTMLElement | null, request: FocusRequest): HTMLElement | null {
  const panel = root?.querySelector<HTMLElement>(`[data-question-panel="${CSS.escape(request.questionId)}"]`)
  if (panel == null) return null
  const target = request.target
  if (target._tag === 'other') return panel.querySelector<HTMLElement>('[data-question-stop="other"] input')
  const options = panel.querySelector<HTMLElement>('[data-question-stop="options"]')
  if (options === null) return null
  if (target._tag === 'option') return options.querySelector<HTMLElement>(`input[value="${optionValue(target.index)}"]`)
  return options.querySelector<HTMLElement>('input:checked') ?? options.querySelector<HTMLElement>('input')
}

/** Measured form width decides the stacked layout, so a narrow lane stacks even inside a wide window. */
function useStacked(ref: React.RefObject<HTMLElement | null>) {
  const [stacked, setStacked] = React.useState(false)
  React.useLayoutEffect(() => {
    const element = ref.current
    if (element === null) return
    const measure = () => setStacked(element.getBoundingClientRect().width < geometryNumbers.questionStack)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return stacked
}

function Clock({ at, prefix }: { at: string; prefix: string }) {
  const time = Date.parse(at)
  if (!Number.isFinite(time)) return null
  return <span {...stylex.props(styles.meta)}>{prefix} <time dateTime={at}>{clock.format(time)}</time></span>
}

const STATE_LABEL: Record<QuestionState['_tag'], string> = {
  pending: 'Waiting for you',
  submitting: 'Submitting',
  answered: 'Submitted',
  'answered-elsewhere': 'Answered elsewhere',
  expired: 'Expired',
  cancelled: 'Cancelled',
  failed: 'Not sent',
}

/**
 * Interactive question form for harness asks (parity with the omp ask tool): radio or checkbox options,
 * an automatic Other row, a per-question note, Recommended badges and markdown option previews. Several
 * questions share a tab strip whose header chips mark answered questions; it stacks below
 * `geometryNumbers.questionStack`. Keyboard: arrows move, 1–9 pick, Tab from a question's last control
 * moves to the next question, Enter submits (or moves to the next unanswered question). The host owns
 * the request state and transport; drafts are controlled when `draft` is passed.
 */
export function QuestionForm({ request, variant, onSubmit, onDecline, onChat, onRetry, onOpenTerminal, onJump, draft, onDraftChange, autoFocus = false }: QuestionFormProps) {
  const rootRef = React.useRef<HTMLElement>(null)
  const stacked = useStacked(rootRef)
  const titleId = React.useId()
  const { questions, state } = request
  const [localDraft, setLocalDraft] = React.useState<Answers>(() => draft ?? {})
  const answers = draft ?? localDraft
  const [chosenId, setActiveId] = React.useState(() => questions[0]?.id)
  const activeId = questions.some(question => question.id === chosenId) ? chosenId : questions[0]?.id
  const [previewIndex, setPreviewIndex] = React.useState<Readonly<Record<string, number>>>({})
  const otherMemory = React.useRef<Record<string, string>>({})
  const focusRequest = React.useRef<FocusRequest | null>(null)

  const interactive = state._tag === 'pending' || state._tag === 'submitting' || state._tag === 'failed'
  const busy = state._tag === 'submitting'
  const canAct = request.answerable && (state._tag === 'pending' || state._tag === 'failed')
  const answeredCount = questions.filter(question => isQuestionAnswered(question, draftFor(answers, question.id))).length
  const complete = questions.every(question => question.required === false || isQuestionAnswered(question, draftFor(answers, question.id)))

  const requestFocus = (questionId: string, target: FocusTarget) => {
    const request = { questionId, target }
    const element = findFocusTarget(rootRef.current, request)
    if (element === null) focusRequest.current = request
    else element.focus()
  }
  React.useLayoutEffect(() => {
    const pending = focusRequest.current
    if (pending === null) return
    const element = findFocusTarget(rootRef.current, pending)
    if (element === null) return
    focusRequest.current = null
    element.focus()
  })
  React.useEffect(() => {
    const first = questions[0]
    if (autoFocus && canAct && first !== undefined) requestFocus(first.id, OPTIONS_TARGET)
    // Mount-only: autofocus never steals focus on later renders.
  }, [])

  const update = (question: QuestionView, change: (draft: AnswerDraft) => AnswerDraft) => {
    const next = { ...answers, [question.id]: change(draftFor(answers, question.id)) }
    setLocalDraft(next)
    onDraftChange?.(next)
  }
  const rememberOther = (question: QuestionView, current: AnswerDraft) => {
    if (current.other !== undefined) otherMemory.current[question.id] = current.other
  }
  const recalledOther = (question: QuestionView, current: AnswerDraft) => current.other ?? otherMemory.current[question.id] ?? ''
  const chooseSingle = (question: QuestionView, value: string) => update(question, current => {
    if (value === OTHER_VALUE) return { ...current, selected: [], other: recalledOther(question, current) }
    const option = question.options[optionIndexOf(value) ?? -1]
    if (option === undefined) return current
    rememberOther(question, current)
    return { ...current, selected: [option.id], other: undefined }
  })
  const chooseMany = (question: QuestionView, values: readonly string[]) => update(question, current => {
    const selected = question.options.filter((_, index) => values.includes(optionValue(index))).map(option => option.id)
    if (values.includes(OTHER_VALUE)) return { ...current, selected, other: recalledOther(question, current) }
    rememberOther(question, current)
    return { ...current, selected, other: undefined }
  })
  const typeOther = (question: QuestionView, value: string) => update(question, current => ({ ...current, selected: usesCheckboxes(question) ? current.selected : [], other: value }))
  const typeNote = (question: QuestionView, value: string) => update(question, current => ({ ...current, note: value }))

  /** 1–9 picks the numbered row: options first, then the Other row. */
  const pick = (question: QuestionView, index: number) => {
    if (question.kind === 'text') return
    const current = draftFor(answers, question.id)
    const isOther = question.allowOther && index === question.options.length
    if (!isOther && index >= question.options.length) return
    const value = isOther ? OTHER_VALUE : optionValue(index)
    if (usesCheckboxes(question)) {
      const values = [...current.selected.map(id => optionValue(question.options.findIndex(option => option.id === id))), ...(current.other === undefined ? [] : [OTHER_VALUE])]
      const turningOn = !values.includes(value)
      chooseMany(question, turningOn ? [...values, value] : values.filter(entry => entry !== value))
      requestFocus(question.id, isOther && turningOn ? { _tag: 'other' } : { _tag: 'option', index })
      return
    }
    chooseSingle(question, value)
    requestFocus(question.id, isOther ? { _tag: 'other' } : { _tag: 'option', index })
  }
  const submit = () => {
    if (!canAct || !complete) return
    onSubmit?.(settle(questions, answers))
  }
  /** Enter submits a complete form; otherwise it moves to the next unanswered required question. */
  const advanceOrSubmit = () => {
    if (complete) return submit()
    const start = questions.findIndex(question => question.id === activeId)
    const ordered = [...questions.slice(start + 1), ...questions.slice(0, start + 1)]
    const next = ordered.find(question => question.required !== false && !isQuestionAnswered(question, draftFor(answers, question.id)))
    if (next === undefined) return
    setActiveId(next.id)
    requestFocus(next.id, OPTIONS_TARGET)
  }
  /** Tab moves through a question's stops (options, Other, note); from the last stop it opens the next question. */
  const tabFrom = (event: React.KeyboardEvent<HTMLElement>, panel: HTMLElement, question: QuestionView) => {
    const stops = [...panel.querySelectorAll<HTMLElement>('[data-question-stop]')]
    const current = (event.target as HTMLElement).closest<HTMLElement>('[data-question-stop]')
    const at = current === null ? -1 : stops.indexOf(current)
    if (at === -1) return
    if (at < stops.length - 1) {
      const next = stops[at + 1]?.querySelector<HTMLElement>('input')
      if (current?.dataset['questionStop'] !== 'options' || next == null) return
      event.preventDefault()
      next.focus()
      return
    }
    const following = questions[questions.indexOf(question) + 1]
    if (following === undefined) return
    event.preventDefault()
    setActiveId(following.id)
    requestFocus(following.id, OPTIONS_TARGET)
  }
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (!canAct || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return
    const target = event.target as HTMLElement
    if (event.key === 'Enter' && target instanceof HTMLInputElement) {
      event.preventDefault()
      advanceOrSubmit()
      return
    }
    const panel = target.closest<HTMLElement>('[data-question-panel]')
    const question = questions.find(entry => entry.id === (panel?.dataset['questionPanel'] ?? activeId))
    if (question === undefined) return
    if (event.key === 'Tab') {
      if (!event.shiftKey && panel !== null) tabFrom(event, panel, question)
      return
    }
    const typing = target instanceof HTMLInputElement && target.type !== 'checkbox' && target.type !== 'radio'
    if (typing || target instanceof HTMLTextAreaElement) return
    if (/^[1-9]$/.test(event.key)) {
      event.preventDefault()
      pick(question, Number(event.key) - 1)
      return
    }
    if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && target instanceof HTMLInputElement && target.type === 'checkbox') {
      const boxes = [...(target.closest('[data-question-stop="options"]')?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? [])]
      const at = boxes.indexOf(target)
      const next = boxes[(at + (event.key === 'ArrowDown' ? 1 : boxes.length - 1)) % boxes.length]
      if (next === undefined) return
      event.preventDefault()
      next.focus()
    }
  }
  const showPreviewFor = (questionId: string, index: number) => setPreviewIndex(current => current[questionId] === index ? current : { ...current, [questionId]: index })
  const onFocus = (event: React.FocusEvent<HTMLElement>) => {
    const target = event.target
    const questionId = target.closest<HTMLElement>('[data-question-panel]')?.dataset['questionPanel']
    const index = target instanceof HTMLInputElement ? optionIndexOf(target.value) : undefined
    if (questionId !== undefined && index !== undefined) showPreviewFor(questionId, index)
  }

  const renderQuestion = (question: QuestionView, index: number) => <QuestionPanel
    key={question.id}
    question={question}
    draft={draftFor(answers, question.id)}
    showHeader={questions.length === 1}
    stacked={stacked}
    isDisabled={busy}
    isReadOnly={!request.answerable}
    previewIndex={previewIndex[question.id]}
    promptId={`${titleId}-q${index}`}
    onPreview={index => showPreviewFor(question.id, index)}
    onSingle={value => chooseSingle(question, value)}
    onMany={values => chooseMany(question, values)}
    onOther={value => typeOther(question, value)}
    onNote={value => typeNote(question, value)}
  />

  const title = request.title ?? (questions.length > 1 ? `${questions.length} questions` : 'Question')
  return <section
    ref={rootRef}
    aria-labelledby={titleId}
    data-question-form=""
    data-state={state._tag}
    data-variant={variant}
    data-stacked={stacked}
    onKeyDown={onKeyDown}
    onFocus={onFocus}
    {...stylex.props(styles.form, variant === 'pinned' ? styles.pinned : styles.card)}
  >
    <div {...stylex.props(styles.header)}>
      <span {...stylex.props(styles.glyph)}><Icon name="message" /></span>
      <span id={titleId} {...stylex.props(styles.title)}>{title}</span>
      <span {...stylex.props(styles.meta)}>from {request.agentLabel}</span>
      <div {...stylex.props(styles.headerEnd)}>
        {state._tag === 'pending' && state.expiresAt !== undefined && <Clock at={state.expiresAt} prefix="Expires" />}
        {(state._tag === 'answered' || state._tag === 'answered-elsewhere' || state._tag === 'expired' || state._tag === 'cancelled') && <Clock at={state.at} prefix={state._tag === 'expired' ? 'Expired at' : 'At'} />}
        <span data-question-state-chip="" {...stylex.props(styles.chip)}>{busy && <Icon name="spinner" spinning size={12} />}{STATE_LABEL[state._tag]}</span>
        {onOpenTerminal !== undefined && <Tooltip label="Open the agent's terminal"><IconButton icon="drawer" label="Open the agent's terminal" onPress={onOpenTerminal} /></Tooltip>}
        {onJump !== undefined && <Tooltip label="Jump to the conversation"><IconButton icon="chevron-right" label="Jump to the conversation" onPress={onJump} /></Tooltip>}
      </div>
    </div>
    {interactive ? <>
      {questions.length > 1 ? <Tabs
        selectedKey={activeId ?? null}
        onSelectionChange={key => setActiveId(String(key))}
        orientation={stacked ? 'vertical' : 'horizontal'}
        {...stylex.props(styles.tabs)}
      >
        <TabList aria-label="Questions" {...stylex.props(styles.tabList, stacked && styles.tabListStacked)}>
          {questions.map((question, index) => {
            const answered = isQuestionAnswered(question, draftFor(answers, question.id))
            return <Tab key={question.id} id={question.id} className={({ isSelected, isFocusVisible }) => className(styles.tab, stacked && styles.tabStacked, isSelected && styles.tabSelected, isFocusVisible && styles.focusRing)}>
              <span {...stylex.props(styles.tabLabel, stacked && styles.tabLabelStacked)}>{question.header ?? `Question ${index + 1}`}</span>
              <span data-question-tab-marker={answered ? 'answered' : 'open'} {...stylex.props(styles.marker, answered && styles.markerAnswered)}>{answered && <Icon name="check" size={12} />}</span>
              {answered && <VisuallyHidden>, answered</VisuallyHidden>}
            </Tab>
          })}
        </TabList>
        {questions.map((question, index) => <TabPanel key={question.id} id={question.id} {...stylex.props(styles.tabPanel)}>{renderQuestion(question, index)}</TabPanel>)}
      </Tabs> : questions.map(renderQuestion)}
      {state._tag === 'failed' && <p role="alert" {...stylex.props(styles.problem)}><Icon name="alert" size={12} />Not sent: {state.error}</p>}
      <div {...stylex.props(styles.footer)}>
        {questions.length > 1 && <span aria-live="polite" {...stylex.props(styles.meta)}>{answeredCount} of {questions.length} answered</span>}
        {canAct ? <span {...stylex.props(styles.hints)}>
          <span><kbd {...stylex.props(styles.kbd)}>1</kbd>–<kbd {...stylex.props(styles.kbd)}>9</kbd> pick</span>
          {questions.length > 1 && <span><kbd {...stylex.props(styles.kbd)}>Tab</kbd> next question</span>}
          <span><kbd {...stylex.props(styles.kbd)}>Enter</kbd> submit</span>
        </span> : !request.answerable && request.readOnlyReason !== undefined && <span {...stylex.props(styles.meta)}>{request.readOnlyReason}</span>}
        {request.answerable && <div {...stylex.props(styles.actions)}>
          {onDecline !== undefined && <Button isDisabled={busy} onPress={onDecline}>{request.declineLabel ?? 'Skip'}</Button>}
          {request.allowChatRedirect && onChat !== undefined && <Button isDisabled={busy} onPress={onChat}>Chat about this</Button>}
          {state._tag === 'failed' && onRetry !== undefined
            ? <Button variant="outline" size="md" onPress={onRetry}>Retry</Button>
            : <Button variant="outline" size="md" isDisabled={busy || !complete} onPress={submit}>{busy ? 'Submitting' : 'Submit'}</Button>}
        </div>}
      </div>
    </> : <Settled state={state} questions={questions} />}
  </section>
}

function Settled({ state, questions }: { state: Exclude<QuestionState, { _tag: 'pending' | 'submitting' | 'failed' }>; questions: readonly QuestionView[] }) {
  const line = (() => {
    switch (state._tag) {
      case 'answered': return `Submitted by ${state.by}.`
      case 'answered-elsewhere': return `Answered in ${state.where}.${state.draftDiscarded ? ' Your draft here was discarded.' : ''}`
      case 'expired': return state.autoAnswer === undefined ? 'Expired before an answer was submitted.' : 'Expired; the agent continued with the automatic answers below.'
      case 'cancelled': return `Cancelled: ${state.reason}`
    }
  })()
  const answers = state._tag === 'answered' ? state.answers : state._tag === 'answered-elsewhere' ? state.answers : state._tag === 'expired' ? state.autoAnswer : undefined
  return <>
    <p data-question-settled="" {...stylex.props(styles.settledLine)}>{line}</p>
    <dl {...stylex.props(styles.summary)}>
      {questions.map(question => {
        const answer = answers?.[question.id]
        const note = filled(answer?.note)
        return <div key={question.id} data-question-summary={question.id} {...stylex.props(styles.summaryRow)}>
          <dt {...stylex.props(styles.summaryPrompt)}>{question.header !== undefined && <span {...stylex.props(styles.chip)}>{question.header}</span>}{question.prompt}</dt>
          <dd {...stylex.props(styles.summaryAnswer)}>{answerText(question, answer)}</dd>
          {note !== undefined && <dd {...stylex.props(styles.summaryNote)}>Note: {note}</dd>}
        </div>
      })}
    </dl>
  </>
}

function answerText(question: QuestionView, answer: AnswerDraft | undefined): string {
  if (answer === undefined) return 'No answer'
  const labels = answer.selected.flatMap(id => question.options.find(option => option.id === id)?.label ?? [])
  const other = filled(answer.other)
  const parts = other === undefined ? labels : [...labels, `Other: ${other}`]
  return parts.length === 0 ? 'No answer' : parts.join(', ')
}

interface QuestionPanelProps {
  readonly question: QuestionView
  readonly draft: AnswerDraft
  readonly showHeader: boolean
  readonly stacked: boolean
  readonly isDisabled: boolean
  readonly isReadOnly: boolean
  readonly previewIndex: number | undefined
  readonly promptId: string
  readonly onPreview: (index: number) => void
  readonly onSingle: (value: string) => void
  readonly onMany: (values: readonly string[]) => void
  readonly onOther: (value: string) => void
  readonly onNote: (value: string) => void
}

function QuestionPanel({ question, draft, showHeader, stacked, isDisabled, isReadOnly, previewIndex, promptId, onPreview, onSingle, onMany, onOther, onNote }: QuestionPanelProps) {
  const hasPreview = question.options.some(option => option.preview !== undefined)
  const selectedIndexes = draft.selected.map(id => question.options.findIndex(option => option.id === id)).filter(index => index >= 0)
  const fallbackIndex = selectedIndexes[0] ?? Math.max(0, question.options.findIndex(option => option.recommended === true))
  const preview = question.options[previewIndex ?? fallbackIndex]
  const otherOpen = question.allowOther && draft.other !== undefined
  const rows = question.options.map((option, index) => ({ option, index, value: optionValue(index) }))
  const onPointerOver = (event: React.PointerEvent<HTMLElement>) => {
    const index = Number((event.target as HTMLElement).closest<HTMLElement>('[data-option-index]')?.dataset['optionIndex'])
    if (Number.isInteger(index) && index < question.options.length) onPreview(index)
  }
  const rowClass = ({ isSelected, isFocusVisible }: { isSelected: boolean; isFocusVisible: boolean }) => className(styles.option, isSelected && styles.optionSelected, isFocusVisible && styles.focusRing, (isReadOnly || isDisabled) && styles.optionInert)
  const content = (row: { option: OptionView; index: number }, isSelected: boolean) => <OptionContent option={row.option} index={row.index} isSelected={isSelected} multi={usesCheckboxes(question)} />

  const control = question.kind === 'text'
    ? <TextField aria-labelledby={promptId} value={draft.other ?? ''} onChange={onOther} isDisabled={isDisabled} isReadOnly={isReadOnly}>
        <Input type={question.secret === true ? 'password' : 'text'} placeholder={question.placeholder ?? 'Type your answer'} {...stylex.props(styles.input)} />
      </TextField>
    : usesCheckboxes(question)
      ? <CheckboxGroup aria-labelledby={promptId} value={[...selectedIndexes.map(optionValue), ...(otherOpen ? [OTHER_VALUE] : [])]} onChange={onMany} isDisabled={isDisabled} isReadOnly={isReadOnly} {...stylex.props(styles.options)}>
          {rows.map(row => <Checkbox key={row.option.id} value={row.value} className={rowClass}>{({ isSelected }) => content(row, isSelected)}</Checkbox>)}
          {question.allowOther && <Checkbox value={OTHER_VALUE} className={rowClass}>{({ isSelected }) => <OtherContent index={question.options.length} isSelected={isSelected} multi />}</Checkbox>}
        </CheckboxGroup>
      : <RadioGroup aria-labelledby={promptId} value={otherOpen ? OTHER_VALUE : selectedIndexes[0] === undefined ? null : optionValue(selectedIndexes[0])} onChange={onSingle} isDisabled={isDisabled} isReadOnly={isReadOnly} {...stylex.props(styles.options)}>
          {rows.map(row => <Radio key={row.option.id} value={row.value} className={rowClass}>{({ isSelected }) => content(row, isSelected)}</Radio>)}
          {question.allowOther && <Radio value={OTHER_VALUE} className={rowClass}>{({ isSelected }) => <OtherContent index={question.options.length} isSelected={isSelected} multi={false} />}</Radio>}
        </RadioGroup>

  return <div data-question-panel={question.id} {...stylex.props(styles.question)}>
    <div {...stylex.props(styles.promptRow)}>
      {showHeader && question.header !== undefined && <span {...stylex.props(styles.chip)}>{question.header}</span>}
      <span id={promptId} {...stylex.props(styles.prompt)}>{question.prompt}</span>
    </div>
    {question.subject?._tag === 'markdown' && <div {...stylex.props(styles.subject)}><Markdown text={question.subject.text} /></div>}
    <div {...stylex.props(styles.body, hasPreview && !stacked && styles.bodyBeside)}>
      <div {...stylex.props(styles.column)}>
        <div data-question-stop="options" onPointerOver={onPointerOver}>{control}</div>
        {otherOpen && <div data-question-stop="other" {...stylex.props(styles.otherField)}>
          <TextField aria-label={`Other answer: ${question.header ?? question.prompt}`} value={draft.other ?? ''} onChange={onOther} isDisabled={isDisabled} isReadOnly={isReadOnly}>
            <Input placeholder={question.placeholder ?? 'Type your own answer'} {...stylex.props(styles.input)} />
          </TextField>
        </div>}
      </div>
      {hasPreview && preview !== undefined && <figure data-question-preview={preview.id} {...stylex.props(styles.preview)}>
        <figcaption {...stylex.props(styles.meta)}>Preview · {preview.label}</figcaption>
        <PreviewBody preview={preview.preview} />
      </figure>}
    </div>
    {question.allowNote && <div data-question-stop="note">
      <TextField value={draft.note ?? ''} onChange={onNote} isDisabled={isDisabled} isReadOnly={isReadOnly} {...stylex.props(styles.note)}>
        <Label {...stylex.props(styles.meta)}>Note</Label>
        <Input placeholder="Add a note for the agent (optional)" {...stylex.props(styles.input)} />
      </TextField>
    </div>}
  </div>
}

const PREVIEW_KIND: Record<Exclude<Preview['_tag'], 'markdown'>, string> = { command: 'Command', diff: 'Diff', url: 'Link' }
function PreviewBody({ preview }: { preview: Preview | undefined }) {
  if (preview === undefined) return <p {...stylex.props(styles.previewEmpty)}>No preview for this option.</p>
  if (preview._tag === 'markdown') return <Markdown text={preview.text} />
  return <p {...stylex.props(styles.previewEmpty)}>{PREVIEW_KIND[preview._tag]} previews are not shown in this form yet.</p>
}

function Indicator({ isSelected, multi }: { isSelected: boolean; multi: boolean }) {
  return <span aria-hidden="true" {...stylex.props(styles.indicator, multi ? styles.box : styles.ring, isSelected && styles.indicatorSelected)}>
    {isSelected && (multi ? <Icon name="check" size={12} /> : <span {...stylex.props(styles.dot)} />)}
  </span>
}

function OptionContent({ option, index, isSelected, multi }: { option: OptionView; index: number; isSelected: boolean; multi: boolean }) {
  return <div data-option-index={index} {...stylex.props(styles.optionRow)}>
    <Indicator isSelected={isSelected} multi={multi} />
    <span {...stylex.props(styles.optionText)}>
      <span {...stylex.props(styles.optionLabel)}>
        {option.label}
        {option.recommended === true && <span data-recommended="" {...stylex.props(styles.badge)}>Recommended</span>}
        {option.tone === 'danger' && <span {...stylex.props(styles.danger)}><Icon name="alert" size={12} label="Destructive" /></span>}
      </span>
      {option.description !== undefined && <span {...stylex.props(styles.description)}>{option.description}</span>}
    </span>
    {index < 9 && <span aria-hidden="true" {...stylex.props(styles.keycap)}>{index + 1}</span>}
  </div>
}

function OtherContent({ index, isSelected, multi }: { index: number; isSelected: boolean; multi: boolean }) {
  return <div data-option-index={index} {...stylex.props(styles.optionRow)}>
    <Indicator isSelected={isSelected} multi={multi} />
    <span {...stylex.props(styles.optionText)}><span {...stylex.props(styles.optionLabel)}>{OTHER_LABEL}</span></span>
    {index < 9 && <span aria-hidden="true" {...stylex.props(styles.keycap)}>{index + 1}</span>}
  </div>
}

const styles = stylex.create({
  form: { display: 'flex', flexDirection: 'column', gap: s.lg, width: '100%', minWidth: 0, boxSizing: 'border-box', color: text.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  card: { padding: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, backgroundColor: surface.raised },
  pinned: { paddingBlock: s.md, paddingInline: s.zero, backgroundColor: surface.transparent },
  header: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm, minWidth: 0 },
  glyph: { display: 'inline-flex', color: text.fgMuted },
  title: { fontWeight: t.weightMedium, color: text.fg },
  meta: { color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  headerEnd: { display: 'flex', alignItems: 'center', gap: s.sm, marginInlineStart: 'auto' },
  chip: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.full, color: text.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, marginInlineEnd: s.sm },
  tabs: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  tabList: { display: 'flex', flexWrap: 'wrap', gap: s.xs },
  tabListStacked: { flexDirection: 'column', flexWrap: 'nowrap' },
  tab: { display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, height: g.controlMd, paddingInline: s.md, boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.full, color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', outlineStyle: 'none', ':hover': { color: text.fg } },
  tabStacked: { width: '100%', justifyContent: 'space-between', borderRadius: r.control },
  tabSelected: { color: text.fg, backgroundColor: surface.rowActive },
  tabLabel: { maxWidth: g.tabMax, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  tabLabelStacked: { maxWidth: 'none' },
  marker: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.iconSm, height: g.iconSm, flexShrink: 0 },
  markerAnswered: { color: text.fgSoft },
  tabPanel: { outlineStyle: 'none', minWidth: 0 },
  focusRing: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  question: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  promptRow: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', minWidth: 0 },
  prompt: { fontWeight: t.weightMedium, color: text.fg, overflowWrap: 'anywhere' },
  subject: { minWidth: 0, color: text.fgSoft },
  body: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  bodyBeside: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1fr)', alignItems: 'start', gap: s.lg },
  column: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  options: { display: 'flex', flexDirection: 'column', gap: s.xs2 },
  option: { display: 'block', borderRadius: r.control, cursor: 'pointer', outlineStyle: 'none', ':hover': { backgroundColor: surface.washSubtle } },
  /** rowHover, not rowActive: muted descriptions keep AA contrast on the selected row in both schemes. */
  optionSelected: { backgroundColor: surface.rowHover, ':hover': { backgroundColor: surface.rowHover } },
  optionInert: { cursor: 'default' },
  optionRow: { display: 'flex', alignItems: 'flex-start', gap: s.md, paddingBlock: s.sm, paddingInline: s.md, minWidth: 0 },
  indicator: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, marginBlockStart: s.xs2, flexShrink: 0, boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: text.fgMuted, color: text.fg },
  ring: { borderRadius: r.full },
  box: { borderRadius: r.keyboard },
  indicatorSelected: { borderColor: text.fg },
  dot: { width: g.statusDot, height: g.statusDot, borderRadius: r.full, backgroundColor: text.fg },
  optionText: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0 },
  optionLabel: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm, color: text.fg, overflowWrap: 'anywhere' },
  description: { color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  badge: { display: 'inline-flex', alignItems: 'center', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.full, color: text.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading },
  danger: { display: 'inline-flex', color: text.fgSoft },
  keycap: { flexShrink: 0, minWidth: g.iconSm, paddingInline: s.xs, textAlign: 'center', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.keyboard, color: text.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: t.metaLeading },
  otherField: { paddingInlineStart: s.panel },
  input: { width: '100%', height: g.controlLg, boxSizing: 'border-box', paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fg, fontFamily: t.fontSans, fontSize: t.uiSize, outlineStyle: 'none', '::placeholder': { color: text.fgMuted }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  note: { display: 'flex', flexDirection: 'column', gap: s.xs },
  preview: { margin: s.zero, display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, padding: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, backgroundColor: surface.washSubtle },
  previewEmpty: { margin: s.zero, color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  problem: { display: 'flex', alignItems: 'center', gap: s.xs, margin: s.zero, color: text.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading },
  footer: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.md, minWidth: 0 },
  hints: { display: 'flex', flexWrap: 'wrap', gap: s.md, color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  kbd: { paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.keyboard, color: text.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize },
  actions: { display: 'flex', alignItems: 'center', gap: s.sm, marginInlineStart: 'auto' },
  settledLine: { margin: s.zero, color: text.fgSoft },
  summary: { display: 'flex', flexDirection: 'column', gap: s.md, margin: s.zero },
  summaryRow: { display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0 },
  summaryPrompt: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  summaryAnswer: { margin: s.zero, color: text.fg, overflowWrap: 'anywhere' },
  summaryNote: { margin: s.zero, color: text.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
})
