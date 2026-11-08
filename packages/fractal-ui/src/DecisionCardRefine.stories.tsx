import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import type { DecisionCardProps, DecisionOption, DecisionPosition } from './assistant-ui/composition/DecisionCard'
import { Icon } from './assistant-ui/composition/Icons'
import { InlineMarkdown } from './assistant-ui/composition/Markdown'
import { markdownInline } from './assistant-ui/composition/markdown-text'
import { NarrowLayoutContext } from './assistant-ui/composition/NarrowLayout'
import { darkTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, geometryVars as g, interactionVars as i, radiusVars as r, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

/**
 * Three directions for refining the decision card, side by side over the same props as `DecisionCard`.
 * A story-only fork: nothing here ships until one is picked and folded back into `DecisionCard.tsx`.
 *
 * - A, Quiet list: one surface, hairline-separated rows, a real radio, a tinted selection, keycaps trailing.
 * - B, Tiles: each answer its own bordered tile; the sender carries the needs-you mark; the stepper shows progress.
 * - C, Focused rows: one line per answer; only the chosen answer opens its consequence.
 */
type Look = 'A' | 'B' | 'C'

const isField = (target: EventTarget): boolean => target instanceof HTMLTextAreaElement
const isChoice = (target: EventTarget): boolean => target instanceof HTMLInputElement
const isBare = (event: React.KeyboardEvent): boolean => !event.altKey && !event.ctrlKey && !event.metaKey
const md = (text: string) => <InlineMarkdown text={markdownInline(text)} />

/** Who asked, then what kind of ask and how long, quieter. The first meta fact is the sender. */
function Sender({ look, meta }: { readonly look: Look; readonly meta: readonly string[] }) {
  const [who, ...rest] = meta
  return <span {...stylex.props(styles.sender)}>
    {look === 'B' ? <span aria-hidden="true" {...stylex.props(styles.needsYou)} /> : null}
    <span {...stylex.props(styles.senderWho, look === 'C' && styles.senderChip)}>
      {look === 'C' ? <Icon name="person" size={12} /> : null}
      <span {...stylex.props(styles.ellipsis)}>{who}</span>
    </span>
    {rest.length === 0 ? null : <span {...stylex.props(styles.senderRest)}>{rest.join(' · ')}</span>}
  </span>
}

function Position({ noun, position, touch, progress }: { readonly noun: string; readonly position: DecisionPosition; readonly touch: boolean; readonly progress: boolean }) {
  const button = stylex.props(styles.pageButton, touch && styles.touchSquare)
  return <span role="group" aria-label={`${noun} ${position.index + 1} of ${position.count}`} {...stylex.props(styles.position)}>
    {progress
      ? <span aria-hidden="true" {...stylex.props(styles.segments)}>{Array.from({ length: position.count }, (_, index) => <span key={index} {...stylex.props(styles.segment, index <= position.index && styles.segmentOn)} />)}</span>
      : null}
    <Aria.Button aria-label={`Previous ${noun}`} isDisabled={position.index === 0} onPress={position.onPrevious} {...button}><span {...stylex.props(styles.flip)}><Icon name="chevron-right" size={14} /></span></Aria.Button>
    <span {...stylex.props(styles.positionText)}>{progress ? <span {...stylex.props(styles.wide)}>{`${noun[0]!.toUpperCase()}${noun.slice(1)} `}</span> : null}{position.index + 1} of {position.count}</span>
    <Aria.Button aria-label={`Next ${noun}`} isDisabled={position.index === position.count - 1} onPress={position.onNext} {...button}><Icon name="chevron-right" size={14} /></Aria.Button>
  </span>
}

function Indicator({ on, any }: { readonly on: boolean; readonly any: boolean }) {
  return <span aria-hidden="true" {...stylex.props(styles.indicator, any && styles.indicatorSquare, on && styles.indicatorOn)}>
    {on ? any ? <Icon name="check" size={12} /> : <span {...stylex.props(styles.dot)} /> : null}
  </span>
}

function OptionRow({ look, option, index, on, any, disabled, touch, last }: { readonly look: Look; readonly option: DecisionOption; readonly index: number; readonly on: boolean; readonly any: boolean; readonly disabled: boolean; readonly touch: boolean; readonly last: boolean }) {
  // C shows an answer's consequence only once it is the chosen one; A and B always do.
  const open = look !== 'C' || on
  const inner = (focused: boolean, hovered: boolean) => <span {...stylex.props(
    styles.option,
    look === 'A' && styles.optionA, look === 'A' && !last && styles.optionRule,
    look === 'B' && styles.optionB, look === 'B' && on && styles.optionBOn,
    look === 'C' && styles.optionC,
    touch && styles.optionTouch,
    hovered && !disabled && !on && styles.optionHover,
    on && look !== 'B' && styles.optionOn,
    focused && styles.optionFocus,
    disabled && styles.optionDisabled,
  )}>
    <Indicator on={on} any={any} />
    <span {...stylex.props(styles.optionText)}>
      <span {...stylex.props(styles.optionLabel, on && styles.optionLabelOn, option.danger === true && look === 'B' && styles.optionDanger)}>{md(option.label)}</span>
      {option.recommended === undefined ? null : <span {...stylex.props(styles.recommendLine)}>
        <span {...stylex.props(styles.badge, look === 'B' && styles.badgeSolid)}>Recommended</span>
        {open && option.recommended.reason !== undefined ? <span {...stylex.props(styles.reason)}>{md(option.recommended.reason)}</span> : null}
      </span>}
      {open && option.rationale !== undefined ? <span {...stylex.props(styles.rationale)}>{typeof option.rationale === 'string' ? md(option.rationale) : option.rationale}</span> : null}
    </span>
    {touch || index >= 9 ? null : <kbd aria-hidden="true" {...stylex.props(styles.key, on && styles.keyOn)}>{index + 1}</kbd>}
  </span>
  const host = stylex.props(styles.optionHost)
  return any
    ? <Aria.Checkbox value={option.id} isDisabled={disabled} {...host}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Checkbox>
    : <Aria.Radio value={option.id} isDisabled={disabled} {...host}>{state => inner(state.isFocusVisible, state.isHovered)}</Aria.Radio>
}

function Options({ look, card, disabled, touch }: { readonly look: Look; readonly card: DecisionCardProps; readonly disabled: boolean; readonly touch: boolean }) {
  const options = card.options ?? []
  if (options.length === 0) return null
  const selected = card.selected ?? []
  const any = card.mode === 'any'
  const rows = options.map((option, index) => <OptionRow key={option.id} look={look} option={option} index={index} on={selected.includes(option.id)} any={any} disabled={disabled} touch={touch} last={index === options.length - 1} />)
  const label = card.optionsLabel ?? 'Answers'
  const group = stylex.props(styles.options, look === 'A' && styles.optionsA, look === 'B' && styles.optionsB, look === 'C' && styles.optionsC)
  return any
    ? <Aria.CheckboxGroup aria-label={label} value={[...selected]} isDisabled={disabled} onChange={ids => card.onSelect?.(ids)} {...group}>{rows}</Aria.CheckboxGroup>
    : <Aria.RadioGroup aria-label={label} value={selected[0] ?? null} isDisabled={disabled} onChange={id => card.onSelect?.([id])} {...group}>{rows}</Aria.RadioGroup>
}

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

function RefineCard({ look, ...card }: DecisionCardProps & { readonly look: Look }): React.ReactNode {
  const touch = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  const panelId = React.useId()
  const full = card.density === 'full'
  const disabled = card.busy === true || card.unavailable !== undefined
  const options = card.options ?? []
  const hasMore = card.contextFull !== undefined || card.more !== undefined
  const setSelected = (ids: readonly string[]) => card.onSelect?.(ids)
  const canSend = card.send !== undefined && !card.send.disabled && !disabled
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing) return
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
    } else if (/^[1-9]$/.test(event.key) && !disabled) {
      const option = options[Number(event.key) - 1]
      if (option === undefined) return
      event.preventDefault()
      const selected = card.selected ?? []
      setSelected(card.mode !== 'any' ? [option.id] : selected.includes(option.id) ? selected.filter(id => id !== option.id) : [...selected, option.id])
      event.currentTarget.querySelectorAll<HTMLInputElement>('input')[Number(event.key) - 1]?.focus()
    }
  }
  const showQuiet = card.unavailable === undefined && (card.words !== undefined || card.dismiss !== undefined || card.send !== undefined)
  return <div {...stylex.props(styles.container)}>
    <section ref={card.focusRef as React.Ref<HTMLElement>} tabIndex={-1} aria-label={card.label} onKeyDownCapture={onKeyDown} {...card.data}
      style={card.maxBlockSize === undefined || full ? undefined : { maxBlockSize: card.maxBlockSize }}
      {...stylex.props(styles.card, look === 'B' && styles.cardB, full && styles.cardFull)}>
      <header {...stylex.props(styles.header, full && styles.flush, full && styles.headerFull)}>
        <Sender look={look} meta={card.meta} />
        {card.stepper === undefined ? null : <Position noun="question" position={card.stepper} touch={touch} progress={look !== 'A'} />}
        {card.pager === undefined ? null : <Position noun="alert" position={card.pager} touch={touch} progress={false} />}
        {card.onFold === undefined ? null : <Aria.Button aria-label="Fold this decision" onPress={card.onFold} {...stylex.props(styles.pageButton, touch && styles.touchSquare)}><span {...stylex.props(styles.flip)}><Icon name="chevron-down" size={14} /></span></Aria.Button>}
      </header>
      <div {...stylex.props(styles.scroll, full && styles.flush, full && styles.scrollFull)}>
        <div {...stylex.props(styles.question)}>
          <div role="heading" aria-level={2} {...stylex.props(styles.heading, full && styles.headingFull)}>{card.heading}</div>
          {full && card.contextFull !== undefined
            ? <div {...stylex.props(styles.context)}>{card.contextFull}</div>
            : card.context === undefined || (open && card.contextFull !== undefined && !full) ? null : <p {...stylex.props(styles.context)}>{md(card.context)}</p>}
          {full || !hasMore ? null : <Aria.Button aria-expanded={open} aria-controls={open ? panelId : undefined} onPress={() => setOpen(value => !value)} {...stylex.props(styles.moreButton, touch && styles.moreTouch)}>
            {open ? 'Less' : 'More'}
            <span {...stylex.props(styles.chevron, open && styles.chevronOpen)}><Icon name="chevron-down" size={12} /></span>
          </Aria.Button>}
          {!full && open ? <div id={panelId} {...stylex.props(styles.more)}>{card.more ?? card.contextFull}</div> : null}
          {full && card.more !== undefined ? <div {...stylex.props(styles.more)}>{card.more}</div> : null}
        </div>
        {card.body === undefined ? null : <div {...stylex.props(styles.body)}>{card.body}</div>}
        <Options look={look} card={card} disabled={disabled} touch={touch} />
      </div>
      {card.unavailable === undefined ? null : <p {...stylex.props(styles.note, full && styles.flush)}>{card.unavailable}</p>}
      {card.status === undefined ? null : <p role={card.status.tone === 'error' ? 'alert' : 'status'} {...stylex.props(styles.note, card.status.tone === 'error' && styles.noteError, full && styles.flush)}>
        {card.status.tone === 'error' ? <Icon name="alert" size={12} /> : null}{card.status.text}
      </p>}
      {!showQuiet ? null : <div {...stylex.props(styles.quiet, look !== 'C' && styles.quietRule, full && styles.flush, full && styles.quietFull)}>
        {card.words === undefined ? <span {...stylex.props(styles.spacer)} /> : <WordsField words={card.words} disabled={disabled} touch={touch} />}
        <span {...stylex.props(styles.actions)}>
          {card.dismiss === undefined ? null : <Aria.Button isDisabled={disabled} onPress={card.dismiss.onPress} {...stylex.props(styles.button, styles.dismiss, touch && styles.touchButton)}>{card.dismiss.label}</Aria.Button>}
          {card.send === undefined ? null : <Aria.Button isDisabled={disabled || card.send.disabled} aria-description={card.send.hint} onPress={card.send.onPress} {...stylex.props(styles.button, styles.send, touch && styles.touchButton)}>
            {card.send.label}
            {touch ? null : <kbd aria-hidden="true" {...stylex.props(styles.sendKey)}>↵</kbd>}
          </Aria.Button>}
        </span>
      </div>}
    </section>
  </div>
}

