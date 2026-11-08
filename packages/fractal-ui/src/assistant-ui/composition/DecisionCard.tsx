import { markdownInline } from './markdown-text'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t, interactionVars } from '../composition-tokens.stylex.ts'
import { Icon } from './Icons'
import { InlineMarkdown } from './Markdown'
import { NarrowLayoutContext } from './NarrowLayout'
import { useCopyFeedback } from './useCopyFeedback'

/**
 * One decision an agent is waiting on, at three densities over the same parts. `docked` sits above a composer and
 * keeps its controls in view by scrolling only its body; `full` is a page's main content; `row` is one line, for a
 * docked card folded away and for an answered one. The card owns presentation and keyboard, never state or transport:
 * the host owns the selection and the words, so a draft survives moving between densities.
 * Width comes from the card's container, never the viewport, so a narrow pane on a wide screen stacks the same way a phone does.
 */
export type DecisionDensity = 'docked' | 'full' | 'row'

export type DecisionOption = {
  readonly id: string
  readonly label: string
  /** What happens next, inside the option's own row; a string is inline Markdown. */
  readonly rationale?: React.ReactNode
  /** Badges the row; the reason reads beside the badge, apart from what happens. */
  readonly recommended?: { readonly reason?: string }
  /** An answer that stops or declines work. It carries no colour at rest: red is for failure. */
  readonly danger?: boolean
}

/** Notes on individual answers, owned by the host: a note goes with its answer while that answer is chosen. */
export type DecisionNotes = {
  readonly values: Readonly<Record<string, string>>
  readonly onChange: (id: string, value: string) => void
  /** The answers that take a note, when not all of them do. */
  readonly only?: readonly string[]
}

const takesNote = (notes: DecisionNotes | undefined, id: string): notes is DecisionNotes => notes !== undefined && (notes.only === undefined || notes.only.includes(id))

export type DecisionPosition = {
  readonly index: number
  readonly count: number
  readonly onPrevious: () => void
  readonly onNext: () => void
}

export type DecisionStatus = { readonly tone: 'progress' | 'error'; readonly text: string }

export type DecisionCardProps = {
  readonly density: DecisionDensity
  /** The card's accessible name. */
  readonly label: string
  /** Who asked, what kind of ask, how long it waited: already-worded facts, in order. */
  readonly meta: readonly string[]
  /** The one heading: the question. */
  readonly heading: React.ReactNode
  /** The question as one line of inline Markdown, for the one-line row. */
  readonly headingText?: string
  /** One line of context, as inline Markdown. */
  readonly context?: string
  /** The context in full. `docked` holds it behind More; `full` shows it. */
  readonly contextFull?: React.ReactNode
  /** What else there is to read, as `DecisionMore`. `docked` holds it behind More; `full` shows it. */
  readonly more?: React.ReactNode
  /** Content the decision cannot be read without (a launch's preview, the call a prompt waits on): always shown. */
  readonly body?: React.ReactNode
  /** The options as one group; empty for an ask answered only in words. */
  readonly options?: readonly DecisionOption[]
  readonly optionsLabel?: string
  /** `one` is a radio group; `any` a set of checkboxes. */
  readonly mode?: 'one' | 'any'
  readonly selected?: readonly string[]
  readonly onSelect?: (ids: readonly string[]) => void
  /** Several alerts in one conversation. */
  readonly pager?: DecisionPosition
  /** Several questions in one ask, answered together. */
  readonly stepper?: DecisionPosition
  /** Lets the person annotate the answers they choose; omit for an ask whose answers carry no words. */
  readonly notes?: DecisionNotes
  /** Omit to show no field. */
  readonly words?: { readonly value: string; readonly placeholder: string; readonly label: string; readonly onChange: (value: string) => void }
  readonly dismiss?: { readonly label: string; readonly onPress: () => void }
  readonly send?: { readonly label: string; readonly disabled: boolean; readonly onPress: () => void; readonly hint?: string }
  /** Set when this device or this kind of ask cannot be answered here: controls show disabled, and this says why and where. */
  readonly unavailable?: React.ReactNode
  /** Everything is disabled at once while an answer is on its way. */
  readonly busy?: boolean
  readonly status?: DecisionStatus
  /** A `docked` card folds to its `row`; a `row` card unfolds to `docked`. */
  readonly onFold?: () => void
  /** A `row` that is the record of an answer rather than a folded card. */
  readonly resolved?: string
  /** Bounds a `docked` card to its host's room; its body scrolls, its controls never do. */
  readonly maxBlockSize?: number
  readonly focusRef?: React.Ref<HTMLElement>
  /** Host markers, as `data-*` attributes on the card. */
  readonly data?: Readonly<Record<`data-${string}`, string>>
}

