import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, ListBox, ListBoxItem, TextField, TextArea, Input, Label, FileTrigger } from 'react-aria-components'
import { accentTokens, tokens } from './embrace-tokens.stylex'
import { EmbraceMarkdownPreview } from './EmbraceToolPreview'
import type { Answers, AnswerDraft, Preview, QuestionRequestView, QuestionVariant, QuestionView } from './question-model'
export type * from './question-model'

const OTHER = '__other'
const pending = (request: QuestionRequestView) => ['pending', 'failed', 'submitting'].includes(request.state._tag)
const answered = (question: QuestionView, draft?: AnswerDraft) => draft !== undefined && (question.kind === 'multi' || draft.selected.some(id => id !== OTHER) || (draft.other ?? '').trim() !== '')
const title = (request: QuestionRequestView) => request.title ?? (request.questions[0]?.kind === 'permission' ? 'Permission needed' : request.questions[0]?.kind === 'plan' ? 'Plan ready for review' : 'Agent asks')
const stateLabel = (request: QuestionRequestView) => !request.answerable && request.state._tag === 'pending' ? 'Answer in terminal' : ({ pending: 'Needs you', submitting: 'Sending…', failed: 'Not delivered', answered: 'Answered', 'answered-elsewhere': 'Answered elsewhere', expired: 'Timed out', cancelled: 'Cancelled', 'chat-redirected': 'Chat redirected' } as const)[request.state._tag]

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
  readonly onAttach?: (questionId: string, files: readonly File[]) => void
}

