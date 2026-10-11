import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, RadioGroup, Radio, CheckboxGroup, Checkbox, TextField, TextArea, Input, Label, FileTrigger } from 'react-aria-components'
import { surfaceVars, textVars, borderVars, accentVars, statusVars } from './composition-tokens.stylex'
import { EmbraceMarkdownPreview } from './EmbraceToolPreview'
import type { Answers, AnswerDraft, Preview, QuestionRequestView, QuestionVariant, QuestionView } from './question-model'
export type * from './question-model'

const OTHER = '__other'
const pending = (request: QuestionRequestView) => ['pending', 'failed', 'submitting'].includes(request.state._tag)
const answered = (question: QuestionView, draft?: AnswerDraft) => question.kind === 'multi' || (draft !== undefined && (draft.selected.some(id => id !== OTHER) || (draft.other ?? '').trim() !== ''))
const title = (request: QuestionRequestView) => request.title ?? (request.questions[0]?.kind === 'permission' ? 'Permission needed' : request.questions[0]?.kind === 'plan' ? 'Plan ready for review' : 'Agent asks')
const stateLabel = (request: QuestionRequestView) => !request.answerable && request.state._tag === 'pending' ? 'Answer in terminal' : ({ pending: 'Needs you', submitting: 'Sending…', failed: 'Not delivered', answered: 'Answered', 'answered-elsewhere': 'Answered elsewhere', expired: 'Timed out', cancelled: 'Cancelled', 'chat-redirected': 'Chat redirected', unrecorded: '' } as const)[request.state._tag]

export const QuestionPreview = ({ preview }: { readonly preview: Preview }) => preview._tag === 'markdown' ? <EmbraceMarkdownPreview markdown={preview.text} /> : <figure {...stylex.props(styles.preview)}>{preview._tag === 'command' ? <><figcaption>{preview.cwd}</figcaption><pre {...stylex.props(styles.pre)}>$ {preview.command}</pre></> : preview._tag === 'diff' ? <><figcaption>{preview.path}</figcaption><pre {...stylex.props(styles.pre)}>{preview.lines.join('\n')}</pre></> : <pre {...stylex.props(styles.pre)}>{preview.url}</pre>}</figure>

export interface QuestionFormProps {
  readonly request: QuestionRequestView
  readonly variant?: QuestionVariant
  readonly draft?: Answers
  readonly onDraftChange?: (answers: Answers) => void
  readonly onSubmit?: (answers: Answers) => void
  readonly onDecline?: () => void
  readonly onChat?: () => void
  readonly onRetry?: () => void
  readonly onOpenTerminal?: () => void
  readonly onJump?: () => void
  readonly composerTyping?: boolean
  readonly composerRouting?: { readonly _tag: 'HeldUntilAnswered' } | { readonly _tag: 'ChatAboutQuestion' }
  readonly ageLabel?: string
  readonly onAttach?: (questionId: string, files: readonly File[]) => void
  readonly autoFocus?: boolean
}

