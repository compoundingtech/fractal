import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Label, TextArea, TextField } from 'react-aria-components'
import { VisuallyHidden } from 'react-aria'
import { Button } from '../assistant-ui/composition/Controls'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { composerChrome as chrome, density } from '../assistant-ui/embrace-tokens.stylex'
import { AgentMarker } from './AgentMarker'

/** Server limit for note text (CR06), in UTF-8 bytes. */
export const noteTextLimitBytes = 1024
/** The visible byte counter appears from this share of the limit; below it the limit is assistive text only. */
const counterThreshold = 0.8
const encoder = new TextEncoder()

/** A note on a session subject ends with that incarnation: it never transfers to the agent's next session. */
export const sessionNoteReach = "Notes reach this session's agent until the session ends."
export const isSessionTarget = (target: string): boolean => target.startsWith('session/')
const reachCopy = (target: string | undefined): string =>
  target === undefined ? 'Notes reach the agent as context.'
    : target.startsWith('mission/') ? 'Notes reach agents currently working on this mission.'
      : isSessionTarget(target) ? sessionNoteReach
        : 'Notes reach the agent as context.'

/** One create's progress: none, in flight, or refused with the gateway's stated cause when it gave one. */
export type NoteCreateState =
  | { readonly _tag: 'Idle' }
  | { readonly _tag: 'Pending' }
  | { readonly _tag: 'Refused'; readonly detail: string | undefined }
export const noteCreateIdle: NoteCreateState = { _tag: 'Idle' }

export interface NoteComposerProps {
  /** Adds the note. A rejection keeps the text and shows an error with the rejection's message; resolution clears the draft. */
  readonly onSubmit: (text: string) => Promise<void>
  /** Why notes cannot be added right now. The draft stays editable, since it is local; nothing submits until the reason clears. */
  readonly disabledReason?: string | undefined
  readonly autoFocus?: boolean | undefined
  /** Controlled draft text; the host retains it per target. */
  readonly draft?: string | undefined
  readonly onDraftChange?: ((text: string) => void) | undefined
  /** The target ref. It selects the reach copy (agent, mission or session) and is never displayed raw. */
  readonly target?: string | undefined
  /** The target's display name (the agent title, or "this session"), used in visible text instead of the ref. */
  readonly targetLabel?: string | undefined
  /** Shows "Target: <label>". Hosts whose pane already names the target turn it off. */
  readonly showTarget?: boolean | undefined
  /** `card`: the standalone composer surface. `bare`: no border, fill or padding, inside a surface that supplies them. */
  readonly surface?: 'card' | 'bare' | undefined
  /** Reports a create in flight, so a host overlay can refuse dismissal until it settles. */
  readonly onPendingChange?: ((pending: boolean) => void) | undefined
  /**
   * Controlled create state. A host that unmounts the composer while a create is in flight (another agent, then
   * back) keeps it per target beside the draft, so the remounted composer shows the pending create or its refusal.
   */
  readonly createState?: NoteCreateState | undefined
  readonly onCreateStateChange?: ((state: NoteCreateState) => void) | undefined
  /** Closing never discards the draft; the host retains controlled drafts per target. */
  readonly onClose?: (() => void) | undefined
}

