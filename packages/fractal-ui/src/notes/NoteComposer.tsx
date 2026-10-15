import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Label, TextArea, TextField, ToggleButton, ToggleButtonGroup, type Key } from 'react-aria-components'
import { VisuallyHidden } from 'react-aria'
import { Button, Tooltip } from '../assistant-ui/composition/Controls'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { composerChrome as chrome, density } from '../assistant-ui/embrace-tokens.stylex'
import { AgentMarker } from './AgentMarker'
import { audiencePresentation, type NoteAudienceChoice } from './audience'

/** Server limit for note text (CR06), in UTF-8 bytes. */
export const noteTextLimitBytes = 1024
/** New, unrelated drafts start private. */
export const defaultNoteAudience: NoteAudienceChoice = 'self'
export interface NoteDraft { readonly text: string; readonly audience: NoteAudienceChoice }
const audiences: readonly NoteAudienceChoice[] = ['self', 'agent']
const encoder = new TextEncoder()

export interface NoteComposerProps {
  /** Adds the note. A rejection keeps the text and audience and shows an error with the rejection's message; resolution clears the draft. */
  readonly onSubmit: (text: string, audience: NoteAudienceChoice) => Promise<void>
  /** Why notes cannot be added right now; the draft stays readable and is not submitted. */
  readonly disabledReason?: string | undefined
  readonly autoFocus?: boolean | undefined
  /** Agent authors write agent-audience notes only: the audience is fixed to `agent`. */
  readonly agentAuthor?: boolean | undefined
  readonly draft?: NoteDraft | undefined
  readonly onDraftChange?: ((draft: NoteDraft) => void) | undefined
  readonly target?: string | undefined
  /** Closing never discards the draft; the host retains controlled drafts per target. */
  readonly onClose?: (() => void) | undefined
}