// --- stories -------------------------------------------------------------------------------------------------------

const options: readonly DecisionOption[] = [
  { id: 'thread', label: 'Thread the session through', recommended: { reason: 'One call shape, type-checked.' }, rationale: 'Touches 14 call sites in packages/worker.' },
  { id: 'overload', label: 'Keep an overload', rationale: 'Adds a second signature to maintain.' },
  { id: 'hold', label: 'Stop the rename', danger: true, rationale: 'The mission waits until you restart it.' },
]

const base: DecisionCardProps = {
  density: 'docked',
  label: 'Worker typecheck fails after the rename',
  meta: ['example/atlas/builder', 'Request', 'waited 3h'],
  heading: 'Thread the session through syncAvatars, or keep a one-argument overload?',
  headingText: 'Thread the session through syncAvatars, or keep a one-argument overload?',
  context: 'syncAvatars calls loadUser without the session, so the worker no longer typechecks after the rename.',
  contextFull: <span>syncAvatars calls loadUser without the session, so the worker no longer typechecks after the rename. The public API shape changes either way, which is why this waits on you.</span>,
  more: <ul style={{ margin: 0, paddingInlineStart: '1.25rem' }}><li>The worker passes only the user id.</li><li>loadUser now needs the session.</li></ul>,
  options,
  optionsLabel: 'Answers',
}