const isField = (target: EventTarget): boolean => target instanceof HTMLTextAreaElement
const noteOf = (target: EventTarget): string | undefined => target instanceof HTMLTextAreaElement ? target.dataset['noteFor'] : undefined
const isChoice = (target: EventTarget): boolean => target instanceof HTMLInputElement
const isBare = (event: React.KeyboardEvent): boolean => !event.altKey && !event.ctrlKey && !event.metaKey

/** A raw reference: shown as itself, copied with a press. */
export function DecisionRef({ value }: { readonly value: string }): React.ReactNode {
  const { copy, state } = useCopyFeedback(value, () => navigator.clipboard.writeText(value))
  return <Aria.Button aria-label={`Copy ${value}`} onPress={() => void copy()} {...stylex.props(styles.ref)}>
    <span {...stylex.props(styles.refText)}>{value}</span>
    <Icon name={state.tag === 'copied' ? 'check' : 'copy'} size={12} />
  </Aria.Button>
}

/** Where this card sits among several: alerts page; questions step, with how far along the ask is. */
function Position({ noun, position, touch, progress = false }: { readonly noun: string; readonly position: DecisionPosition; readonly touch: boolean; readonly progress?: boolean }) {
  const buttonStyle = stylex.props(styles.pageButton, touch && styles.touchSquare)
  return <span role="group" aria-label={`${noun} ${position.index + 1} of ${position.count}`} {...stylex.props(styles.position)}>
    {progress
      ? <span aria-hidden="true" {...stylex.props(styles.segments)}>{Array.from({ length: position.count }, (_, index) => <span key={index} data-step={index <= position.index ? 'done' : 'todo'} {...stylex.props(styles.segment, index <= position.index && styles.segmentDone)} />)}</span>
      : null}
    <Aria.Button aria-label={`Previous ${noun}`} isDisabled={position.index === 0} onPress={position.onPrevious} {...buttonStyle}><span {...stylex.props(styles.flip)}><Icon name="chevron-right" size={14} /></span></Aria.Button>
    <span {...stylex.props(styles.positionText)}>
      {progress ? <><span {...stylex.props(styles.positionNoun)}>{`${noun[0]!.toUpperCase()}${noun.slice(1)}`}</span>{' '}</> : null}
      <span>{position.index + 1} of {position.count}</span>
    </span>
    <Aria.Button aria-label={`Next ${noun}`} isDisabled={position.index === position.count - 1} onPress={position.onNext} {...buttonStyle}><Icon name="chevron-right" size={14} /></Aria.Button>
  </span>
}

/** Who asked, whole; then what kind of ask and how long it waited, which give way first. */
function Sender({ meta }: { readonly meta: readonly string[] }) {
  const [who, ...rest] = meta
  return <>
    <span {...stylex.props(styles.senderWho)}>{who}</span>
    {rest.length === 0 ? null : <>{' '}<span {...stylex.props(styles.senderRest)}>· {rest.join(' · ')}</span></>}
  </>
}

/** A note on one chosen answer: Enter or Escape keeps it and returns to the answers. */
function NoteField({ option, value, onChange, disabled, touch, fieldRef }: { readonly option: DecisionOption; readonly value: string; readonly onChange: (value: string) => void; readonly disabled: boolean; readonly touch: boolean; readonly fieldRef: React.Ref<HTMLTextAreaElement> }) {
  const field = React.useRef<HTMLTextAreaElement | null>(null)
  React.useLayoutEffect(() => {
    const node = field.current
    if (node === null) return
    node.style.height = 'auto'
    node.style.height = `${node.scrollHeight + node.offsetHeight - node.clientHeight}px`
  }, [value])
  const ref = (node: HTMLTextAreaElement | null) => {
    field.current = node
    if (typeof fieldRef === 'function') fieldRef(node)
    else if (fieldRef !== null) (fieldRef as React.MutableRefObject<HTMLTextAreaElement | null>).current = node
  }
  return <textarea ref={ref} rows={1} data-note-for={option.id} aria-label={`Note for ${option.label}`} placeholder="Add a note to this answer…" value={value} disabled={disabled} onChange={event => onChange(event.target.value)} {...stylex.props(styles.field, styles.noteField, touch && styles.fieldTouch)} />
}