/** Draft ownership stays with the host; all three surfaces share the same question id. */
export const QuestionForm = (props: QuestionFormProps) => {
  const { request, draft = {} } = props
  const [step, setStep] = React.useState(0)
  const [notesOpen, setNotesOpen] = React.useState<Readonly<Record<string, boolean>>>({})
  const [focused, setFocused] = React.useState<string>()
  const root = React.useRef<HTMLDivElement>(null)
  const submitRef = React.useRef<HTMLButtonElement>(null)
  const otherRef = React.useRef<HTMLTextAreaElement>(null)
  const noteRef = React.useRef<HTMLTextAreaElement>(null)
  const question = request.questions[Math.min(step, request.questions.length - 1)]
  const review = request.questions.length > 1 && step === request.questions.length
  const disabled = !request.answerable || request.state._tag === 'submitting'
  const complete = request.questions.every(q => q.required === false || answered(q, draft[q.id]))
  const focusedPreview = question?.options.find(option => option.id === focused)?.preview ?? question?.options.find(option => draft[question.id]?.selected.includes(option.id))?.preview ?? (question?.options.find(option => option.recommended) ?? question?.options[0])?.preview
  const update = (q: QuestionView, patch: Partial<AnswerDraft>) => props.onDraftChange?.({ ...draft, [q.id]: { selected: [], ...draft[q.id], ...patch } })
  const submit = (next = draft) => {
    if (!disabled && request.questions.every(q => q.required === false || answered(q, next[q.id]))) {
      const answers = Object.fromEntries(request.questions.flatMap(q => next[q.id] !== undefined ? [[q.id, next[q.id]]] : q.kind === 'multi' ? [[q.id, { selected: [] }]] : []))
      props.onSubmit?.(answers)
    }
  }
  const focusChoice = (q: QuestionView, id?: string) => {
    const preferred = id ?? draft[q.id]?.selected[0] ?? q.options.find(option => option.recommended)?.id ?? q.options[0]?.id ?? OTHER
    const label = Array.from(root.current?.querySelectorAll<HTMLElement>('[data-choice-id]') ?? []).find(element => element.dataset.choiceId === preferred)
    label?.querySelector<HTMLInputElement>('input')?.focus()
  }
  const go = (index: number) => {
    const next = Math.max(0, Math.min(index, request.questions.length > 1 ? request.questions.length : 0))
    setStep(next)
    setFocused(undefined)
    requestAnimationFrame(() => {
      if (next === request.questions.length) { submitRef.current?.focus(); return }
      const destination = request.questions[next]
      if (destination === undefined) return
      if (destination.kind === 'text') root.current?.querySelector<HTMLInputElement | HTMLTextAreaElement>('input[type="password"], textarea')?.focus()
      else focusChoice(destination)
    })
  }
  const advance = () => { if (step < request.questions.length - 1 || (request.questions.length > 1 && !review)) go(step + 1); else submit() }
  const choose = (q: QuestionView, id: string, advanceAfter = false) => {
    const current = draft[q.id]?.selected ?? []
    const selected = q.kind === 'multi' ? current.includes(id) ? current.filter(value => value !== id) : [...current, id] : [id]
    const next = { ...draft, [q.id]: { ...draft[q.id], selected, ...(q.kind !== 'multi' && id !== OTHER ? { other: undefined } : id === OTHER && !selected.includes(OTHER) ? { other: undefined } : {}) } }
    props.onDraftChange?.(next)
    if (id === OTHER && selected.includes(OTHER)) requestAnimationFrame(() => otherRef.current?.focus())
    else if (advanceAfter && q.kind !== 'multi') { if (request.questions.length > 1) go(step + 1); else submit(next) }
  }
  const keys = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (question !== undefined) focusChoice(question); return }
    if (question === undefined) return
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); submit(); return }
    if ((event.target as HTMLElement).matches('textarea, input[type="password"]')) { if (!disabled && event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); advance() }; return }
    const choiceId = (event.target as HTMLElement).closest<HTMLElement>('[data-choice-id]')?.dataset.choiceId
    if (choiceId === undefined) return
    const ids = [...question.options.map(option => option.id), ...(question.allowOther ? [OTHER] : [])]
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); const index = ids.indexOf(choiceId); focusChoice(question, ids[(index + ids.length + (event.key === 'ArrowDown' ? 1 : -1)) % ids.length]); return }
    if (event.key === 'ArrowRight' || event.key === 'ArrowLeft' || event.key === 'Tab') {
      const next = step + (event.key === 'ArrowLeft' || (event.key === 'Tab' && event.shiftKey) ? -1 : 1)
      if (next >= 0 && next <= (request.questions.length > 1 ? request.questions.length : 0)) { event.preventDefault(); event.stopPropagation(); go(next) }
      return
    }
    if (disabled) return
    if (/^[1-9]$/.test(event.key) && !event.metaKey && !event.ctrlKey) { const id = ids[Number(event.key) - 1]; if (id !== undefined) { event.preventDefault(); event.stopPropagation(); choose(question, id) } }
    else if (event.key === 'Enter') { event.preventDefault(); event.stopPropagation(); if (question.kind === 'multi') advance(); else choose(question, choiceId, true) }
    else if (event.key === 'n' && question.allowNote) { event.preventDefault(); setNotesOpen({ ...notesOpen, [question.id]: true }); requestAnimationFrame(() => noteRef.current?.focus()) }
  }
  if (!pending(request)) return <QuestionSummary request={request} />
  const initialChoice = question?.options.find(option => draft[question.id]?.selected.includes(option.id))?.id ?? question?.options.find(option => option.recommended)?.id ?? question?.options[0]?.id
  const choices = question === undefined ? [] : [...question.options, ...(question.allowOther ? [{ id: OTHER, label: 'Other…' }] : [])]
  return <section {...stylex.props(styles.card, props.variant === 'pinned' && styles.pinned)} data-question-variant={props.variant ?? 'transcript'} aria-label={`${title(request)} from ${request.agentLabel}`}>
    <header {...stylex.props(styles.header)}><span aria-hidden="true">{request.questions[0]?.kind === 'permission' ? '◇' : request.questions[0]?.kind === 'plan' ? '≡' : '?'}</span><strong>{request.agentLabel}</strong><span {...stylex.props(styles.chip)}>{stateLabel(request)}</span></header>
    <div {...stylex.props(styles.meta)}>{[request.source, props.ageLabel].filter(Boolean).join(' · ')}</div>
    {props.onJump !== undefined ? <Button {...stylex.props(styles.button)} onPress={props.onJump}>Show in thread</Button> : null}
    <div ref={root} onKeyDownCapture={keys} {...stylex.props(styles.body)}>
      {request.questions.length > 1 ? <nav aria-label="Questions" {...stylex.props(styles.header)}>{request.questions.map((q, index) => <Button key={q.id} {...stylex.props(styles.button)} aria-current={step === index ? 'step' : undefined} onPress={() => go(index)}>{q.header ?? `Q${index + 1}`}</Button>)}<Button {...stylex.props(styles.button)} onPress={() => go(request.questions.length)} aria-current={review ? 'step' : undefined}>Review</Button></nav> : null}
      {review ? <AnswerList request={request} answers={draft} /> : question !== undefined ? <>
        <EmbraceMarkdownPreview markdown={question.prompt} />
        {question.subject !== undefined ? <QuestionPreview preview={question.subject} /> : null}
        {question.kind === 'text' ? <TextField value={draft[question.id]?.other ?? ''} onChange={other => update(question, { other })} isDisabled={disabled} isRequired={question.required !== false}>
          <Label>Your answer</Label>
          {question.secret ? <Input type="password" autoFocus={props.autoFocus} {...stylex.props(styles.field)} placeholder={question.placeholder} /> : <TextArea ref={otherRef} autoFocus={props.autoFocus} {...stylex.props(styles.field)} placeholder={question.placeholder} />}
        </TextField> : question.kind === 'multi' ? <CheckboxGroup aria-label={question.prompt} value={[...(draft[question.id]?.selected ?? [])]} onChange={selected => {
          if (disabled) return
          const addedOther = selected.includes(OTHER) && !draft[question.id]?.selected.includes(OTHER)
          update(question, { selected, ...(!selected.includes(OTHER) && draft[question.id]?.selected.includes(OTHER) ? { other: undefined } : {}) })
          if (addedOther) requestAnimationFrame(() => otherRef.current?.focus())
        }} isDisabled={request.state._tag === 'submitting'} isReadOnly={!request.answerable} isRequired={question.required === true} {...stylex.props(styles.body)}>
          {choices.map((option, index) => <Checkbox key={option.id} value={option.id} data-choice-id={option.id} autoFocus={props.autoFocus && option.id === initialChoice} onFocus={() => setFocused(option.id)} {...stylex.props(styles.option, 'tone' in option && option.tone === 'danger' && styles.danger)}>{({ isSelected, isFocusVisible }) => <div {...stylex.props(isSelected && styles.selected, isFocusVisible && styles.focus)}><span aria-hidden="true">{isSelected ? '☑' : '☐'} {index + 1}. </span><ChoiceLabel option={option} /></div>}</Checkbox>)}
        </CheckboxGroup> : <RadioGroup aria-label={question.prompt} value={draft[question.id]?.selected[0] ?? ''} onChange={id => { if (!disabled) choose(question, id) }} isDisabled={request.state._tag === 'submitting'} isReadOnly={!request.answerable} isRequired={question.required !== false} {...stylex.props(styles.body)}>
          {choices.map((option, index) => <Radio key={option.id} value={option.id} data-choice-id={option.id} autoFocus={props.autoFocus && option.id === initialChoice} onFocus={() => setFocused(option.id)} {...stylex.props(styles.option, 'tone' in option && option.tone === 'danger' && styles.danger)}>{({ isSelected, isFocusVisible }) => <div {...stylex.props(isSelected && styles.selected, isFocusVisible && styles.focus)}><span aria-hidden="true">{isSelected ? '●' : '○'} {index + 1}. </span><ChoiceLabel option={option} /></div>}</Radio>)}
        </RadioGroup>}
        {focusedPreview !== undefined ? <QuestionPreview preview={focusedPreview} /> : null}
        {question.allowOther && draft[question.id]?.selected.includes(OTHER) ? <TextField value={draft[question.id]?.other ?? ''} onChange={other => update(question, { other })} isDisabled={disabled}><Label>Your answer</Label><TextArea ref={otherRef} {...stylex.props(styles.field)} /></TextField> : null}
        {question.allowNote ? <><Button {...stylex.props(styles.button, styles.note)} onPress={() => { setNotesOpen({ ...notesOpen, [question.id]: !notesOpen[question.id] }); requestAnimationFrame(() => noteRef.current?.focus()) }}>Note</Button>{notesOpen[question.id] || draft[question.id]?.note !== undefined ? <TextField value={draft[question.id]?.note ?? ''} onChange={note => update(question, { note })} isDisabled={disabled}><Label>Note to the agent</Label><TextArea ref={noteRef} {...stylex.props(styles.field)} /></TextField> : null}</> : null}
        {question.allowAttachments && props.onAttach !== undefined ? <FileTrigger acceptedFileTypes={['image/*']} allowsMultiple onSelect={files => { if (files !== null) props.onAttach?.(question.id, Array.from(files)) }}><Button {...stylex.props(styles.button)} isDisabled={disabled}>Attach image</Button></FileTrigger> : null}
        {draft[question.id]?.attachments?.map(attachment => <span key={attachment} {...stylex.props(styles.chip)}>{attachment}</span>)}
      </> : null}
    </div>
    {request.state._tag === 'failed' ? <p role="alert">Couldn't deliver the answer: {request.state.error}. Your answers are kept. <Button {...stylex.props(styles.button)} onPress={props.onRetry}>Retry</Button></p> : null}
    {request.state._tag === 'pending' && request.state.expiresAt !== undefined ? <p {...stylex.props(styles.meta)}>Answer deadline · {request.state.expiresAt}</p> : null}
    {props.composerTyping ? <p {...stylex.props(styles.meta)}>{props.composerRouting?._tag === 'ChatAboutQuestion' ? 'Your message goes to the agent as a note about this question' : 'delivered after the question is answered'}</p> : null}
    {!request.answerable ? <footer {...stylex.props(styles.header)}>{request.readOnlyReason ?? 'Answer this question in the terminal.'}<Button {...stylex.props(styles.button)} isDisabled={props.onOpenTerminal === undefined} onPress={props.onOpenTerminal}>Open terminal</Button></footer> : <footer {...stylex.props(styles.header)}>
      <span {...stylex.props(styles.meta)}>1–9 pick · ↑↓ move · Space toggle · Enter next · ⌘/Ctrl Enter submit · Esc keeps your draft</span>
      {props.onDecline !== undefined ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={props.onDecline}>Skip · ends the turn</Button> : null}
      {request.allowChatRedirect ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={props.onChat}>Chat about this</Button> : null}
      {request.questions.length > 1 && !review ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={advance}>Next</Button> : null}
      <Button ref={submitRef} {...stylex.props(styles.button)} isDisabled={disabled || !complete || props.onSubmit === undefined} onPress={() => submit()}>{request.state._tag === 'submitting' ? 'Sending…' : 'Submit'}</Button>
    </footer>}
  </section>
}

