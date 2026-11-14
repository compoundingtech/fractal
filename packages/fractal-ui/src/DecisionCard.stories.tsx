import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fireEvent, userEvent, within } from 'storybook/test'
import { DecisionCard, DecisionRef, type DecisionCardProps, type DecisionOption } from './assistant-ui/composition/DecisionCard'
import { NarrowLayoutContext } from './assistant-ui/composition/NarrowLayout'
import { darkTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const options: readonly DecisionOption[] = [
  { id: 'thread', label: 'Thread the session through', recommended: { reason: 'One call shape, type-checked.' }, rationale: 'Touches 14 call sites in packages/worker.' },
  { id: 'overload', label: 'Keep an overload', rationale: 'Adds a second signature to maintain.' },
  { id: 'hold', label: 'Stop the rename', danger: true, rationale: 'The mission waits until you restart it.' },
]

const more = <>
  <ul style={{ margin: 0, paddingInlineStart: '1.25rem' }}><li>The worker passes only the user id.</li><li>loadUser now needs the session.</li></ul>
  <span><DecisionRef value="resource/axe/decision/d5a79305a7d3a299fc5817adac076130231333e03ceb557afb9985e9ef144cfd/asxrpt" /></span>
</>

const base: DecisionCardProps = {
  density: 'docked',
  label: 'Worker typecheck fails after the rename',
  meta: ['example/atlas/builder', 'Request', 'waited 3h'],
  heading: 'Thread the session through syncAvatars, or keep a one-argument overload?',
  headingText: 'Thread the session through syncAvatars, or keep a one-argument overload?',
  context: 'syncAvatars calls loadUser without the session, so the worker no longer typechecks after the rename.',
  contextFull: <span>syncAvatars calls loadUser without the session, so the worker no longer typechecks after the rename. The public API shape changes either way, which is why this waits on you.</span>,
  more,
  options,
  optionsLabel: 'Answers',
}

/** A host's state: the card owns none of it. Recommended is preselected; sending is a separate act. */
function Live({ start = ['thread'], draft = '', placeholder = 'Or tell it something else…', onSent, ...card }: Partial<DecisionCardProps> & { readonly start?: readonly string[]; readonly draft?: string; readonly placeholder?: string; readonly onSent?: (answer: string) => void }) {
  const [selected, setSelected] = React.useState(start)
  const [words, setWords] = React.useState(draft)
  const [sent, setSent] = React.useState<string>()
  const answer = (value: string) => {
    setSent(value)
    onSent?.(value)
  }
  return <>
    <DecisionCard
      {...base}
      selected={selected}
      onSelect={setSelected}
      words={{ value: words, placeholder, label: 'Your answer to example/atlas/builder', onChange: setWords }}
      dismiss={{ label: 'Dismiss', onPress: () => answer('dismiss') }}
      send={{ label: 'Send', disabled: selected.length === 0 && words.trim() === '', onPress: () => answer(words.trim() || selected.join(',')) }}
      {...card}
    />
    <output data-testid="sent" hidden>{sent}</output>
  </>
}

const cutover: Partial<DecisionCardProps> = {
  label: 'Cache migration waits on a cutover',
  heading: 'Cut the build cache over to the content store now?',
  headingText: 'Cut the build cache over to the content store now?',
  context: 'Every build output already lives in the content store; the old cache is read only for misses.',
  options: [
    { id: 'now', label: 'Cut over now', recommended: { reason: 'Misses have been zero for a week.' }, rationale: 'Deletes the old cache.' },
    { id: 'later', label: 'Wait a release', rationale: 'Keeps both caches another week.' },
  ],
}

/** Two asks in a row, as the conversation dock shows them: answering one mounts the next, and focus follows into it. */
function Queue() {
  const [at, setAt] = React.useState(0)
  const [answers, setAnswers] = React.useState<readonly string[]>([])
  const card = React.useRef<HTMLElement>(null)
  const follows = React.useRef(false)
  React.useLayoutEffect(() => {
    if (!follows.current) return
    follows.current = false
    if (document.activeElement === null || document.activeElement === document.body) card.current?.focus()
  })
  const onSent = (answer: string) => {
    follows.current = card.current?.contains(document.activeElement) === true
    setAnswers(sent => [...sent, answer])
    setAt(index => index + 1)
  }
  return <>
    {at === 0 ? <Live key="rename" focusRef={card} onSent={onSent} /> : null}
    {at === 1 ? <Live key="cutover" {...cutover} start={['now']} focusRef={card} onSent={onSent} /> : null}
    <output data-testid="answers" hidden>{answers.join(' ')}</output>
  </>
}

type Args = { readonly scheme: Scheme; readonly width: number; readonly touch: boolean; readonly children?: React.ReactNode }
const Frame = ({ scheme, width, touch, children }: Args) => <NarrowLayoutContext.Provider value={touch}>
  <main data-scheme={scheme} {...stylex.props(styles.root, darkTheme, scheme === 'light' && lightTheme)}>
    <div style={{ width, maxWidth: '100%' }} {...stylex.props(styles.lane)}>{children}</div>
  </main>
</NarrowLayoutContext.Provider>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Decision Card',
  component: Frame,
  args: { scheme: 'dark', width: 736, touch: false },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, width: { control: { type: 'range', min: 280, max: 1000 } }, touch: { control: 'boolean' }, children: { table: { disable: true } } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'A presentational card over literal props; the alert adapter stories carry the domain data.' } },
} satisfies Meta<typeof Frame>
export default meta