type Noting = { readonly id: string | undefined; readonly open: (id: string) => void; readonly fieldRef: React.Ref<HTMLTextAreaElement> }

function OptionRow({ option, index, on, any, disabled, touch, first, notes, noting }: { readonly option: DecisionOption; readonly index: number; readonly on: boolean; readonly any: boolean; readonly disabled: boolean; readonly touch: boolean; readonly first: boolean; readonly notes?: DecisionNotes; readonly noting: Noting }) {
  const noted = takesNote(notes, option.id)
  const note = noted ? notes.values[option.id] ?? '' : ''
  // A note belongs to a chosen answer: it shows while that answer is chosen and is being written or holds words.
  const showNote = noted && on && (noting.id === option.id || note.trim() !== '')
  const offerNote = noted && on && !showNote && !disabled
  const inner = (focused: boolean, hovered: boolean) => <span {...stylex.props(styles.option, touch && styles.optionTouch, hovered && !disabled && !on && styles.optionHover, focused && styles.optionFocus, disabled && styles.optionDisabled)}>
    <span aria-hidden="true" {...stylex.props(styles.indicator, any && styles.indicatorSquare, on && styles.indicatorOn)}>
      {on ? any ? <Icon name="check" size={12} /> : <span {...stylex.props(styles.dot)} /> : null}
    </span>
    <span {...stylex.props(styles.optionText)}>
      <span {...stylex.props(styles.optionLabel)}><InlineMarkdown text={markdownInline(option.label)} /></span>
      {option.recommended === undefined ? null : <span {...stylex.props(styles.recommendLine)}>
        <span {...stylex.props(styles.badge)}>Recommended</span>
        {option.recommended.reason === undefined ? null : <span {...stylex.props(styles.reason)}><InlineMarkdown text={markdownInline(option.recommended.reason)} /></span>}
      </span>}
      {option.rationale === undefined ? null : <span {...stylex.props(styles.rationale)}>{typeof option.rationale === 'string' ? <InlineMarkdown text={markdownInline(option.rationale)} /> : option.rationale}</span>}
    </span>
    {touch || index >= 9 ? null : <kbd aria-hidden="true" {...stylex.props(styles.key, on && styles.keyOn)}>{index + 1}</kbd>}
  </span>
  const choice = any
    ? <Aria.Checkbox value={option.id} isDisabled={disabled} {...stylex.props(styles.optionHost)}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Checkbox>
    : <Aria.Radio value={option.id} isDisabled={disabled} {...stylex.props(styles.optionHost)}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Radio>
  return <div data-answer={option.id} {...stylex.props(styles.answer, !first && styles.answerRuled, on && styles.answerOn)}>
    {choice}
    {showNote || offerNote ? <div {...stylex.props(styles.annex)}>
      {showNote
        ? <NoteField option={option} value={note} onChange={value => notes!.onChange(option.id, value)} disabled={disabled} touch={touch} fieldRef={noting.id === option.id ? noting.fieldRef : null} />
        : <Aria.Button aria-label={`Add note${touch ? '' : ' n'} to ${option.label}`} onPress={() => noting.open(option.id)} {...stylex.props(styles.noteButton, touch && styles.noteTouch)}><Icon name="pencil" size={12} />Add note{touch ? null : <>{' '}<kbd {...stylex.props(styles.noteKey)}>n</kbd></>}</Aria.Button>}
    </div> : null}
  </div>
}