const ChoiceLabel = ({ option }: { readonly option: QuestionView['options'][number] }) => <><span>{option.label}</span>{option.recommended ? <span {...stylex.props(styles.chip)}>Recommended</span> : null}{option.scope !== undefined ? <span {...stylex.props(styles.chip)}>{option.scope}</span> : null}{option.description !== undefined ? <p {...stylex.props(styles.meta)}>{option.description}</p> : null}</>

const AnswerList = ({ request, answers }: { readonly request: QuestionRequestView; readonly answers: Answers }) => <dl>{request.questions.map((q, index) => {
  const answer = answers[q.id]
  const labels = answer?.selected.filter(id => id !== OTHER).map(id => q.options.find(option => option.id === id)?.label ?? id) ?? []
  if (answer?.other) labels.push(`“${answer.other}”`)
  return <div key={q.id}><dt>{q.header ?? `Q${index + 1}`} · {q.prompt}</dt><dd>{answer === undefined ? 'Left open' : labels.join(', ') || (q.kind === 'multi' ? 'none' : 'Left open')}{answer?.autoSelected ? <span {...stylex.props(styles.meta)}> · auto-selected (timeout)</span> : null}{answer?.note ? <p>Note: {answer.note}</p> : null}{answer?.attachments?.map(attachment => <span key={attachment} {...stylex.props(styles.chip)}>{attachment}</span>)}</dd></div>
})}</dl>

