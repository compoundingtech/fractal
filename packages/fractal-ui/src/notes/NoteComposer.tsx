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
/** The byte counter appears once the draft is this close to the limit. */
const counterThreshold = noteTextLimitBytes - 128
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
}

/** One-field note input: text, an audience switch (Alt+A), and one confirm (Enter). */
export function NoteComposer({ onSubmit, disabledReason, autoFocus = false, agentAuthor = false }: NoteComposerProps) {
  const defaultAudience: NoteAudienceChoice = agentAuthor ? 'agent' : 'self'
  const [text, setText] = React.useState('')
  const [chosen, setChosen] = React.useState<NoteAudienceChoice>(defaultAudience)
  const [pending, setPending] = React.useState(false)
  const [failure, setFailure] = React.useState<{ readonly detail: string | undefined } | undefined>(undefined)
  const inputRef = React.useRef<HTMLTextAreaElement>(null)
  const ids = { help: React.useId(), limit: React.useId(), reason: React.useId(), audience: React.useId() }
  const audience = agentAuthor ? 'agent' : chosen
  const body = text.trim()
  const bytes = encoder.encode(body).length
  const overLimit = bytes > noteTextLimitBytes
  const audienceLocked = agentAuthor || disabledReason !== undefined || pending
  const canSubmit = disabledReason === undefined && !pending && body.length > 0 && !overLimit
  const altLabel = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌥A' : 'Alt+A'

  const submit = async () => {
    if (!canSubmit) return
    setPending(true)
    setFailure(undefined)
    try {
      await onSubmit(body, audience)
    } catch (error) {
      setPending(false)
      setFailure({ detail: error instanceof Error && error.message.trim() !== '' ? error.message.trim() : undefined })
      inputRef.current?.focus()
      return
    }
    setPending(false)
    setText('')
    setChosen(defaultAudience)
  }

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === 'KeyA') {
      // Option+A would otherwise type a character on macOS.
      event.preventDefault()
      if (!audienceLocked) setChosen(current => current === 'self' ? 'agent' : 'self')
    }
  }
  const onFieldKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      void submit()
    }
  }

  const describedBy = [ids.help, overLimit || bytes > counterThreshold ? ids.limit : undefined, disabledReason === undefined ? undefined : ids.reason].filter(Boolean).join(' ')
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
      onChange={setText}
      isReadOnly={pending || disabledReason !== undefined}
      isInvalid={overLimit}
      autoFocus={autoFocus}
      aria-describedby={describedBy}
      {...stylex.props(styles.field)}
    >
      <VisuallyHidden><Label>Note</Label></VisuallyHidden>
      <TextArea ref={inputRef} rows={1} placeholder="Add a note…" aria-keyshortcuts={agentAuthor ? undefined : 'Alt+A'} data-testid="note-composer-input" onKeyDown={onFieldKeyDown} {...stylex.props(styles.input)} />
    </TextField>
    <div {...stylex.props(styles.footer)}>
      <ToggleButtonGroup
        aria-label="Audience"
        aria-describedby={agentAuthor ? ids.audience : undefined}
        selectionMode="single"
        disallowEmptySelection
        selectedKeys={[audience]}
        onSelectionChange={(keys: Set<Key>) => { const [key] = keys; if (key === 'self' || key === 'agent') setChosen(key) }}
        isDisabled={audienceLocked}
        data-testid="note-audience-toggle"
        {...stylex.props(styles.segments)}
      >
        {audiences.map(choice => {
          const presentation = audiencePresentation(choice)
          return <Tooltip key={choice} label={agentAuthor ? presentation.description : `${presentation.description} ${altLabel} switches audience.`} placement="top">
            <ToggleButton id={choice} data-testid={`note-audience-${choice}`} {...stylex.props(styles.segment)}>
              {choice === 'agent' && <AgentMarker />}
              {presentation.label}
            </ToggleButton>
          </Tooltip>
        })}
      </ToggleButtonGroup>
      <span id={ids.help} data-testid="note-composer-help" {...stylex.props(styles.help)}>
        {agentAuthor ? '' : <><kbd {...stylex.props(styles.kbd)}>{altLabel}</kbd> switches audience · </>}
        <kbd {...stylex.props(styles.kbd)}>Enter</kbd> adds · <kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd> new line
      </span>
      <span {...stylex.props(styles.grow)} />
      <Button type="submit" variant="outline" size="md" isDisabled={!canSubmit && !pending} isPending={pending} data-testid="note-composer-submit">
        {pending ? 'Adding…' : 'Add note'}
      </Button>
    </div>
    {agentAuthor && <p id={ids.audience} data-testid="note-composer-agent-only" {...stylex.props(styles.meta)}>Agents write notes for agents only, so this note is shared with the agent.</p>}
    {(overLimit || bytes > counterThreshold) && <p id={ids.limit} aria-live="polite" data-testid="note-composer-limit" {...stylex.props(styles.meta, overLimit && styles.error)}>
      {overLimit ? `Too long by ${bytes - noteTextLimitBytes} bytes; notes hold ${noteTextLimitBytes} bytes.` : `${bytes} of ${noteTextLimitBytes} bytes`}
    </p>}
    {disabledReason !== undefined && <p id={ids.reason} data-testid="note-composer-disabled-reason" {...stylex.props(styles.meta)}>{disabledReason}</p>}
    {failure !== undefined && <p role="alert" data-testid="note-composer-error" {...stylex.props(styles.meta, styles.error)}>
      Couldn't add the note. Your text is kept; try again.
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
    display: 'inline-flex', alignItems: 'center', gap: s.xs2, padding: s.xs2, flexShrink: 0,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: chrome.controlRadius,
  },
  segment: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, height: g.controlSm, paddingInline: density.controlX, borderWidth: 0,
    borderRadius: r.sm, backgroundColor: { default: surface.transparent, ':is([data-selected])': surface.rowActive },
    color: { default: text.fgMuted, ':is([data-selected])': text.fg },
    fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap',
    cursor: { default: 'pointer', ':is([data-disabled])': 'default' }, opacity: { default: 1, ':is([data-disabled]:not([data-selected]))': 0.64 },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  help: { minWidth: 0, fontSize: t.denseSize, lineHeight: t.metaLeading, color: text.fgMuted },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs2, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  grow: { flexGrow: 1 },
  meta: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted, overflowWrap: 'anywhere' },
  error: { color: status.dangerFg },
})