function Options({ card, disabled, touch, noting }: { readonly card: DecisionCardProps; readonly disabled: boolean; readonly touch: boolean; readonly noting: Noting }) {
  const options = card.options ?? []
  if (options.length === 0) return null
  const selected = card.selected ?? []
  const any = card.mode === 'any'
  const rows = options.map((option, index) => <OptionRow key={option.id} option={option} index={index} on={selected.includes(option.id)} any={any} disabled={disabled} touch={touch} first={index === 0} noting={noting} {...(card.notes === undefined ? {} : { notes: card.notes })} />)
  const label = card.optionsLabel ?? 'Answers'
  return any
    ? <Aria.CheckboxGroup aria-label={label} value={[...selected]} isDisabled={disabled} onChange={ids => card.onSelect?.(ids)} {...stylex.props(styles.options)}>{rows}</Aria.CheckboxGroup>
    : <Aria.RadioGroup aria-label={label} value={selected[0] ?? null} isDisabled={disabled} onChange={id => card.onSelect?.([id])} {...stylex.props(styles.options)}>{rows}</Aria.RadioGroup>
}

/** Grows with its words up to the editor's maximum, then scrolls. */
function WordsField({ words, disabled, touch }: { readonly words: NonNullable<DecisionCardProps['words']>; readonly disabled: boolean; readonly touch: boolean }) {
  const field = React.useRef<HTMLTextAreaElement>(null)
  React.useLayoutEffect(() => {
    const node = field.current
    if (node === null) return
    node.style.height = 'auto'
    node.style.height = `${node.scrollHeight + node.offsetHeight - node.clientHeight}px`
  }, [words.value])
  return <textarea ref={field} rows={1} aria-label={words.label} placeholder={words.placeholder} value={words.value} disabled={disabled} onChange={event => words.onChange(event.target.value)} {...stylex.props(styles.field, touch && styles.fieldTouch)} />
}

function Row({ card, touch }: { readonly card: DecisionCardProps; readonly touch: boolean }) {
  const chosen = card.options?.filter(option => card.selected?.includes(option.id)).map(option => option.label).join(', ')
  return <div {...stylex.props(styles.row, touch && styles.rowTouch)}>
    <span {...stylex.props(styles.rowText)}>
      <span {...stylex.props(styles.rowHeading)}>{card.resolved ?? (card.headingText === undefined ? card.heading : <InlineMarkdown text={markdownInline(card.headingText)} />)}</span>
      {card.resolved !== undefined || chosen === undefined || chosen === '' ? null : <span {...stylex.props(styles.rowChosen)}>{chosen}</span>}
    </span>
    {card.status === undefined ? null : <span role={card.status.tone === 'error' ? 'alert' : 'status'} {...stylex.props(styles.rowStatus, card.status.tone === 'error' && styles.statusError)}>{card.status.text}</span>}
    {card.pager === undefined ? null : <Position noun="alert" position={card.pager} touch={touch} />}
    {card.send === undefined || card.resolved !== undefined ? null : <Aria.Button isDisabled={card.send.disabled || card.busy === true} onPress={card.send.onPress} {...stylex.props(styles.send, touch && styles.touchButton)}>{card.send.label}</Aria.Button>}
    {card.onFold === undefined ? null : <Aria.Button aria-label="Show this decision" onPress={card.onFold} {...stylex.props(styles.pageButton, touch && styles.touchSquare)}><Icon name="chevron-down" size={12} /></Aria.Button>}
  </div>
}