type Story = StoryObj<typeof meta>

export const scenarioExemptions: readonly string[] = [
  'ChooseAny:primitive',
  'Compact:primitive',
  'Docked:primitive',
  'DockedLight:primitive',
  'DraftEnter:primitive',
  'Failed:primitive',
  'Full:primitive',
  'HeldEnter:primitive',
  'NarrowContainer:primitive',
  'Pager:primitive',
  'Phone:primitive',
  'ReadOnly:primitive',
  'Resolved:primitive',
  'Sending:primitive',
  'ShortRoom:primitive',
  'Stepper:primitive',
  'WordsOnly:primitive',
]

/** The focused card: one heading, one line of context, one group with each rationale in its row, a quiet row. */
export const Docked: Story = {
  args: { children: <Live /> },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('radio', { name: /Thread the session through/ })).toBeChecked()
    // Selecting answers nothing; sending is its own act.
    await userEvent.click(canvas.getByRole('radio', { name: /Keep an overload/ }))
    await expect(canvas.getByTestId('sent').textContent).toBe('')
    await userEvent.keyboard('1')
    await expect(canvas.getByRole('radio', { name: /Thread the session through/ })).toBeChecked()
    await userEvent.keyboard('{ArrowDown}')
    await expect(canvas.getByRole('radio', { name: /Keep an overload/ })).toBeChecked()
    await userEvent.keyboard('{Enter}')
    await expect(canvas.getByTestId('sent').textContent).toBe('overload')
  },
}
/**
 * Enter answers only a choice the person made on this question. The Enter that answers one ask, held or pressed again,
 * lands on the next card's preselected recommendation unread: it must answer nothing until a choice or Send.
 */
