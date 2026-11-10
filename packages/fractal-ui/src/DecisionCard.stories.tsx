import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
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
function Live({ start = ['thread'], placeholder = 'Or tell it something else…', ...card }: Partial<DecisionCardProps> & { readonly start?: readonly string[]; readonly placeholder?: string }) {
  const [selected, setSelected] = React.useState(start)
  const [words, setWords] = React.useState('')
  const [sent, setSent] = React.useState<string>()
  return <>
    <DecisionCard
      {...base}
      selected={selected}
      onSelect={setSelected}
      words={{ value: words, placeholder, label: 'Your answer to example/atlas/builder', onChange: setWords }}
      dismiss={{ label: 'Dismiss', onPress: () => setSent('dismiss') }}
      send={{ label: 'Send', disabled: selected.length === 0 && words.trim() === '', onPress: () => setSent(words.trim() || selected.join(',')) }}
      {...card}
    />
    <output data-testid="sent" hidden>{sent}</output>
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
  'Failed:primitive',
  'Full:primitive',
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