/** Draft ownership stays with the host; all three surfaces share the same question id. */
export const QuestionForm = (props: QuestionFormProps) => {
  const { request, draft = {} } = props
  const [step, setStep] = React.useState(0)
  const [noteOpen, setNoteOpen] = React.useState(false)
  const [focused, setFocused] = React.useState<string>()
  const root = React.useRef<HTMLDivElement>(null)
  const question = request.questions[Math.min(step, request.questions.length - 1)]
  const focusedPreview = question?.options.find(option => option.id === focused)?.preview ?? question?.options.find(option => draft[question.id]?.selected.includes(option.id))?.preview ?? (question?.options.find(option => option.recommended) ?? question?.options[0])?.preview
  const review = step >= request.questions.length
  const disabled = !request.answerable || request.state._tag === 'submitting'
  const complete = request.questions.every(q => q.required === false || answered(q, draft[q.id]))
  const update = (q: QuestionView, patch: Partial<AnswerDraft>) => props.onDraftChange?.({ ...draft, [q.id]: { selected: [], ...draft[q.id], ...patch } })
  const submit = (next = draft) => { if (!disabled && request.questions.every(q => q.required === false || answered(q, next[q.id]))) props.onSubmit?.(next) }
  const advance = () => { if (step < request.questions.length - 1 || (request.questions.length > 1 && !review)) setStep(step + 1); else submit() }
  const choose = (q: QuestionView, id: string, advanceAfter = false) => {
    const current = draft[q.id]?.selected ?? []
    const selected = q.kind === 'multi' ? current.includes(id) ? current.filter(value => value !== id) : [...current, id] : [id]
    const next = { ...draft, [q.id]: { ...draft[q.id], selected, ...(id === OTHER ? {} : { other: undefined }) } }
    props.onDraftChange?.(next)
    if (advanceAfter && q.kind !== 'multi' && id !== OTHER) { if (request.questions.length > 1) setStep(step + 1); else submit(next) }
  }
  const keys = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); root.current?.querySelector<HTMLElement>('[role="option"]')?.focus(); return }
    if (disabled || question === undefined) return
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); submit(); return }
    if ((event.target as HTMLElement).matches('textarea, input')) { if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); advance() }; return }
    if ((event.target as HTMLElement).closest('[role="listbox"]') === null) return
    const ids = [...question.options.map(option => option.id), ...(question.allowOther ? [OTHER] : [])]
    if (/^[1-9]$/.test(event.key) && !event.metaKey && !event.ctrlKey) { const id = ids[Number(event.key) - 1]; if (id !== undefined) { event.preventDefault(); event.stopPropagation(); choose(question, id) } }
    else if (event.key === 'Enter') { event.preventDefault(); event.stopPropagation(); if (question.kind === 'multi') advance(); else if (focused !== undefined) choose(question, focused, true) }
    else if (event.key === 'ArrowRight' || event.key === 'ArrowLeft' || event.key === 'Tab') { const direction = event.key === 'ArrowLeft' || (event.key === 'Tab' && event.shiftKey) ? -1 : 1; const next = step + direction; if (next >= 0 && next <= (request.questions.length > 1 ? request.questions.length : 0)) { event.preventDefault(); setStep(next) } }
    else if (event.key === 'n' && question.allowNote) { event.preventDefault(); setNoteOpen(true) }
  }
  if (!pending(request)) return <QuestionSummary request={request} />
  return <section {...stylex.props(styles.card)} data-question-variant={props.variant ?? 'transcript'} aria-label={`${title(request)} from ${request.agentLabel}`}>
    <header {...stylex.props(styles.header)}><strong>{title(request)}</strong><span {...stylex.props(styles.chip)}>{stateLabel(request)}</span></header>
    <div {...stylex.props(styles.meta)}>{request.agentLabel} · {request.source} · {request.askedAt}</div>
    {props.onJump !== undefined ? <Button {...stylex.props(styles.button)} onPress={props.onJump}>Show in thread</Button> : null}
    <div ref={root} onKeyDownCapture={keys} {...stylex.props(styles.body)}>
      {request.questions.length > 1 ? <nav aria-label="Questions" {...stylex.props(styles.header)}>{request.questions.map((q, index) => <Button key={q.id} {...stylex.props(styles.button)} aria-current={step === index ? 'step' : undefined} onPress={() => setStep(index)}>{q.header ?? `Q${index + 1}`}</Button>)}<Button {...stylex.props(styles.button)} onPress={() => setStep(request.questions.length)} aria-current={review ? 'step' : undefined}>Review</Button></nav> : null}
      {review ? <AnswerList request={request} answers={draft} /> : question !== undefined ? <>
        <EmbraceMarkdownPreview markdown={question.prompt} />
        {question.subject !== undefined ? <QuestionPreview preview={question.subject} /> : null}
        {question.kind === 'text' ? <TextField value={draft[question.id]?.other ?? ''} onChange={other => update(question, { other })} isDisabled={disabled}>
          <Label>Your answer</Label>
          {question.secret ? <Input type="password" {...stylex.props(styles.field)} placeholder={question.placeholder} /> : <TextArea {...stylex.props(styles.field)} placeholder={question.placeholder} />}
        </TextField> : <ListBox aria-label={question.prompt} selectionMode={question.kind === 'multi' ? 'multiple' : 'single'} selectedKeys={draft[question.id]?.selected ?? []} isDisabled={request.state._tag === 'submitting'} onSelectionChange={selection => { if (!disabled && selection !== 'all') update(question, { selected: [...selection].map(String) }) }} {...stylex.props(styles.body)}>
          {question.options.map((option, index) => <ListBoxItem key={option.id} id={option.id} textValue={option.label} onFocus={() => setFocused(option.id)} {...stylex.props(styles.option, option.tone === 'danger' && styles.danger)}>{({ isSelected, isFocusVisible }) => <div {...stylex.props(isSelected && styles.selected, isFocusVisible && styles.focus)}><span>{question.kind === 'multi' ? isSelected ? '☑' : '☐' : isSelected ? '●' : '○'} {index + 1}. {option.label}</span>{option.recommended ? <span {...stylex.props(styles.chip)}>Recommended</span> : null}{option.scope !== undefined ? <span {...stylex.props(styles.chip)}>{option.scope}</span> : null}{option.description !== undefined ? <p {...stylex.props(styles.meta)}>{option.description}</p> : null}</div>}</ListBoxItem>)}
          {question.allowOther ? <ListBoxItem id={OTHER} textValue="Other" onFocus={() => setFocused(OTHER)} {...stylex.props(styles.option)}>Other…</ListBoxItem> : null}
        </ListBox>}
        {focusedPreview !== undefined ? <QuestionPreview preview={focusedPreview} /> : null}
        {question.allowOther && draft[question.id]?.selected.includes(OTHER) ? <TextField value={draft[question.id]?.other ?? ''} onChange={other => update(question, { other })} isDisabled={disabled}><Label>Your answer</Label><TextArea {...stylex.props(styles.field)} /></TextField> : null}
        {question.allowNote ? <><Button {...stylex.props(styles.button)} onPress={() => setNoteOpen(!noteOpen)}>Note</Button>{noteOpen || draft[question.id]?.note !== undefined ? <TextField value={draft[question.id]?.note ?? ''} onChange={note => update(question, { note })} isDisabled={disabled}><Label>Note to the agent</Label><TextArea {...stylex.props(styles.field)} /></TextField> : null}</> : null}
        {question.allowAttachments && props.onAttach !== undefined ? <FileTrigger acceptedFileTypes={['image/*']} allowsMultiple onSelect={files => { if (files !== null) props.onAttach?.(question.id, Array.from(files)) }}><Button {...stylex.props(styles.button)} isDisabled={disabled}>Attach image</Button></FileTrigger> : null}
        {draft[question.id]?.attachments?.map(attachment => <span key={attachment} {...stylex.props(styles.chip)}>{attachment}</span>)}
      </> : null}
    </div>
    {request.state._tag === 'failed' ? <p role="alert">Couldn't deliver the answer: {request.state.error}. Your answers are kept. <Button {...stylex.props(styles.button)} onPress={props.onRetry}>Retry</Button></p> : null}
    {request.state._tag === 'pending' && request.state.expiresAt !== undefined ? <p {...stylex.props(styles.meta)}>Answer deadline · {request.state.expiresAt}</p> : null}
    {props.composerTyping ? <p {...stylex.props(styles.meta)}>delivered after the question is answered</p> : null}
    {!request.answerable ? <footer>{request.readOnlyReason ?? 'Answer this question in the terminal.'}<Button {...stylex.props(styles.button)} onPress={props.onOpenTerminal}>Open terminal</Button></footer> : <footer {...stylex.props(styles.header)}><span {...stylex.props(styles.meta)}>1–9 pick · ↑↓ move · Space toggle · Enter next · ⌘/Ctrl Enter submit · Esc keeps your draft</span>{props.onDecline !== undefined ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={props.onDecline}>Skip · ends the turn</Button> : null}{request.allowChatRedirect ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={props.onChat}>Chat about this</Button> : null}{request.questions.length > 1 && !review ? <Button {...stylex.props(styles.button)} isDisabled={disabled} onPress={advance}>Next</Button> : null}<Button {...stylex.props(styles.button)} isDisabled={disabled || !complete} onPress={() => submit()}>{request.state._tag === 'submitting' ? 'Sending…' : 'Submit'}</Button></footer>}
  </section>
}