const OriginalQuestions = ({ request }: { readonly request: QuestionRequestView }) => <>{request.questions.map(q => <div key={q.id}><EmbraceMarkdownPreview markdown={q.prompt} />{q.subject !== undefined ? <QuestionPreview preview={q.subject} /> : null}{q.options.map(option => <div key={option.id}><ChoiceLabel option={option} />{option.preview !== undefined ? <QuestionPreview preview={option.preview} /> : null}</div>)}</div>)}</>

export const QuestionSummary = ({ request }: { readonly request: QuestionRequestView }) => {
  const state = request.state
  if (state._tag === 'unrecorded') return <section {...stylex.props(styles.summary)} aria-label="Original question"><OriginalQuestions request={request} />{state.reason !== '' ? <p {...stylex.props(styles.meta)}>{state.reason}</p> : null}</section>
  const answers = state._tag === 'answered' ? state.answers : state._tag === 'answered-elsewhere' ? state.answers : state._tag === 'expired' ? state.autoAnswer : undefined
  const footer = state._tag === 'answered' ? [state.by !== '' ? `Answered by ${state.by}` : '', state.where !== undefined ? `in ${state.where}` : '', state.at].filter(Boolean).join(' · ') : state._tag === 'answered-elsewhere' ? `Answered in ${state.where} · ${state.at}${state.draftDiscarded ? ' · your unsent web draft was discarded' : ''}` : state._tag === 'expired' ? `${state.at} · ${state.autoAnswer !== undefined ? 'auto-selected (timeout)' : 'continued without an answer'}` : state._tag === 'cancelled' ? [state.reason, state.at].filter(Boolean).join(' · ') : state._tag === 'chat-redirected' ? `${state.message} · ${state.at}` : ''
  return <section {...stylex.props(styles.summary)} aria-label="Question outcome"><strong>{stateLabel(request)}</strong>{answers !== undefined ? <AnswerList request={request} answers={answers} /> : request.questions.map(q => <p key={q.id}>{q.prompt}</p>)}
    {footer !== '' ? <p {...stylex.props(styles.meta)}>{footer}</p> : null}
    <details><summary>Original questions</summary><OriginalQuestions request={request} /></details>
  </section>
}