function Live({ look, start = ['thread'], ...card }: Partial<DecisionCardProps> & { readonly look: Look; readonly start?: readonly string[] }) {
  const [selected, setSelected] = React.useState(start)
  const [words, setWords] = React.useState('')
  const [sent, setSent] = React.useState<string>()
  return <>
    <RefineCard
      look={look}
      {...base}
      selected={selected}
      onSelect={setSelected}
      words={{ value: words, placeholder: 'Or tell it something else…', label: 'Your answer to example/atlas/builder', onChange: setWords }}
      dismiss={{ label: 'Dismiss', onPress: () => setSent('dismiss') }}
      send={{ label: 'Send', disabled: selected.length === 0 && words.trim() === '', onPress: () => setSent(words.trim() || selected.join(',')) }}
      {...card}
    />
    <output data-testid="sent" hidden>{sent}</output>
  </>
}

const position = (index: number, count: number): DecisionPosition => ({ index, count, onPrevious: () => undefined, onNext: () => undefined })

/** Every state worth comparing, stacked: plain, stepper, pager + choose-any, failed. */
function Sheet({ look }: { readonly look: Look }) {
  return <div {...stylex.props(styles.sheet)}>
    <Live look={look} />
    <Live look={look} stepper={position(1, 3)} send={{ label: 'Next', disabled: false, onPress: () => undefined }} />
    <Live look={look} mode="any" start={['thread', 'hold']} pager={position(1, 5)} onFold={() => undefined} />
    <Live look={look} status={{ tone: 'error', text: 'Could not confirm answer. Request restored; check before sending again.' }} />
  </div>
}