/** Plain-text note input. Every note reaches the target's agents, so there is no audience choice. */
export function NoteComposer({ onSubmit, disabledReason, autoFocus = false, draft, onDraftChange, target, targetLabel, showTarget = true, surface: chromeSurface = 'card', onPendingChange, createState, onCreateStateChange, onClose }: NoteComposerProps) {
  const [localDraft, setLocalDraft] = React.useState('')
  const text = draft ?? localDraft
  const updateDraft = (next: string) => { setLocalDraft(next); onDraftChange?.(next) }
  const [localCreate, setLocalCreate] = React.useState<NoteCreateState>(noteCreateIdle)
  const create = createState ?? localCreate
  // Every write goes through here, including one that lands after this composer unmounted: the host's state is the record.
  const setCreate = (next: NoteCreateState) => { setLocalCreate(next); onCreateStateChange?.(next); onPendingChange?.(next._tag === 'Pending') }
  const pending = create._tag === 'Pending'
  const failure = create._tag === 'Refused' ? create : undefined
  const [feedback, setFeedback] = React.useState('')
  // The text a Discard removed, until the next edit or submit; Undo puts it back.
  const [discarded, setDiscarded] = React.useState<string | undefined>(undefined)
  const inputRef = React.useRef<HTMLTextAreaElement>(null)
  const ids = { help: React.useId(), limit: React.useId(), reason: React.useId(), reach: React.useId(), target: React.useId() }
  const bytes = encoder.encode(text).length
  const overLimit = bytes > noteTextLimitBytes
  const showCounter = bytes >= noteTextLimitBytes * counterThreshold
  const canSubmit = disabledReason === undefined && !pending && text.trim().length > 0 && !overLimit
  const bare = chromeSurface === 'bare'
  const targetLine = showTarget && targetLabel !== undefined

  const submit = async () => {
    if (!canSubmit) return
    setCreate({ _tag: 'Pending' })
    setDiscarded(undefined)
    setFeedback('Adding note…')
    try {
      await onSubmit(text)
    } catch (error) {
      setFeedback('')
      setCreate({ _tag: 'Refused', detail: error instanceof Error && error.message.trim() !== '' ? error.message.trim() : undefined })
      inputRef.current?.focus()
      return
    }
    setCreate(noteCreateIdle)
    updateDraft('')
    setFeedback('Note added.')
  }
  const discard = () => {
    setDiscarded(text)
    updateDraft('')
    if (failure !== undefined) setCreate(noteCreateIdle)
    setFeedback('Draft discarded.')
    inputRef.current?.focus()
  }
  const undoDiscard = () => {
    if (discarded === undefined) return
    updateDraft(discarded)
    setDiscarded(undefined)
    setFeedback('Draft restored.')
    inputRef.current?.focus()
  }

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape' && !event.nativeEvent.isComposing && !pending) {
      event.preventDefault()
      onClose?.()
    }
  }
  const onFieldKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      void submit()
    }
  }

  const describedBy = [ids.help, ids.reach, ids.limit, targetLine ? ids.target : undefined, disabledReason === undefined ? undefined : ids.reason].filter(Boolean).join(' ')
  return <form
    data-testid="note-composer"
    data-surface={chromeSurface}
    aria-busy={pending || undefined}
    onKeyDown={onKeyDown}
    onSubmit={event => { event.preventDefault(); void submit() }}
    {...stylex.props(styles.root, bare ? styles.bare : styles.card, !bare && overLimit && styles.cardInvalid)}
  >
    <TextField
      value={text}
      onChange={next => { setDiscarded(undefined); updateDraft(next) }}
      isReadOnly={pending}
      isInvalid={overLimit}
      autoFocus={autoFocus}
      aria-describedby={describedBy}
      {...stylex.props(styles.field)}
    >
      <VisuallyHidden><Label>Note</Label></VisuallyHidden>
      <TextArea ref={inputRef} rows={1} placeholder="Add a note…" data-testid="note-composer-input" onKeyDown={onFieldKeyDown} {...stylex.props(styles.input, bare && styles.inputBare, bare && overLimit && styles.inputInvalid)} />
    </TextField>
    <div {...stylex.props(styles.footer)}>
      <span id={ids.help} data-testid="note-composer-help" {...stylex.props(styles.help)}>
        <kbd {...stylex.props(styles.kbd)}>Enter</kbd> adds · <kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd> new line
      </span>
      <span {...stylex.props(styles.grow)} />
      {/* Discard is quiet and only offered for a draft; Undo in the status line restores it. */}
      {text.length > 0 && !pending && <Button type="button" variant="ghost" size="md" data-testid="note-composer-discard" onPress={discard}>Discard draft</Button>}
      <Button type="submit" variant="outline" size="md" isDisabled={!canSubmit} data-testid="note-composer-submit">
        {pending ? 'Adding…' : failure !== undefined ? 'Retry' : 'Add note'}
      </Button>
    </div>
    {/* The live region stays mounted; it takes no layout space while it has nothing to say. */}
    <div {...stylex.props(styles.statusLine, feedback === '' && discarded === undefined && styles.visuallyHidden)}>
      <p role="status" aria-live="polite" aria-atomic="true" data-testid="note-composer-status" {...stylex.props(styles.meta)}>{feedback}</p>
      {discarded !== undefined && <Button type="button" variant="ghost" size="sm" data-testid="note-composer-undo" onPress={undoDiscard}>Undo</Button>}
    </div>
    {/* Every note is agent context: state who it reaches, never that it was delivered. */}
    <p id={ids.reach} data-testid="note-composer-reach" {...stylex.props(styles.meta, styles.reach)}>
      <AgentMarker />
      {reachCopy(target)} People with access can read them too.
    </p>
    {targetLine && <p id={ids.target} data-testid="note-composer-target" {...stylex.props(styles.meta)}>Target: {targetLabel}.</p>}
    <p id={ids.limit} aria-live="polite" data-testid="note-composer-limit" {...stylex.props(styles.meta, overLimit && styles.error, !showCounter && styles.visuallyHidden)}>
      {overLimit ? `Too long by ${bytes - noteTextLimitBytes} bytes; notes hold ${noteTextLimitBytes} bytes.`
        : showCounter ? `${bytes} of ${noteTextLimitBytes} bytes` : `Notes hold up to ${noteTextLimitBytes} bytes.`}
    </p>
    {disabledReason !== undefined && <p id={ids.reason} data-testid="note-composer-disabled-reason" {...stylex.props(styles.meta)}>{disabledReason}</p>}
    {failure !== undefined && <p role="alert" data-testid="note-composer-error" {...stylex.props(styles.meta, styles.error)}>
      {/* A stated cause replaces the generic connection advice rather than repeating it. */}
      Couldn't add the note. Your text is kept.
      {failure.detail === undefined ? ' Check your connection and Retry.' : <> <span data-testid="note-composer-error-detail">{failure.detail}</span></>}
    </p>}
  </form>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minWidth: 0, boxSizing: 'border-box', color: text.fg, fontFamily: t.fontSans },
  card: {
    gap: density.composerGap, padding: density.composerPad, backgroundColor: surface.glassFill,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: { default: border.glassBorder, ':focus-within': border.focusBorder }, borderRadius: chrome.composerRadius,
  },
  // Over the limit the field edge carries the invalid state; the text itself stays readable.
  cardInvalid: { borderColor: { default: status.dangerFg, ':focus-within': status.dangerFg } },
  bare: { gap: s.sm },
  field: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  input: {
    display: 'block', width: '100%', boxSizing: 'border-box', minHeight: g.compactEditorMin, maxHeight: g.editorMax, fieldSizing: 'content',
    overflowY: 'auto', resize: 'none', padding: density.inputPad, borderWidth: 0, borderRadius: chrome.inputRadius, backgroundColor: surface.transparent, color: text.fg,
    fontFamily: t.fontSans, fontSize: density.inputSize, lineHeight: t.uiLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere',
    '::placeholder': { color: text.fgMuted },
    ':focus-visible': { outlineStyle: 'none' },
  },
  // Inside a popover or disclosure the field draws its own edge, nested inside the host's radius.
  inputBare: {
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: { default: border.borderStrong, ':focus': border.focusBorder },
    borderRadius: r.sm, backgroundColor: surface.controlFill,
  },
  inputInvalid: { borderColor: { default: status.dangerFg, ':focus': status.dangerFg } },
  footer: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, minWidth: 0 },
  statusLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, minWidth: 0 },
  reach: { display: 'flex', alignItems: 'center', gap: s.xs },
  help: { minWidth: 0, fontSize: t.denseSize, lineHeight: t.metaLeading, color: text.fgMuted },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs2, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  grow: { flexGrow: 1 },
  meta: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted, overflowWrap: 'anywhere' },
  error: { color: status.dangerFg },
  visuallyHidden: { position: 'absolute', width: 1, height: 1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
})