export function DecisionCard(card: DecisionCardProps): React.ReactNode {
  const touch = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  const [notingId, setNotingId] = React.useState<string>()
  const noteField = React.useRef<HTMLTextAreaElement | null>(null)
  const panelId = React.useId()
  const full = card.density === 'full'
  const disabled = card.busy === true || card.unavailable !== undefined
  const options = card.options ?? []
  const hasMore = card.contextFull !== undefined || card.more !== undefined
  const setSelected = (ids: readonly string[]) => card.onSelect?.(ids)
  const canSend = card.send !== undefined && !card.send.disabled && !disabled
  /** Chooses an answer, as its row's key or press would, and opens its note. */
  const openNote = (id: string) => {
    const selected = card.selected ?? []
    if (!selected.includes(id)) setSelected(card.mode === 'any' ? [...selected, id] : [id])
    setNotingId(id)
  }
  // The note opened by a key or a press takes focus once it is on screen.
  React.useLayoutEffect(() => { if (notingId !== undefined) noteField.current?.focus() }, [notingId])
  const backToAnswer = (root: HTMLElement, id: string) => {
    setNotingId(undefined)
    root.querySelector<HTMLInputElement>(`[data-answer="${CSS.escape(id)}"] input`)?.focus()
  }
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing) return
    const note = noteOf(event.target)
    if (note !== undefined) {
      // A note is kept as written; Enter and Escape both return to the answers, where Enter sends.
      if ((event.key === 'Enter' && !event.shiftKey && isBare(event)) || event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        backToAnswer(event.currentTarget, note)
      }
      return
    }
    if (isField(event.target)) {
      if (event.key === 'Enter' && !event.shiftKey && isBare(event)) {
        event.preventDefault()
        if (canSend) card.send!.onPress()
      } else if (event.key === 'Escape' && card.words !== undefined && card.words.value !== '') {
        event.preventDefault()
        event.stopPropagation()
        card.words.onChange('')
      }
      return
    }
    if (!isBare(event)) return
    if (event.key === 'Enter' && !event.shiftKey && (isChoice(event.target) || event.target === event.currentTarget)) {
      event.preventDefault()
      if (canSend) card.send!.onPress()
    } else if (event.key === 'Escape' && (card.selected?.length ?? 0) > 0 && !disabled) {
      event.preventDefault()
      event.stopPropagation()
      setSelected([])
    } else if (event.key === 'n' && isChoice(event.target) && !disabled) {
      const id = (event.target as HTMLInputElement).value
      if (!takesNote(card.notes, id) || !options.some(option => option.id === id)) return
      event.preventDefault()
      openNote(id)
    } else if (/^[1-9]$/.test(event.key) && !disabled) {
      const option = options[Number(event.key) - 1]
      if (option === undefined) return
      event.preventDefault()
      const selected = card.selected ?? []
      setSelected(card.mode !== 'any' ? [option.id] : selected.includes(option.id) ? selected.filter(id => id !== option.id) : [...selected, option.id])
      // Roving continues from the chosen row; focus is already inside this card, so nothing is taken from elsewhere.
      event.currentTarget.querySelectorAll<HTMLInputElement>('[data-answer] input')[Number(event.key) - 1]?.focus()
    }
  }
  const noting: Noting = { id: notingId, open: openNote, fieldRef: noteField }
  const frame = {
    ref: card.focusRef as React.Ref<HTMLElement>, tabIndex: -1, 'aria-label': card.label, onKeyDownCapture: onKeyDown, ...card.data,
  }
  if (card.density === 'row') {
    return <div {...stylex.props(styles.container)}>
      <section {...frame} {...stylex.props(styles.card, styles.cardRow)}><Row card={card} touch={touch} /></section>
    </div>
  }
  return <div {...stylex.props(styles.container)}>
    <section {...frame} style={card.maxBlockSize === undefined || full ? undefined : { maxBlockSize: card.maxBlockSize }} {...stylex.props(styles.card, full && styles.cardFull)}>
      <header {...stylex.props(styles.header, full && styles.headerFull)}>
        <Sender meta={card.meta} />
        <span {...stylex.props(styles.headerControls)}>
          {card.stepper === undefined ? null : <Position noun="question" position={card.stepper} touch={touch} progress />}
          {card.pager === undefined ? null : <Position noun="alert" position={card.pager} touch={touch} />}
          {card.onFold === undefined ? null : <Aria.Button aria-label="Fold this decision" onPress={card.onFold} {...stylex.props(styles.pageButton, touch && styles.touchSquare)}><span {...stylex.props(styles.flip)}><Icon name="chevron-down" size={14} /></span></Aria.Button>}
        </span>
      </header>
      <div {...stylex.props(styles.scroll, full && styles.scrollFull)}>
        <div {...stylex.props(styles.question)}>
          <div role="heading" aria-level={2} {...stylex.props(styles.heading, full && styles.headingFull)}>{card.heading}</div>
          {full && card.contextFull !== undefined
            ? <div {...stylex.props(styles.contextFull)}>{card.contextFull}</div>
            : card.context === undefined || (open && !full && card.contextFull !== undefined) ? null : <p {...stylex.props(styles.context)}><InlineMarkdown text={markdownInline(card.context)} /></p>}
          {full || !hasMore ? null : <Aria.Button aria-expanded={open} aria-controls={open ? panelId : undefined} onPress={() => setOpen(value => !value)} {...stylex.props(styles.moreButton, touch && styles.moreTouch)}>
            More
            <span {...stylex.props(styles.chevron, open && styles.chevronOpen)}><Icon name="chevron-down" size={12} /></span>
          </Aria.Button>}
          {!full && open ? <div id={panelId} {...stylex.props(styles.more)}>{card.more ?? card.contextFull}</div> : null}
          {full && card.more !== undefined ? <div {...stylex.props(styles.more)}>{card.more}</div> : null}
        </div>
        {card.body === undefined ? null : <div {...stylex.props(styles.body)}>{card.body}</div>}
        <Options card={card} disabled={disabled} touch={touch} noting={noting} />
      </div>
      {card.unavailable === undefined ? null : <p {...stylex.props(styles.unavailable, full && styles.flush)}>{card.unavailable}</p>}
      {card.status === undefined ? null : <p role={card.status.tone === 'error' ? 'alert' : 'status'} {...stylex.props(styles.status, card.status.tone === 'error' && styles.statusError, full && styles.flush)}>
        {card.status.tone === 'error' ? <span aria-hidden="true" {...stylex.props(styles.statusIcon)}><Icon name="alert" size={12} /></span> : null}
        {card.status.text}
      </p>}
      {card.unavailable !== undefined || (card.words === undefined && card.dismiss === undefined && card.send === undefined) ? null : <div {...stylex.props(styles.quiet, full && styles.flush, full && styles.quietFull)}>
        {card.words === undefined ? <span {...stylex.props(styles.spacer)} /> : <WordsField words={card.words} disabled={disabled} touch={touch} />}
        <span {...stylex.props(styles.actions)}>
          {card.dismiss === undefined ? null : <Aria.Button isDisabled={disabled} onPress={card.dismiss.onPress} {...stylex.props(styles.dismiss, touch && styles.touchButton)}>{card.dismiss.label}</Aria.Button>}
          {card.send === undefined ? null : <Aria.Button isDisabled={disabled || card.send.disabled} aria-description={card.send.hint} onPress={card.send.onPress} {...stylex.props(styles.send, touch && styles.touchButton)}>
{card.send.label}</Aria.Button>}
        </span>
      </div>}
    </section>
  </div>
}