const AnswerList = ({ request, answers }: { readonly request: QuestionRequestView; readonly answers: Answers }) => (
  <dl>{request.questions.map((q, index) => {
    const answer = answers[q.id]
    const labels = answer?.selected.filter(id => id !== OTHER).map(id => q.options.find(option => option.id === id)?.label ?? id) ?? []
    if (answer?.other) labels.push(`“${answer.other}”`)
    return <div key={q.id}>
      <dt>{q.header ?? `Q${index + 1}`} · {q.prompt}</dt>
      <dd>{answer === undefined ? 'Left open' : labels.join(', ') || (q.kind === 'multi' ? 'none' : 'Left open')}
        {answer?.autoSelected ? <span {...stylex.props(styles.meta)}> · auto-selected (timeout)</span> : null}
        {answer?.note ? <p>Note: {answer.note}</p> : null}
        {answer?.attachments?.map(attachment => <span key={attachment} {...stylex.props(styles.chip)}>{attachment}</span>)}
      </dd>
    </div>
  })}</dl>
)

export const QuestionSummary = ({ request }: { readonly request: QuestionRequestView }) => {
  const state = request.state
  const answers = state._tag === 'answered' ? state.answers : state._tag === 'answered-elsewhere' ? state.answers : state._tag === 'expired' ? state.autoAnswer : undefined
  return <section {...stylex.props(styles.summary)} aria-label="Question outcome"><strong>{stateLabel(request)}</strong>{answers !== undefined ? <AnswerList request={request} answers={answers} /> : request.questions.map(q => <p key={q.id}>{q.prompt} · Left open</p>)}
    <p {...stylex.props(styles.meta)}>{state._tag === 'answered' ? `Answered by ${state.by}${state.where !== undefined ? ` in ${state.where}` : ''} · ${state.at}` : state._tag === 'answered-elsewhere' ? `Answered in ${state.where} · ${state.at}${state.draftDiscarded ? ' · your unsent web draft was discarded' : ''}` : state._tag === 'expired' ? `${state.at} · ${state.autoAnswer !== undefined ? 'auto-selected (timeout)' : 'continued without an answer'}` : state._tag === 'cancelled' ? `Skipped · turn ended · ${state.reason} · ${state.at}` : state._tag === 'chat-redirected' ? `${state.message} · ${state.at}` : ''}</p>
    <details><summary>Original questions</summary>{request.questions.map(q => <div key={q.id}><EmbraceMarkdownPreview markdown={q.prompt} />{q.subject !== undefined ? <QuestionPreview preview={q.subject} /> : null}{q.options.map(option => <div key={option.id}>{option.label}{option.preview !== undefined ? <QuestionPreview preview={option.preview} /> : null}</div>)}</div>)}</details>
  </section>
}