/** Labelled audience selection and multiline note input. */
export function NoteComposer({ onSubmit, disabledReason, autoFocus = false, agentAuthor = false, draft, onDraftChange, target, onClose }: NoteComposerProps) {
  const initialAudience = agentAuthor ? 'agent' : defaultNoteAudience
  const [localDraft, setLocalDraft] = React.useState<NoteDraft>({ text: '', audience: initialAudience })
  const current = draft ?? localDraft
  const updateDraft = (next: NoteDraft) => { setLocalDraft(next); onDraftChange?.(next) }
  const { text } = current
  const [pending, setPending] = React.useState(false)
  const [failure, setFailure] = React.useState<{ readonly detail: string | undefined } | undefined>(undefined)
  const [feedback, setFeedback] = React.useState('')
  const inputRef = React.useRef<HTMLTextAreaElement>(null)
  const ids = { help: React.useId(), limit: React.useId(), reason: React.useId(), audience: React.useId(), target: React.useId() }
  const audience = agentAuthor ? 'agent' : current.audience
  const bytes = encoder.encode(text).length
  const overLimit = bytes > noteTextLimitBytes
  const audienceLocked = agentAuthor || disabledReason !== undefined || pending
  const canSubmit = disabledReason === undefined && !pending && text.trim().length > 0 && !overLimit

  const submit = async () => {
    if (!canSubmit) return
    setPending(true)
    setFailure(undefined)
    setFeedback('Adding note…')
    try {
      await onSubmit(text, audience)
    } catch (error) {
      setPending(false)
      setFeedback('')
      setFailure({ detail: error instanceof Error && error.message.trim() !== '' ? error.message.trim() : undefined })
      inputRef.current?.focus()
      return
    }
    setPending(false)
    updateDraft({ text: '', audience: initialAudience })
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

  const describedBy = [ids.help, ids.limit, target === undefined ? undefined : ids.target, disabledReason === undefined ? undefined : ids.reason].filter(Boolean).join(' ')
  return <form
    data-testid="note-composer"
    data-note-audience={audience}
    aria-busy={pending || undefined}
    onKeyDown={onKeyDown}
    onSubmit={event => { event.preventDefault(); void submit() }}
    {...stylex.props(styles.root)}
  >
    <TextField
      value={text}
      onChange={text => updateDraft({ text, audience })}
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
      <ToggleButtonGroup
        aria-label="Audience"
        aria-describedby={agentAuthor ? ids.audience : undefined}
        selectionMode="single"
        disallowEmptySelection
        selectedKeys={[audience]}
        onSelectionChange={(keys: Set<Key>) => { const [key] = keys; if (key === 'self' || key === 'agent') { updateDraft({ text, audience: key }); setFeedback(`Audience: ${audiencePresentation(key).label}.`) } }}
        onKeyDown={event => {
          if (audienceLocked || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return
          const next = event.key === 'Home' ? 'self' : event.key === 'End' ? 'agent' : audience === 'self' ? 'agent' : 'self'
          updateDraft({ text, audience: next })
          setFeedback(`Audience: ${audiencePresentation(next).label}.`)
        }}
        isDisabled={audienceLocked}
        data-testid="note-audience-toggle"
        {...stylex.props(styles.segments)}
      >
        {audiences.map(choice => {
          const presentation = audiencePresentation(choice)
          return <Tooltip key={choice} label={presentation.description} placement="top">
            <ToggleButton id={choice} data-testid={`note-audience-${choice}`} {...stylex.props(styles.segment)}>
              {choice === 'agent' && <AgentMarker />}
              {presentation.label}
            </ToggleButton>
          </Tooltip>
        })}
      </ToggleButtonGroup>
      <span id={ids.help} data-testid="note-composer-help" {...stylex.props(styles.help)}>
        Audience ·{' '}
        <kbd {...stylex.props(styles.kbd)}>Enter</kbd> adds · <kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd> new line
      </span>
      <span {...stylex.props(styles.grow)} />
      <Button type="submit" variant="outline" size="md" isDisabled={!canSubmit} data-testid="note-composer-submit">
        {pending ? 'Adding…' : failure !== undefined ? 'Retry' : 'Add note'}
      </Button>
      <Button type="button" variant="outline" size="md" data-testid="note-composer-discard" isDisabled={pending || (text.length === 0 && audience === initialAudience)} onPress={() => {
        updateDraft({ text: '', audience: initialAudience }); setFailure(undefined); setFeedback('Draft discarded.')
      }}>Discard draft</Button>
    </div>
    <p role="status" aria-live="polite" aria-atomic="true" data-testid="note-composer-status" {...stylex.props(styles.meta)}>{feedback}</p>
    {target !== undefined && <p id={ids.target} {...stylex.props(styles.meta)}>Target: {target}.{target.startsWith('mission/') && ' Shared notes reach agents currently working on this mission.'}</p>}
    {agentAuthor && <p id={ids.audience} data-testid="note-composer-agent-only" {...stylex.props(styles.meta)}>Agents write notes for agents only. People can see shared notes too.</p>}
    <p id={ids.limit} aria-live="polite" data-testid="note-composer-limit" {...stylex.props(styles.meta, overLimit && styles.error)}>
      {overLimit ? `Too long by ${bytes - noteTextLimitBytes} bytes; notes hold ${noteTextLimitBytes} bytes.` : `${bytes} of ${noteTextLimitBytes} bytes`}
    </p>
    {disabledReason !== undefined && <p id={ids.reason} data-testid="note-composer-disabled-reason" {...stylex.props(styles.meta)}>{disabledReason}</p>}
    {failure !== undefined && <p role="alert" data-testid="note-composer-error" {...stylex.props(styles.meta, styles.error)}>
      Couldn't add the note. Your text, audience and target are kept. Check your connection and Retry.
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
  segments: {
    display: 'inline-flex', flexWrap: 'wrap', maxWidth: '100%', alignItems: 'center', gap: s.xs2, padding: s.xs2, minWidth: 0,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: chrome.controlRadius,
  },
  segment: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minHeight: g.controlSm, minWidth: 0, paddingInline: density.controlX, borderWidth: 0,
    borderRadius: r.sm, backgroundColor: { default: surface.transparent, ':is([data-selected])': surface.rowActive },
    color: { default: text.fgMuted, ':is([data-selected])': text.fg },
    fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'normal', textAlign: 'start',
    cursor: { default: 'pointer', ':is([data-disabled])': 'default' }, opacity: { default: 1, ':is([data-disabled]:not([data-selected]))': 0.64 },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  help: { minWidth: 0, fontSize: t.denseSize, lineHeight: t.metaLeading, color: text.fgMuted },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs2, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  grow: { flexGrow: 1 },
  meta: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted, overflowWrap: 'anywhere' },
  error: { color: status.dangerFg },
})