const narrow = '@container decision-card (max-width: 480px)'
const focusRing = { ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: g.focusOffset } }
const quietButton = { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: g.controlMd, paddingInline: s.row, borderRadius: r.sm, borderWidth: 0, fontFamily: t.fontSans, fontSize: t.metaSize, fontWeight: t.weightMedium, cursor: 'pointer', whiteSpace: 'nowrap', ...focusRing, ':disabled': { opacity: 0.45, cursor: 'default' } }

const styles = stylex.create({
  container: { width: '100%', minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column', containerType: 'inline-size', containerName: 'decision-card' },
  card: { display: 'flex', flexDirection: 'column', minHeight: 0, boxSizing: 'border-box', color: c.fg, backgroundColor: c.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.lg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, outline: 'none' },
  cardFull: { backgroundColor: 'transparent', borderWidth: 0, borderRadius: r.none },
  cardRow: { borderRadius: r.base },
  // Who asked is read first and given way last: the kind and wait shrink, then the controls wrap below.
  header: { flexShrink: 0, display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.sm, rowGap: s.xs2, minHeight: g.controlMd, paddingBlockStart: s.md, paddingInline: s.lg, fontSize: t.metaSize, lineHeight: t.metaLeading },
  headerFull: { paddingInline: 0, paddingBlockStart: 0 },
  senderWho: { flexShrink: 0, display: 'inline-block', maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fg, fontWeight: t.weightMedium },
  senderRest: { flexGrow: 1, flexShrink: 1, flexBasis: 0, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fgSoft },
  headerControls: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.xs, marginInlineStart: 'auto' },
  position: { display: 'inline-flex', alignItems: 'center', gap: s.xs2, flexShrink: 0, color: c.fgSoft, fontSize: t.metaSize },
  positionText: { fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', paddingInline: s.xs2 },
  // A narrow card keeps the count and drops the word and the progress first.
  positionNoun: { display: { default: 'inline', [narrow]: 'none' } },
  segments: { display: { default: 'inline-flex', [narrow]: 'none' }, gap: s.xs2, marginInlineEnd: s.sm },
  segment: { width: '14px', height: '3px', borderRadius: r.full, backgroundColor: c.border },
  segmentDone: { backgroundColor: c.fgSoft },
  pageButton: { ...quietButton, flexShrink: 0, width: g.controlSm, minHeight: g.controlSm, paddingInline: 0, backgroundColor: 'transparent', color: c.fgSoft, ':hover': { backgroundColor: c.rowHover, color: c.fg } },
  touchSquare: { width: g.touchTarget, minHeight: g.touchTarget },
  touchButton: { minHeight: g.touchTarget, paddingInline: s.xl },
  flip: { display: 'inline-flex', transform: 'rotate(180deg)' },
  // Only this part scrolls: a tall decision can never push Send out of reach.
  scroll: { flexGrow: 1, flexShrink: 1, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain', display: 'flex', flexDirection: 'column', gap: s.lg, paddingBlockStart: s.sm, paddingBlockEnd: s.lg, paddingInline: s.lg },
  scrollFull: { overflowY: 'visible', gap: s.xl, paddingInline: 0, paddingBlock: s.lg },
  // The question, its context and More read as one block, closer together than the answers below them.
  question: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.xs },
  heading: { flexShrink: 0, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightSemibold, overflowWrap: 'anywhere' },
  headingFull: { fontSize: g.dialogTitle, lineHeight: 1.35 },
  context: { flexShrink: 0, margin: 0, color: c.fgSoft, overflowWrap: 'anywhere', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' },
  contextFull: { flexShrink: 0, color: c.fgSoft, overflowWrap: 'anywhere' },
  moreButton: { flexShrink: 0, display: 'inline-flex', alignSelf: 'flex-start', alignItems: 'center', gap: s.xs, minHeight: g.controlSm, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg }, ...focusRing },
  moreTouch: { minHeight: g.touchTarget },
  chevron: { display: 'inline-flex', transitionProperty: 'transform', transitionDuration: '120ms' },
  chevronOpen: { transform: 'rotate(180deg)' },
  more: { flexShrink: 0, paddingBlockStart: s.xs, display: 'flex', flexDirection: 'column', gap: s.md, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  body: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  ref: { display: 'inline-flex', alignItems: 'center', gap: s.xs, maxWidth: '100%', minHeight: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: c.fg }, ...focusRing },
  refText: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  // One surface: answers are rows split by hairlines, not boxes inside a box.
  options: { flexShrink: 0, display: 'flex', flexDirection: 'column', marginInline: `calc(-1 * ${s.sm})` },
  answer: { borderRadius: r.base, transitionProperty: 'background-color', transitionDuration: '80ms' },
  answerRuled: { borderBlockStartWidth: g.hairline, borderBlockStartStyle: 'solid', borderBlockStartColor: c.border },
  // The chosen answer carries the selection colour, readable in both themes; a grey fill was not.
  answerOn: { backgroundColor: `color-mix(in srgb, ${c.primary} 9%, transparent)`, borderBlockStartColor: 'transparent' },
  optionHost: { display: 'block', position: 'relative', cursor: 'pointer', outline: 'none' },
  option: { display: 'flex', alignItems: 'flex-start', gap: s.row, paddingBlock: s.md, paddingInline: s.sm, borderRadius: r.base },
  optionTouch: { minHeight: g.touchTarget, boxSizing: 'border-box', paddingBlock: s.row },
  optionHover: { backgroundColor: c.rowHover },
  optionFocus: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: g.focusInset },
  optionDisabled: { cursor: 'default', opacity: 0.6 },
  indicator: { flexShrink: 0, boxSizing: 'border-box', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, marginBlockStart: s.xs2, borderRadius: r.full, borderWidth: '1.5px', borderStyle: 'solid', borderColor: c.fgSoft, color: c.onPrimary },
  indicatorSquare: { borderRadius: r.keyboard },
  indicatorOn: { borderColor: c.primary, backgroundColor: c.primary },
  dot: { width: '6px', height: '6px', borderRadius: r.full, backgroundColor: c.onPrimary },
  optionText: { minWidth: 0, flexGrow: 1, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  optionLabel: { fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  // The reason sits with its badge, apart from what the answer does.
  recommendLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.sm, rowGap: s.xs2 },
  badge: { flexShrink: 0, paddingInline: s.sm, borderRadius: r.full, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, color: c.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  reason: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  rationale: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  // A shortcut hint, trailing, so it is not mistaken for the selection mark.
  key: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minWidth: '18px', height: '18px', marginBlockStart: '1px', paddingInline: s.xs2, boxSizing: 'border-box', borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: 1, fontVariantNumeric: 'tabular-nums' },
  keyOn: { borderColor: c.borderStrong },
  // A chosen answer's note sits under its words, aligned with them.
  annex: { display: 'flex', paddingInlineStart: `calc(${s.sm} + ${g.icon} + ${s.row})`, paddingInlineEnd: s.sm, paddingBlockEnd: s.md, marginBlockStart: `calc(-1 * ${s.xs})` },
  noteField: { flexBasis: 'auto', backgroundColor: c.raised },
  noteButton: { ...quietButton, gap: s.xs, minHeight: g.controlSm, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, backgroundColor: 'transparent', color: c.fgSoft, fontWeight: t.weightRegular, ':hover': { color: c.fg } },
  noteTouch: { minHeight: g.touchTarget },
  noteKey: { fontFamily: t.fontSans, fontSize: t.denseSize, color: c.fgMuted, marginInlineStart: s.xs },
  unavailable: { flexShrink: 0, margin: 0, paddingBlock: s.sm, paddingInline: s.lg, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  status: { flexShrink: 0, display: 'flex', alignItems: 'baseline', gap: s.sm, margin: 0, paddingBlockEnd: s.sm, paddingInline: s.lg, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  statusError: { color: c.dangerFg },
  statusIcon: { display: 'inline-flex', alignSelf: 'center' },
  // The quiet row: it never scrolls away, and neither of its buttons reads as an option.
  quiet: { flexShrink: 0, display: 'flex', flexWrap: { default: 'nowrap', [narrow]: 'wrap' }, alignItems: 'flex-end', justifyContent: 'flex-end', gap: s.sm, paddingBlock: s.md, paddingInline: s.lg, borderBlockStartWidth: g.hairline, borderBlockStartStyle: 'solid', borderBlockStartColor: c.border, backgroundColor: c.washSubtle },
  flush: { paddingInline: 0 },
  // A long decision scrolls its page; its controls stay on screen, as they do docked.
  quietFull: { position: 'sticky', bottom: 0, backgroundColor: c.canvas },
  spacer: { flexGrow: 1 },
  field: { flexGrow: 1, flexShrink: 1, flexBasis: { default: 0, [narrow]: '100%' }, minWidth: 0, minHeight: g.controlMd, maxHeight: g.editorMax, boxSizing: 'border-box', resize: 'none', paddingBlock: s.xs, paddingInline: s.md, borderRadius: r.base, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, backgroundColor: c.controlFill, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, '::placeholder': { color: c.fgSoft }, outlineStyle: 'none', ':focus': { borderColor: interactionVars.focusRing }, ':disabled': { opacity: 0.6 } },
  fieldTouch: { minHeight: g.touchTarget, fontSize: t.touchInputSize, lineHeight: t.uiLeading },
  actions: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.sm },
  dismiss: { ...quietButton, backgroundColor: 'transparent', color: c.fgSoft, ':hover': { backgroundColor: c.rowHover, color: c.fg } },
  // A disabled Send is a neutral control, not a dimmed primary that still looks live.
  send: { ...quietButton, backgroundColor: c.primary, color: c.onPrimary, ':hover': { backgroundColor: c.primaryHover }, ':disabled': { opacity: 1, backgroundColor: c.rowActive, color: c.fgSoft, cursor: 'default' } },
  row: { display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlLg, paddingBlock: s.xs, paddingInline: s.lg },
  rowTouch: { minHeight: g.touchTarget },
  rowText: { flexGrow: 1, minWidth: 0, display: 'flex', alignItems: 'baseline', gap: s.md, overflow: 'hidden' },
  rowHeading: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, fontWeight: t.weightMedium },
  rowChosen: { flexShrink: 0, display: { default: 'inline', [narrow]: 'none' }, maxWidth: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fgSoft, fontSize: t.metaSize },
  rowStatus: { flexShrink: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fgSoft, fontSize: t.metaSize },
})