export const NeedsYouBar = ({ requests, expanded, onToggle, transcriptVisibility = 0, children }: { readonly requests: readonly QuestionRequestView[]; readonly expanded: boolean; readonly onToggle: () => void; readonly transcriptVisibility?: number; readonly children?: React.ReactNode }) => {
  const waiting = requests.filter(pending)
  const first = waiting[0]
  if (first === undefined || transcriptVisibility >= 0.6) return null
  return <aside {...stylex.props(styles.card)} aria-label="Needs you"><Button {...stylex.props(styles.button)} aria-expanded={expanded} onPress={onToggle}>◌ {first.agentLabel} is waiting on you · {first.questions[0]?.prompt}{waiting.length > 1 ? ` · +${waiting.length - 1}` : ''} · {expanded ? 'Hide' : 'Answer'}</Button>{expanded ? children : null}</aside>
}
export const NeedsYouInbox = ({ requests, onShowInThread, onOpenTerminal, formProps }: { readonly requests: readonly QuestionRequestView[]; readonly onShowInThread: (id: string) => void; readonly onOpenTerminal: (id: string) => void; readonly formProps?: (id: string) => Omit<QuestionFormProps, 'request' | 'variant' | 'onJump' | 'onOpenTerminal'> }) => <section aria-label="Needs you inbox" {...stylex.props(styles.body)}>{requests.filter(pending).map(request => <details key={request.id} {...stylex.props(styles.card)}><summary {...stylex.props(styles.inboxSummary)}>{request.agentLabel} · {request.questions[0]?.prompt}</summary><QuestionForm {...formProps?.(request.id)} request={request} variant="pinned" onJump={() => onShowInThread(request.id)} onOpenTerminal={() => onOpenTerminal(request.id)} /></details>)}</section>