export const NeedsYouBar = ({ requests, expanded, onToggle, transcriptVisibility = 0, children }: { readonly requests: readonly QuestionRequestView[]; readonly expanded: boolean; readonly onToggle: () => void; readonly transcriptVisibility?: number; readonly children?: React.ReactNode }) => {
  const waiting = requests.filter(pending)
  const first = waiting[0]
  if (first === undefined || transcriptVisibility >= 0.6) return null
  return <aside {...stylex.props(styles.card)} aria-label="Needs you"><Button {...stylex.props(styles.button)} aria-expanded={expanded} onPress={onToggle}>◌ {first.agentLabel} is waiting on you · {first.questions[0]?.prompt}{waiting.length > 1 ? ` · +${waiting.length - 1}` : ''} · {expanded ? 'Hide' : 'Answer'}</Button>{expanded ? children : null}</aside>
}
export const NeedsYouInbox = ({ requests, onShowInThread, onOpenTerminal, formProps }: { readonly requests: readonly QuestionRequestView[]; readonly onShowInThread: (id: string) => void; readonly onOpenTerminal: (id: string) => void; readonly formProps?: (id: string) => Omit<QuestionFormProps, 'request' | 'variant' | 'onJump' | 'onOpenTerminal'> }) => <section aria-label="Needs you inbox" {...stylex.props(styles.body)}>{requests.filter(pending).map(request => <QuestionForm key={request.id} {...formProps?.(request.id)} request={request} variant="inbox" onJump={() => onShowInThread(request.id)} onOpenTerminal={() => onOpenTerminal(request.id)} />)}</section>

const styles = stylex.create({
  card: { padding: 14, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 8, backgroundColor: tokens.panel, color: tokens.ink, minWidth: 0, overflowWrap: 'anywhere' },
  summary: { paddingBlock: 8, color: tokens.ink, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: tokens.line },
  body: { display: 'flex', flexDirection: 'column', gap: 10, minWidth: 0 },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8 },
  meta: { color: tokens.muted, fontSize: 12 },
  chip: { color: tokens.muted, fontSize: 11, paddingInline: 5 },
  button: { paddingBlock: 6, paddingInline: 9, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 5, backgroundColor: tokens.recess, color: tokens.ink, cursor: 'pointer', outlineColor: accentTokens.accent, outlineOffset: 2 },
  danger: { color: tokens.danger },
  option: { padding: 8, borderRadius: 4, outlineColor: accentTokens.accent, outlineOffset: 2, cursor: 'pointer' },
  selected: { backgroundColor: tokens.recess }, focus: { outlineWidth: 2, outlineStyle: 'solid', outlineColor: accentTokens.accent },
  field: { width: '100%', boxSizing: 'border-box', padding: 8, backgroundColor: tokens.recess, color: tokens.ink, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 4 },
  preview: { margin: 0, padding: 10, backgroundColor: tokens.recess, borderRadius: 4 },
  pre: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontSize: 12 },
})
