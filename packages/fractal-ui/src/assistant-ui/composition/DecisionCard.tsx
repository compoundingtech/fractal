import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../composition-tokens.stylex.ts'
import { Icon } from './Icons'
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
  /** What happens next, inside the option's own row. */
  readonly rationale?: React.ReactNode
  /** Badges the row; a reason shows beside the rationale. */
  readonly recommended?: { readonly reason?: string }
  readonly danger?: boolean
}

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
  /** The question as plain text, for the one-line row. */
  readonly headingText?: string
  /** One line of context, as plain text. */
  readonly context?: string
  /** The context in full. `docked` holds it behind More; `full` shows it. */
  readonly contextFull?: React.ReactNode
  /** What else there is to read, as `DecisionMore`. `docked` holds it behind More; `full` shows it. */
  readonly more?: React.ReactNode
  /** More starts open. */
  readonly moreOpen?: boolean
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

function Position({ noun, position, touch }: { readonly noun: string; readonly position: DecisionPosition; readonly touch: boolean }) {
  const buttonStyle = stylex.props(styles.pageButton, touch && styles.touchSquare)
  return <span role="group" aria-label={`${noun} ${position.index + 1} of ${position.count}`} {...stylex.props(styles.position)}>
    <Aria.Button aria-label={`Previous ${noun}`} isDisabled={position.index === 0} onPress={position.onPrevious} {...buttonStyle}><span {...stylex.props(styles.flip)}><Icon name="chevron-right" size={12} /></span></Aria.Button>
    <span {...stylex.props(styles.positionText)}>{position.index + 1} of {position.count}</span>
    <Aria.Button aria-label={`Next ${noun}`} isDisabled={position.index === position.count - 1} onPress={position.onNext} {...buttonStyle}><Icon name="chevron-right" size={12} /></Aria.Button>
  </span>
}

function OptionRow({ option, index, on, any, disabled, touch }: { readonly option: DecisionOption; readonly index: number; readonly on: boolean; readonly any: boolean; readonly disabled: boolean; readonly touch: boolean }) {
  const inner = (focused: boolean, hovered: boolean) => <span {...stylex.props(styles.option, touch && styles.optionTouch, hovered && !disabled && styles.optionHover, on && styles.optionOn, focused && styles.optionFocus, disabled && styles.optionDisabled)}>
    <span aria-hidden="true" {...stylex.props(styles.key, any && styles.keySquare, on && styles.keyOn)}>{any ? (on ? <Icon name="check" size={12} /> : null) : index < 9 ? index + 1 : null}</span>
    <span {...stylex.props(styles.optionText)}>
      <span {...stylex.props(styles.optionTop)}>
        <span {...stylex.props(styles.optionLabel, option.danger === true && styles.optionDanger)}>{option.label}</span>
        {option.recommended === undefined ? null : <span {...stylex.props(styles.badge)}>Recommended</span>}
      </span>
      {option.recommended?.reason === undefined ? null : <span {...stylex.props(styles.reason)}>{option.recommended.reason}</span>}
      {option.rationale === undefined ? null : <span {...stylex.props(styles.rationale)}>{option.rationale}</span>}
    </span>
  </span>
  return any
    ? <Aria.Checkbox value={option.id} isDisabled={disabled} {...stylex.props(styles.optionHost)}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Checkbox>
    : <Aria.Radio value={option.id} isDisabled={disabled} {...stylex.props(styles.optionHost)}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Radio>
}