const styles = stylex.create({
  card: { padding: 12, maxWidth: 720, width: '100%', boxSizing: 'border-box', borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: 8, backgroundColor: surfaceVars.raised, color: textVars.fg, minWidth: 0, overflowWrap: 'anywhere' },
  pinned: { padding: 0, borderWidth: 0, backgroundColor: 'transparent' },
  note: { alignSelf: 'flex-start', borderWidth: 0, backgroundColor: 'transparent', paddingInline: 0 },
  inboxSummary: { cursor: 'pointer', overflow: 'hidden', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' },
  summary: { paddingBlock: 8, color: textVars.fg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: borderVars.borderStrong },
  body: { display: 'flex', flexDirection: 'column', gap: 10, minWidth: 0 },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8 },
  meta: { color: textVars.fgMuted, fontSize: 12 },
  chip: { color: textVars.fgSoft, fontSize: 11, paddingInline: 5, paddingBlock: 2, backgroundColor: surfaceVars.rowActive, borderRadius: 4 },
  button: { paddingBlock: 6, paddingInline: 9, borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: 5, backgroundColor: surfaceVars.rowActive, color: textVars.fg, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: 2 },
  danger: { color: statusVars.dangerFg },
  option: { padding: 5, borderRadius: 4, outlineColor: accentVars.primary, outlineOffset: 2, cursor: 'pointer' },
  selected: { backgroundColor: surfaceVars.rowActive }, focus: { outlineWidth: 2, outlineStyle: 'solid', outlineColor: accentVars.primary },
  field: { width: '100%', boxSizing: 'border-box', padding: 8, backgroundColor: surfaceVars.rowActive, color: textVars.fg, borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: 4 },
  preview: { margin: 0, padding: 10, backgroundColor: surfaceVars.rowActive, borderRadius: 4 },
  pre: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontSize: 12 },
})