type Args = { readonly look: Look; readonly scheme: Scheme; readonly width: number; readonly touch: boolean }
const Frame = ({ look, scheme, width, touch }: Args) => <NarrowLayoutContext.Provider value={touch}>
  <main data-scheme={scheme} {...stylex.props(styles.root, darkTheme, scheme === 'light' && lightTheme)}>
    <div style={{ width, maxWidth: '100%' }} {...stylex.props(styles.lane)}><Sheet look={look} /></div>
  </main>
</NarrowLayoutContext.Provider>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Decision Card/Refine',
  component: Frame,
  args: { look: 'A', scheme: 'dark', width: 736, touch: false },
  argTypes: { look: { control: 'radio', options: ['A', 'B', 'C'] }, scheme: { control: 'radio', options: ['dark', 'light'] }, width: { control: { type: 'range', min: 280, max: 1000 } }, touch: { control: 'boolean' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'Design exploration over literal props; a picked direction folds back into DecisionCard.' } },
} satisfies Meta<typeof Frame>
export default meta
type Story = StoryObj<typeof meta>

export const scenarioExemptions: readonly string[] = [
  'AQuietList:primitive', 'AQuietListLight:primitive', 'AQuietListPhone:primitive',
  'BTiles:primitive', 'BTilesLight:primitive', 'BTilesPhone:primitive',
  'CFocusedRows:primitive', 'CFocusedRowsLight:primitive', 'CFocusedRowsPhone:primitive',
]