function Options({ card, choose, disabled, touch }: { readonly card: DecisionCardProps; readonly choose: (ids: readonly string[]) => void; readonly disabled: boolean; readonly touch: boolean }) {
  const options = card.options ?? []
  if (options.length === 0) return null
  const selected = card.selected ?? []
  const any = card.mode === 'any'
  const rows = options.map((option, index) => <OptionRow key={option.id} option={option} index={index} on={selected.includes(option.id)} any={any} disabled={disabled} touch={touch} />)
  const label = card.optionsLabel ?? 'Answers'
  return any
    ? <Aria.CheckboxGroup aria-label={label} value={[...selected]} isDisabled={disabled} onChange={choose} {...stylex.props(styles.options)}>{rows}</Aria.CheckboxGroup>
    : <Aria.RadioGroup aria-label={label} value={selected[0] ?? null} isDisabled={disabled} onChange={id => choose([id])} {...stylex.props(styles.options)}>{rows}</Aria.RadioGroup>
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

function Row({ card, send, pager, touch }: { readonly card: DecisionCardProps; readonly send: () => void; readonly pager: DecisionPosition | undefined; readonly touch: boolean }) {
  const chosen = card.options?.filter(option => card.selected?.includes(option.id)).map(option => option.label).join(', ')
  return <div {...stylex.props(styles.row, touch && styles.rowTouch)}>
    <span {...stylex.props(styles.rowText)}>
      <span {...stylex.props(styles.rowHeading)}>{card.resolved ?? card.headingText ?? card.heading}</span>
      {card.resolved !== undefined || chosen === undefined || chosen === '' ? null : <span {...stylex.props(styles.rowChosen)}>{chosen}</span>}
    </span>
    {card.status === undefined ? null : <span role={card.status.tone === 'error' ? 'alert' : 'status'} {...stylex.props(styles.rowStatus, card.status.tone === 'error' && styles.statusError)}>{card.status.text}</span>}
    {pager === undefined ? null : <Position noun="alert" position={pager} touch={touch} />}
    {card.send === undefined || card.resolved !== undefined ? null : <Aria.Button isDisabled={card.send.disabled || card.busy === true} onPress={send} {...stylex.props(styles.send, touch && styles.touchButton)}>{card.send.label}</Aria.Button>}
    {card.onFold === undefined ? null : <Aria.Button aria-label="Show this decision" onPress={card.onFold} {...stylex.props(styles.pageButton, touch && styles.touchSquare)}><Icon name="chevron-down" size={12} /></Aria.Button>}
  </div>
}

export function DecisionCard(card: DecisionCardProps): React.ReactNode {
  const touch = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(card.moreOpen === true)
  const panelId = React.useId()
  const full = card.density === 'full'
  const disabled = card.busy === true || card.unavailable !== undefined
  const options = card.options ?? []
  const hasMore = card.contextFull !== undefined || card.more !== undefined
  // Enter answers only what the person chose or wrote on this question, never a preselection or a draft they may not have
  // read: focus follows into the next card as one is answered, and the same Enter must not land on it. Send always answers.
  const chosen = React.useRef(false)
  const wrote = React.useRef(false)
  const choose = (ids: readonly string[]) => {
    chosen.current = true
    card.onSelect?.(ids)
  }
  const words = card.words === undefined ? undefined : {
    ...card.words,
    onChange: (value: string) => {
      wrote.current = true
      card.words?.onChange(value)
    },
  }
  const forget = () => {
    chosen.current = false
    wrote.current = false
  }
  const send = () => {
    forget()
    card.send?.onPress()
  }
  const enter = () => {
    const deliberate = chosen.current || (wrote.current && (card.words?.value.trim() ?? '') !== '')
    if (deliberate && card.send !== undefined && !card.send.disabled && !disabled) send()
  }
  // A choice belongs to the question it was made on; moving to another question or alert starts over.
  const moving = (position: DecisionPosition | undefined): DecisionPosition | undefined => position === undefined ? undefined : {
    ...position,
    onPrevious: () => { forget(); position.onPrevious() },
    onNext: () => { forget(); position.onNext() },
  }
  const pager = moving(card.pager)
  const stepper = moving(card.stepper)
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing) return
    // A held Enter repeats into whatever took focus after the first press answered, field, choice, frame or button alike:
    // only a fresh press acts. Shift+Enter still repeats newlines.
    if (event.key === 'Enter' && event.repeat && !event.shiftKey && isBare(event)) {
      event.preventDefault()
      return
    }
    if (isField(event.target)) {
      if (event.key === 'Enter' && !event.shiftKey && isBare(event)) {
        event.preventDefault()
        enter()
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
      enter()
    } else if (event.key === 'Escape' && (card.selected?.length ?? 0) > 0 && !disabled) {
      event.preventDefault()
      event.stopPropagation()
      card.onSelect?.([])
    } else if (/^[1-9]$/.test(event.key) && !disabled) {
      const option = options[Number(event.key) - 1]
      if (option === undefined) return
      event.preventDefault()
      const selected = card.selected ?? []
      choose(card.mode !== 'any' ? [option.id] : selected.includes(option.id) ? selected.filter(id => id !== option.id) : [...selected, option.id])
      // Roving continues from the chosen row; focus is already inside this card, so nothing is taken from elsewhere.
      event.currentTarget.querySelectorAll<HTMLInputElement>('input')[Number(event.key) - 1]?.focus()
    }
  }
  const frame = {
    ref: card.focusRef as React.Ref<HTMLElement>, tabIndex: -1, 'aria-label': card.label, onKeyDownCapture: onKeyDown,
    // Pressing or clicking a choice confirms it, even the one already chosen for the person.
    onClickCapture: (event: React.MouseEvent<HTMLElement>) => { if (isChoice(event.target)) chosen.current = true },
    ...card.data,
  }
  if (card.density === 'row') {
    return <div {...stylex.props(styles.container)}>
      <section {...frame} {...stylex.props(styles.card, styles.cardRow)}><Row card={card} send={send} pager={pager} touch={touch} /></section>
    </div>
  }
  return <div {...stylex.props(styles.container)}>
    <section {...frame} style={card.maxBlockSize === undefined || full ? undefined : { maxBlockSize: card.maxBlockSize }} {...stylex.props(styles.card, full && styles.cardFull)}>
      <header {...stylex.props(styles.header, full && styles.headerFull)}>
        <span {...stylex.props(styles.meta)}>{card.meta.join(' · ')}</span>
        {stepper === undefined ? null : <Position noun="question" position={stepper} touch={touch} />}
        {pager === undefined ? null : <Position noun="alert" position={pager} touch={touch} />}
        {card.onFold === undefined ? null : <Aria.Button aria-label="Fold this decision" onPress={card.onFold} {...stylex.props(styles.pageButton, touch && styles.touchSquare)}><span {...stylex.props(styles.flipUp)}><Icon name="chevron-down" size={12} /></span></Aria.Button>}
      </header>
      <div {...stylex.props(styles.scroll, full && styles.scrollFull)}>
        <div role="heading" aria-level={2} {...stylex.props(styles.heading, full && styles.headingFull)}>{card.heading}</div>
        {full && card.contextFull !== undefined
          ? <div {...stylex.props(styles.contextFull)}>{card.contextFull}</div>
          : card.context === undefined || (open && !full && card.contextFull !== undefined) ? null : <p {...stylex.props(styles.context)}>{card.context}</p>}
        {full
          ? card.more === undefined ? null : <div {...stylex.props(styles.more)}>{card.more}</div>
          : !hasMore ? null : <>
            <Aria.Button aria-expanded={open} aria-controls={open ? panelId : undefined} onPress={() => setOpen(value => !value)} {...stylex.props(styles.moreButton, touch && styles.moreTouch)}>
              <span {...stylex.props(open && styles.turned)}><Icon name="chevron-right" size={12} /></span>
              More
            </Aria.Button>
            {open ? <div id={panelId} {...stylex.props(styles.more)}>{card.more ?? card.contextFull}</div> : null}
          </>}
        {card.body === undefined ? null : <div {...stylex.props(styles.body)}>{card.body}</div>}
        <Options card={card} choose={choose} disabled={disabled} touch={touch} />
      </div>
      {card.unavailable === undefined ? null : <p {...stylex.props(styles.unavailable, full && styles.flush)}>{card.unavailable}</p>}
      {card.status === undefined ? null : <p role={card.status.tone === 'error' ? 'alert' : 'status'} {...stylex.props(styles.status, card.status.tone === 'error' && styles.statusError, full && styles.flush)}>{card.status.text}</p>}
      {card.unavailable !== undefined || (card.words === undefined && card.dismiss === undefined && card.send === undefined) ? null : <div {...stylex.props(styles.quiet, full && styles.flush, full && styles.quietFull)}>
        {words === undefined ? <span {...stylex.props(styles.spacer)} /> : <WordsField words={words} disabled={disabled} touch={touch} />}
        <span {...stylex.props(styles.actions)}>
          {card.dismiss === undefined ? null : <Aria.Button isDisabled={disabled} onPress={card.dismiss.onPress} {...stylex.props(styles.dismiss, touch && styles.touchButton)}>{card.dismiss.label}</Aria.Button>}
          {card.send === undefined ? null : <Aria.Button isDisabled={disabled || card.send.disabled} aria-description={card.send.hint} onPress={send} {...stylex.props(styles.send, touch && styles.touchButton)}>{card.send.label}</Aria.Button>}
        </span>
      </div>}
    </section>
  </div>
}