export const HeldEnter: Story = {
  args: { children: <Queue /> },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const answers = () => canvas.getByTestId('answers').textContent
    const repeat = (target: Element) => fireEvent.keyDown(target, { key: 'Enter', code: 'Enter', repeat: true })
    // The first ask, answered deliberately from the keyboard: its recommendation confirmed with its digit, then Enter.
    await userEvent.click(canvas.getByRole('radio', { name: /Keep an overload/ }))
    await userEvent.keyboard('1')
    await userEvent.keyboard('{Enter}')
    await expect(answers()).toBe('thread')
    const next = canvas.getByRole('region', { name: cutover.label })
    await expect(next).toHaveFocus()
    await expect(canvas.getByRole('radio', { name: /Cut over now/ })).toBeChecked()
    // The same Enter held: its repeats land on the next card and answer nothing.
    repeat(next)
    repeat(next)
    await expect(answers()).toBe('thread')
    // A fresh Enter on its frame, its preselected choice, or its empty field answers nothing either: nothing was chosen here.
    await userEvent.keyboard('{Enter}')
    canvas.getByRole('radio', { name: /Cut over now/ }).focus()
    await userEvent.keyboard('{Enter}')
    repeat(canvas.getByRole('radio', { name: /Cut over now/ }))
    canvas.getByRole('textbox').focus()
    await userEvent.keyboard('{Enter}')
    repeat(canvas.getByRole('textbox'))
    await expect(answers()).toBe('thread')
    // A held Enter on Send answers nothing either; a fresh press on Send answers what is chosen, recommendation included.
    const send = canvas.getByRole('button', { name: 'Send' })
    send.focus()
    repeat(send)
    repeat(send)
    await expect(answers()).toBe('thread')
    await userEvent.keyboard('{Enter}')
    await expect(answers()).toBe('thread now')
  },
}
/** Words the card arrived with are a draft, not an answer: Enter sends only once the person writes or chooses here. */
export const DraftEnter: Story = {
  args: { children: <Live draft="Thread it, then rerun the worker" /> },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const field = canvas.getByRole('textbox')
    field.focus()
    await userEvent.keyboard('{Enter}')
    await expect(canvas.getByTestId('sent').textContent).toBe('')
    await userEvent.type(field, ' typecheck')
    fireEvent.keyDown(field, { key: 'Enter', code: 'Enter', repeat: true })
    await expect(canvas.getByTestId('sent').textContent).toBe('')
    await userEvent.keyboard('{Enter}')
    await expect(canvas.getByTestId('sent').textContent).toBe('Thread it, then rerun the worker typecheck')
  },
}
export const DockedLight: Story = { args: { scheme: 'light', children: <Live /> } }
/** A body taller than the room scrolls; the question's controls stay in view. */
export const ShortRoom: Story = {
  args: { children: <Live maxBlockSize={240} /> },
  play: async ({ canvasElement }) => {
    const send = within(canvasElement).getByRole('button', { name: 'Send' })
    const card = within(canvasElement).getByRole('region', { name: base.label })
    await expect(send.getBoundingClientRect().bottom).toBeLessThanOrEqual(card.getBoundingClientRect().bottom)
    await expect(card.getBoundingClientRect().height).toBeLessThanOrEqual(240)
  },
}
/** The main-area details view: no frame, context and subjects in full. */
export const Full: Story = { args: { width: 736, children: <Live density="full" /> } }
/** Folded to one line for a short window: question, what is chosen, Send. */
export const Compact: Story = { args: { children: <Live density="row" onFold={() => undefined} /> } }
/** The record of an answer: controls gone at once. */
export const Resolved: Story = { args: { children: <DecisionCard {...base} density="row" resolved="You answered Thread the session through" status={{ tone: 'progress', text: 'Sending…' }} /> } }
export const Sending: Story = { args: { children: <Live busy status={{ tone: 'progress', text: 'Sending…' }} /> } }
/** The choice and the words stay; the reason shows where the person is looking. */
export const Failed: Story = { args: { children: <Live status={{ tone: 'error', text: 'Could not confirm answer. Request restored; check before sending again.' }} /> } }
/** Options stay readable and disabled, with why and where. */
export const ReadOnly: Story = {
  args: { children: <Live unavailable={<>This device is read-only, so it can't answer here. Answer it with <code>fractal alerts</code>.</>} /> },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('radio', { name: /Thread the session through/ })).toBeDisabled()
    await expect(canvas.queryByRole('button', { name: 'Send' })).toBeNull()
  },
}
export const WordsOnly: Story = { args: { children: <Live options={[]} start={[]} placeholder="Tell it what to do…" /> } }
export const Pager: Story = { args: { children: <Live pager={{ index: 0, count: 3, onPrevious: () => undefined, onNext: () => undefined }} /> } }
export const Stepper: Story = { args: { children: <Live stepper={{ index: 1, count: 3, onPrevious: () => undefined, onNext: () => undefined }} send={{ label: 'Next', disabled: false, onPress: () => undefined }} /> } }
export const ChooseAny: Story = { args: { children: <Live mode="any" start={['thread', 'hold']} /> } }
/** Stacking follows the card's container, not the viewport. */
export const NarrowContainer: Story = { args: { width: 320, children: <Live /> } }
export const Phone: Story = { args: { width: 360, touch: true, children: <Live /> } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', display: 'flex', justifyContent: 'center', alignItems: 'flex-start', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  lane: { display: 'flex', flexDirection: 'column', minWidth: 0, maxHeight: g.transcriptFrameHeight },
})