/** The same keyboard contract in every direction; the ring shows for keys only. */
const keyboard: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const card = canvas.getAllByRole('region', { name: base.label })[0]!
  const scope = within(card)
  await userEvent.click(scope.getByText('Keep an overload'))
  await expect(scope.getByRole('radio', { name: /Keep an overload/ })).toBeChecked()
  await expect(canvas.getAllByTestId('sent')[0]!.textContent).toBe('')
  await userEvent.keyboard('1')
  await expect(scope.getByRole('radio', { name: /Thread the session through/ })).toBeChecked()
  await userEvent.keyboard('{ArrowDown}')
  await expect(scope.getByRole('radio', { name: /Keep an overload/ })).toBeChecked()
}

export const AQuietList: Story = { name: 'A · Quiet list', args: { look: 'A' }, play: keyboard }
export const AQuietListLight: Story = { name: 'A · Quiet list (light)', args: { look: 'A', scheme: 'light' } }
export const AQuietListPhone: Story = { name: 'A · Quiet list (phone)', args: { look: 'A', width: 408, touch: true } }
export const BTiles: Story = { name: 'B · Tiles', args: { look: 'B' }, play: keyboard }
export const BTilesLight: Story = { name: 'B · Tiles (light)', args: { look: 'B', scheme: 'light' } }
export const BTilesPhone: Story = { name: 'B · Tiles (phone)', args: { look: 'B', width: 408, touch: true } }
export const CFocusedRows: Story = { name: 'C · Focused rows', args: { look: 'C' }, play: keyboard }
export const CFocusedRowsLight: Story = { name: 'C · Focused rows (light)', args: { look: 'C', scheme: 'light' } }
export const CFocusedRowsPhone: Story = { name: 'C · Focused rows (phone)', args: { look: 'C', width: 408, touch: true } }

// --- styles --------------------------------------------------------------------------------------------------------