const narrow = '@container decision-card (max-width: 480px)'
const focusRing = { ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}`, outlineOffset: g.focusOffset } }
const quietButton = { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: g.controlMd, paddingInline: s.row, borderRadius: r.sm, borderWidth: 0, fontFamily: t.fontSans, fontSize: t.metaSize, fontWeight: t.weightMedium, cursor: 'pointer', whiteSpace: 'nowrap', ...focusRing, ':disabled': { opacity: 0.45, cursor: 'default' } }

const styles = stylex.create({
  container: { width: '100%', minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column', containerType: 'inline-size', containerName: 'decision-card' },
  card: { display: 'flex', flexDirection: 'column', minHeight: 0, boxSizing: 'border-box', color: c.fg, backgroundColor: c.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.lg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, outline: 'none' },
  cardFull: { backgroundColor: 'transparent', borderWidth: 0, borderRadius: r.none },
  cardRow: { borderRadius: r.base },
  header: { flexShrink: 0, display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlMd, paddingBlockStart: s.md, paddingInline: s.lg },
  headerFull: { paddingInline: 0, paddingBlockStart: 0 },
  meta: { flexGrow: 1, minWidth: 0, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  position: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, color: c.fgSoft, fontSize: t.metaSize },
  positionText: { fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  pageButton: { ...quietButton, flexShrink: 0, width: g.controlSm, minHeight: g.controlSm, paddingInline: 0, backgroundColor: 'transparent', color: c.fgSoft, ':hover': { backgroundColor: c.rowHover, color: c.fg } },
  touchSquare: { width: g.touchTarget, minHeight: g.touchTarget },
  touchButton: { minHeight: g.touchTarget, paddingInline: s.xl },
  flip: { display: 'inline-flex', transform: 'rotate(180deg)' },
  flipUp: { display: 'inline-flex', transform: 'rotate(180deg)' },
  turned: { display: 'inline-flex', transform: 'rotate(90deg)' },
  // Only this part scrolls: a tall decision can never push Send out of reach.
  scroll: { flexGrow: 1, flexShrink: 1, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain', display: 'flex', flexDirection: 'column', gap: s.md, paddingBlock: s.sm, paddingInline: s.lg },
  scrollFull: { overflowY: 'visible', gap: s.lg, paddingInline: 0, paddingBlock: s.lg },
  heading: { flexShrink: 0, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightSemibold, overflowWrap: 'anywhere' },
  headingFull: { fontSize: g.dialogTitle, lineHeight: 1.35 },
  context: { flexShrink: 0, margin: 0, color: c.fgSoft, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  contextFull: { flexShrink: 0, color: c.fgSoft, overflowWrap: 'anywhere' },
  moreButton: { flexShrink: 0, display: 'inline-flex', alignSelf: 'flex-start', alignItems: 'center', gap: s.xs, minHeight: g.controlSm, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg }, ...focusRing },
  moreTouch: { minHeight: g.touchTarget },
  more: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.md, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  body: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  ref: { display: 'inline-flex', alignItems: 'center', gap: s.xs, maxWidth: '100%', minHeight: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: c.fg }, ...focusRing },
  refText: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  options: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.xs2, padding: s.xs, borderRadius: r.base, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border },
  optionHost: { display: 'block', position: 'relative', cursor: 'pointer', outline: 'none' },
  option: { display: 'flex', alignItems: 'flex-start', gap: s.row, paddingBlock: s.md, paddingInline: s.row, borderRadius: r.sm },
  optionTouch: { minHeight: g.touchTarget, boxSizing: 'border-box' },
  optionHover: { backgroundColor: c.rowHover },
  optionOn: { backgroundColor: c.rowActive },
  optionFocus: { outline: `${g.focusRing} solid ${c.primary}`, outlineOffset: `calc(-1 * ${g.focusRing})` },
  optionDisabled: { cursor: 'default' },
  key: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, marginBlockStart: s.xs2, borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, color: c.fgSoft, fontSize: t.denseSize, lineHeight: 1, fontVariantNumeric: 'tabular-nums' },
  keySquare: { color: c.onPrimary },
  keyOn: { backgroundColor: c.primary, borderColor: c.primary, color: c.onPrimary },
  optionText: { minWidth: 0, flexGrow: 1, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  optionTop: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.md, rowGap: s.xs2 },
  optionLabel: { fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  optionDanger: { color: c.dangerFg },
  badge: { flexShrink: 0, paddingInline: s.sm, borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.primary, color: c.runningFg, fontSize: t.denseSize, lineHeight: t.metaLeading },
  reason: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  rationale: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  unavailable: { flexShrink: 0, margin: 0, paddingBlock: s.sm, paddingInline: s.lg, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  status: { flexShrink: 0, margin: 0, paddingBlockStart: s.xs, paddingInline: s.lg, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  statusError: { color: c.dangerFg },
  // The quiet row: it never scrolls away, and neither of its buttons reads as an option.
  quiet: { flexShrink: 0, display: 'flex', flexWrap: { default: 'nowrap', [narrow]: 'wrap' }, alignItems: 'flex-end', justifyContent: 'flex-end', gap: s.md, paddingBlock: s.md, paddingInline: s.lg },
  flush: { paddingInline: 0 },
  // A long decision scrolls its page; its controls stay on screen, as they do docked.
  quietFull: { position: 'sticky', bottom: 0, backgroundColor: c.canvas },
  spacer: { flexGrow: 1 },
  field: { flexGrow: 1, flexShrink: 1, flexBasis: { default: 0, [narrow]: '100%' }, minWidth: 0, minHeight: g.controlMd, maxHeight: g.editorMax, boxSizing: 'border-box', resize: 'none', paddingBlock: s.xs, paddingInline: s.md, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, '::placeholder': { color: c.fgSoft }, ':focus-visible': { outline: 'none', borderColor: c.primary }, ':disabled': { opacity: 0.6 } },
  fieldTouch: { minHeight: g.touchTarget, fontSize: t.touchInputSize, lineHeight: t.uiLeading },
  actions: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.xs },
  dismiss: { ...quietButton, backgroundColor: 'transparent', color: c.fgSoft, ':hover': { backgroundColor: c.rowHover, color: c.fg } },
  send: { ...quietButton, backgroundColor: c.primary, color: c.onPrimary, ':hover': { backgroundColor: c.primaryHover } },
  row: { display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlLg, paddingBlock: s.xs, paddingInline: s.lg },
  rowTouch: { minHeight: g.touchTarget },
  rowText: { flexGrow: 1, minWidth: 0, display: 'flex', alignItems: 'baseline', gap: s.md, overflow: 'hidden' },
  rowHeading: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, fontWeight: t.weightMedium },
  rowChosen: { flexShrink: 0, display: { default: 'inline', [narrow]: 'none' }, maxWidth: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fgSoft, fontSize: t.metaSize },
  rowStatus: { flexShrink: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.fgSoft, fontSize: t.metaSize },
})
