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
const encoder = new TextEncoder()

export interface NoteComposerProps {
  /** Adds the note. A rejection keeps the text and shows an error with the rejection's message; resolution clears the draft. */
  readonly onSubmit: (text: string) => Promise<void>
  /** Why notes cannot be added right now; the draft stays readable and is not submitted. */
  readonly disabledReason?: string | undefined
  readonly autoFocus?: boolean | undefined
  /** Controlled draft text; the host retains it per target. */
  readonly draft?: string | undefined
  readonly onDraftChange?: ((text: string) => void) | undefined
  readonly target?: string | undefined
  /** Closing never discards the draft; the host retains controlled drafts per target. */
  readonly onClose?: (() => void) | undefined
}

/** Plain-text note input. Every note reaches the target's agents, so there is no audience choice. */
export function NoteComposer({ onSubmit, disabledReason, autoFocus = false, draft, onDraftChange, target, onClose }: NoteComposerProps) {
  const [localDraft, setLocalDraft] = React.useState('')
  const text = draft ?? localDraft
  const updateDraft = (next: string) => { setLocalDraft(next); onDraftChange?.(next) }
  const [pending, setPending] = React.useState(false)
  const [failure, setFailure] = React.useState<{ readonly detail: string | undefined } | undefined>(undefined)
  const [feedback, setFeedback] = React.useState('')
  const inputRef = React.useRef<HTMLTextAreaElement>(null)
  const ids = { help: React.useId(), limit: React.useId(), reason: React.useId(), reach: React.useId(), target: React.useId() }
  const bytes = encoder.encode(text).length
  const overLimit = bytes > noteTextLimitBytes
  const canSubmit = disabledReason === undefined && !pending && text.trim().length > 0 && !overLimit

  const submit = async () => {
    if (!canSubmit) return
    setPending(true)
    setFailure(undefined)
    setFeedback('Adding note…')
    try {
      await onSubmit(text)
    } catch (error) {
      setPending(false)
      setFeedback('')
      setFailure({ detail: error instanceof Error && error.message.trim() !== '' ? error.message.trim() : undefined })
      inputRef.current?.focus()
      return
    }
    setPending(false)
    updateDraft('')
    setFeedback('Note added.')
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

  const describedBy = [ids.help, ids.reach, ids.limit, target === undefined ? undefined : ids.target, disabledReason === undefined ? undefined : ids.reason].filter(Boolean).join(' ')
  return <form
    data-testid="note-composer"
    aria-busy={pending || undefined}
    onKeyDown={onKeyDown}
    onSubmit={event => { event.preventDefault(); void submit() }}
    {...stylex.props(styles.root)}
  >
    <TextField
      value={text}
      onChange={updateDraft}
      isReadOnly={pending || disabledReason !== undefined}
      isInvalid={overLimit}
      autoFocus={autoFocus}
      aria-describedby={describedBy}
      {...stylex.props(styles.field)}
    >
      <VisuallyHidden><Label>Note</Label></VisuallyHidden>
      <TextArea ref={inputRef} rows={1} placeholder="Add a note…" data-testid="note-composer-input" onKeyDown={onFieldKeyDown} {...stylex.props(styles.input)} />
    </TextField>
    <div {...stylex.props(styles.footer)}>
      <span id={ids.help} data-testid="note-composer-help" {...stylex.props(styles.help)}>
        <kbd {...stylex.props(styles.kbd)}>Enter</kbd> adds · <kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd> new line
      </span>
      <span {...stylex.props(styles.grow)} />
      <Button type="submit" variant="outline" size="md" isDisabled={!canSubmit} data-testid="note-composer-submit">
        {pending ? 'Adding…' : failure !== undefined ? 'Retry' : 'Add note'}
      </Button>
      <Button type="button" variant="outline" size="md" data-testid="note-composer-discard" isDisabled={pending || text.length === 0} onPress={() => {
        updateDraft(''); setFailure(undefined); setFeedback('Draft discarded.')
      }}>Discard draft</Button>
    </div>
    <p role="status" aria-live="polite" aria-atomic="true" data-testid="note-composer-status" {...stylex.props(styles.meta)}>{feedback}</p>
    {/* Every note is agent context: state who it reaches, never that it was delivered. */}
    <p id={ids.reach} data-testid="note-composer-reach" {...stylex.props(styles.meta, styles.reach)}>
      <AgentMarker />
      {target?.startsWith('mission/') ? 'Notes reach agents currently working on this mission.' : 'Notes reach the agent as context.'} People with access can read them too.
    </p>
    {target !== undefined && <p id={ids.target} {...stylex.props(styles.meta)}>Target: {target}.</p>}
    <p id={ids.limit} aria-live="polite" data-testid="note-composer-limit" {...stylex.props(styles.meta, overLimit && styles.error)}>
      {overLimit ? `Too long by ${bytes - noteTextLimitBytes} bytes; notes hold ${noteTextLimitBytes} bytes.` : `${bytes} of ${noteTextLimitBytes} bytes`}
    </p>
    {disabledReason !== undefined && <p id={ids.reason} data-testid="note-composer-disabled-reason" {...stylex.props(styles.meta)}>{disabledReason}</p>}
    {failure !== undefined && <p role="alert" data-testid="note-composer-error" {...stylex.props(styles.meta, styles.error)}>
      Couldn't add the note. Your text and target are kept. Check your connection and Retry.
      {failure.detail !== undefined && <> <span data-testid="note-composer-error-detail">{failure.detail}</span></>}
    </p>}
  </form>
}

const styles = stylex.create({
  root: {
    display: 'flex', flexDirection: 'column', gap: density.composerGap, minWidth: 0, boxSizing: 'border-box', padding: density.composerPad,
    backgroundColor: surface.glassFill, color: text.fg, fontFamily: t.fontSans,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.glassBorder, borderRadius: chrome.composerRadius,
    ':focus-within': { borderColor: border.focusBorder },
  },
  field: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  input: {
    display: 'block', width: '100%', boxSizing: 'border-box', minHeight: g.compactEditorMin, maxHeight: g.editorMax, fieldSizing: 'content',
    overflowY: 'auto', resize: 'none', padding: density.inputPad, borderWidth: 0, borderRadius: chrome.inputRadius, backgroundColor: surface.transparent, color: text.fg,
    fontFamily: t.fontSans, fontSize: density.inputSize, lineHeight: t.uiLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere',
    '::placeholder': { color: text.fgMuted },
    ':focus-visible': { outlineStyle: 'none' },
    ':is([data-invalid])': { color: status.dangerFg },
  },
  footer: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, minWidth: 0 },
  reach: { display: 'flex', alignItems: 'center', gap: s.xs },
  help: { minWidth: 0, fontSize: t.denseSize, lineHeight: t.metaLeading, color: text.fgMuted },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs2, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  grow: { flexGrow: 1 },
  meta: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted, overflowWrap: 'anywhere' },
  error: { color: status.dangerFg },
})