const narrow = '@container refine-card (max-width: 480px)'
const selectedWash = `color-mix(in srgb, ${c.primary} 9%, transparent)`
const selectedEdge = `color-mix(in srgb, ${c.primary} 55%, transparent)`
const focusRing = { ':focus-visible': { outline: `${g.focusRing} solid ${i.focusRing}`, outlineOffset: g.focusOffset } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', display: 'flex', justifyContent: 'center', alignItems: 'flex-start', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  lane: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  sheet: { display: 'flex', flexDirection: 'column', gap: s.section },

  container: { width: '100%', minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column', containerType: 'inline-size', containerName: 'refine-card' },
  card: { display: 'flex', flexDirection: 'column', minHeight: 0, boxSizing: 'border-box', color: c.fg, backgroundColor: c.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.lg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, outline: 'none', overflow: 'hidden' },
  cardB: { backgroundColor: c.canvas },
  cardFull: { backgroundColor: 'transparent', borderWidth: 0, borderRadius: r.none, overflow: 'visible' },
  flush: { paddingInline: 0 },

  // Header: the sender is the first thing read and the last thing truncated.
  header: { flexShrink: 0, display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlMd, paddingBlockStart: s.md, paddingInline: s.lg },
  headerFull: { paddingBlockStart: 0 },
  sender: { flexGrow: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading },
  needsYou: { flexShrink: 0, width: '6px', height: '6px', borderRadius: r.full, backgroundColor: c.attention },
  senderWho: { minWidth: 0, maxWidth: '100%', flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.xs, color: c.fg, fontWeight: t.weightMedium },
  senderChip: { paddingInline: s.sm, minHeight: g.controlSm, boxSizing: 'border-box', borderRadius: r.full, backgroundColor: c.controlFill, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, color: c.fgSoft },
  senderRest: { flexShrink: 100, minWidth: 0, color: c.fgSoft, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  ellipsis: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },

  position: { display: 'inline-flex', alignItems: 'center', gap: s.xs2, flexShrink: 0, color: c.fgSoft, fontSize: t.metaSize },
  positionText: { fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', paddingInline: s.xs2 },
  segments: { display: { default: 'inline-flex', [narrow]: 'none' }, gap: s.xs2, marginInlineEnd: s.sm },
  wide: { display: { default: 'inline', [narrow]: 'none' } },
  segment: { width: '14px', height: '3px', borderRadius: r.full, backgroundColor: c.border },
  segmentOn: { backgroundColor: c.fgSoft },
  pageButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.controlSm, height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':disabled': { opacity: 0.35, cursor: 'default', backgroundColor: 'transparent' }, ...focusRing },
  touchSquare: { width: g.touchTarget, height: g.touchTarget },
  flip: { display: 'inline-flex', transform: 'rotate(180deg)' },

  scroll: { flexGrow: 1, flexShrink: 1, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain', display: 'flex', flexDirection: 'column', gap: s.lg, paddingBlockStart: s.sm, paddingBlockEnd: s.lg, paddingInline: s.lg },
  scrollFull: { overflowY: 'visible', gap: s.xl },
  // The question block: heading, its context, and More belong together, tighter than the gap to the answers.
  question: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.xs },
  heading: { fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightSemibold, overflowWrap: 'anywhere', textWrap: 'pretty' },
  headingFull: { fontSize: g.dialogTitle, lineHeight: 1.35 },
  context: { margin: 0, color: c.fgSoft, overflowWrap: 'anywhere', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' },
  moreButton: { display: 'inline-flex', alignSelf: 'flex-start', alignItems: 'center', gap: s.xs, minHeight: g.controlSm, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg }, ...focusRing },
  moreTouch: { minHeight: g.touchTarget },
  chevron: { display: 'inline-flex', transitionProperty: 'transform', transitionDuration: '120ms' },
  chevronOpen: { transform: 'rotate(180deg)' },
  more: { display: 'flex', flexDirection: 'column', gap: s.md, paddingBlockStart: s.xs, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  body: { flexShrink: 0, display: 'flex', flexDirection: 'column', gap: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading },

  // Options
  options: { flexShrink: 0, display: 'flex', flexDirection: 'column' },
  optionsA: { marginInline: `calc(-1 * ${s.sm})` },
  optionsB: { gap: s.sm },
  optionsC: { gap: s.xs2, marginInline: `calc(-1 * ${s.sm})` },
  optionHost: { display: 'block', position: 'relative', cursor: 'pointer', outline: 'none' },
  option: { position: 'relative', display: 'flex', alignItems: 'flex-start', gap: s.row, borderRadius: r.base, transitionProperty: 'background-color, border-color', transitionDuration: '80ms' },
  optionA: { paddingBlock: s.md, paddingInline: s.sm },
  // A hairline between rows that a tinted row covers, so selection reads as one shape.
  optionRule: { '::after': { content: '""', position: 'absolute', insetInline: s.sm, bottom: 0, height: g.hairline, backgroundColor: c.border } },
  optionB: { paddingBlock: s.md, paddingInline: s.row, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, backgroundColor: c.raised },
  optionBOn: { borderColor: selectedEdge, backgroundColor: selectedWash },
  optionC: { alignItems: 'flex-start', paddingBlock: s.sm, paddingInline: s.sm },
  optionTouch: { minHeight: g.touchTarget, boxSizing: 'border-box', paddingBlock: s.row },
  optionHover: { backgroundColor: c.rowHover },
  optionOn: { backgroundColor: selectedWash, '::after': { opacity: 0 } },
  optionFocus: { outline: `${g.focusRing} solid ${i.focusRing}`, outlineOffset: `calc(-1 * ${g.focusRing})` },
  optionDisabled: { cursor: 'default', opacity: 0.6 },
  indicator: { flexShrink: 0, boxSizing: 'border-box', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: '16px', height: '16px', marginBlockStart: '2px', borderRadius: r.full, borderWidth: '1.5px', borderStyle: 'solid', borderColor: c.fgSoft, color: c.onPrimary },
  indicatorSquare: { borderRadius: r.keyboard },
  indicatorOn: { borderColor: c.primary, backgroundColor: c.primary },
  dot: { width: '6px', height: '6px', borderRadius: r.full, backgroundColor: c.onPrimary },
  optionText: { minWidth: 0, flexGrow: 1, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  recommendLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.sm, rowGap: s.xs2 },
  optionDanger: { color: c.dangerFg },
  optionLabel: { fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  optionLabelOn: { color: c.fg },
  badge: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', paddingInline: s.sm, borderRadius: r.full, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, color: c.fgSoft, fontSize: t.denseSize, lineHeight: '18px', fontWeight: t.weightMedium },
  badgeSolid: { backgroundColor: c.rowActive, borderColor: 'transparent' },
  // The reason is why it is recommended; the rationale is what happens. Different voices, not two equal grey lines.
  reason: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  rationale: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  key: { flexShrink: 0, marginBlockStart: '1px', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minWidth: '18px', height: '18px', paddingInline: s.xs2, boxSizing: 'border-box', borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: 1, fontVariantNumeric: 'tabular-nums' },
  keyOn: { color: c.fgSoft, borderColor: c.borderStrong },

  // Footer
  note: { flexShrink: 0, display: 'flex', alignItems: 'center', gap: s.sm, margin: 0, paddingBlockEnd: s.sm, paddingInline: s.lg, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  noteError: { color: c.dangerFg },
  quiet: { flexShrink: 0, display: 'flex', flexWrap: { default: 'nowrap', [narrow]: 'wrap' }, alignItems: 'flex-end', justifyContent: 'flex-end', gap: s.sm, paddingBlock: s.md, paddingInline: s.lg },
  quietRule: { borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border, backgroundColor: c.washSubtle },
  quietFull: { position: 'sticky', bottom: 0, backgroundColor: c.canvas },
  spacer: { flexGrow: 1 },
  field: { flexGrow: 1, flexShrink: 1, flexBasis: { default: 0, [narrow]: '100%' }, minWidth: 0, minHeight: g.controlMd, maxHeight: g.editorMax, boxSizing: 'border-box', resize: 'none', paddingBlock: s.xs, paddingInline: s.md, borderRadius: r.base, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, backgroundColor: c.controlFill, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, outlineStyle: 'none', '::placeholder': { color: c.fgSoft }, ':focus': { borderColor: c.focusBorder }, ':disabled': { opacity: 0.6 } },
  fieldTouch: { minHeight: g.touchTarget, fontSize: t.touchInputSize, lineHeight: t.uiLeading },
  actions: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.sm },
  button: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: s.sm, minHeight: g.controlMd, paddingInline: s.row, borderRadius: r.base, borderWidth: 0, fontFamily: t.fontSans, fontSize: t.metaSize, fontWeight: t.weightMedium, cursor: 'pointer', whiteSpace: 'nowrap', ...focusRing },
  dismiss: { backgroundColor: 'transparent', color: c.fgSoft, ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':disabled': { opacity: 0.45, cursor: 'default' } },
  // Disabled Send is a neutral control, not a dimmed primary that still looks live.
  send: { backgroundColor: c.primary, color: c.onPrimary, ':hover': { backgroundColor: c.primaryHover }, ':disabled': { backgroundColor: c.rowActive, color: c.fgSoft, cursor: 'default' } },
  sendKey: { fontFamily: t.fontSans, fontSize: t.denseSize, opacity: 0.7 },
  touchButton: { minHeight: g.touchTarget, paddingInline: s.xl },
})
