/**
 * Question form exploration: an interactive kit QuestionForm with omp ask tool parity.
 *
 * Variant dimensions:
 * - Scenario (S1–S3): one question, four tabbed questions, multi-select with Other and a note.
 * - State (P/S/E): pending, submitted, expired.
 * - Width (W440/W1440): the host window width; the form sits in the reading lane and stacks below 600 px of its own width.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { ToggleButton } from 'react-aria-components'
import { QuestionForm, OTHER_LABEL } from './assistant-ui/questions/QuestionForm'
import { questionScenarios, questionStateFor, submittedAt, type QuestionPhase, type QuestionScenario } from './assistant-ui/questions/fixtures'
import type { Answers, QuestionRequestView } from './assistant-ui/questions/model'
import { Icon } from './assistant-ui/composition/Icons'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as text, borderVars as border, accentVars as accent, geometryVars as g, geometryNumbers, radiusVars as r, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

type ScenarioId = 'S1' | 'S2' | 'S3'
type StateId = 'P' | 'S' | 'E'
type WidthId = 'W440' | 'W1440'
interface Variant<T extends string> { readonly id: T; readonly description: string }
const SCENARIOS: readonly Variant<ScenarioId>[] = [
  { id: 'S1', description: 'One question: radios, Recommended badge, markdown preview, automatic Other row' },
  { id: 'S2', description: 'Four tabbed questions: header chips with answered markers, notes, single, multi and free-text answers' },
  { id: 'S3', description: 'Multi-select with Other text and a note' },
]
const STATES: readonly Variant<StateId>[] = [
  { id: 'P', description: 'Pending: interactive; Enter submits or moves to the next unanswered question' },
  { id: 'S', description: 'Submitted: read-only summary of the sent answers and notes' },
  { id: 'E', description: 'Expired: the automatic answers the agent continued with' },
]
const WIDTHS: readonly Variant<WidthId>[] = [
  { id: 'W440', description: '440 px window: tabs stack and previews move below the options' },
  { id: 'W1440', description: '1440 px window: reading lane, tabs in one row, previews beside the options' },
]
const SCENARIO: Record<ScenarioId, QuestionScenario> = { S1: 'one', S2: 'tabbed', S3: 'multi' }
const PHASE: Record<StateId, QuestionPhase> = { P: 'pending', S: 'submitted', E: 'expired' }

/** Colocated variant picker: ID buttons plus the current variant's description. */
function VariantPicker<T extends string>({ label, variants, value, onChange }: { label: string; variants: readonly Variant<T>[]; value: T; onChange: (value: T) => void }) {
  const current = variants.find(variant => variant.id === value)
  return <div role="group" aria-label={label} {...stylex.props(styles.picker)}>
    <span {...stylex.props(styles.pickerLabel)}>{label}</span>
    <span {...stylex.props(styles.pickerButtons)}>
      {variants.map(variant => <ToggleButton key={variant.id} isSelected={variant.id === value} onChange={() => onChange(variant.id)} className={({ isSelected, isFocusVisible }) => stylex.props(styles.pickerButton, isSelected && styles.pickerButtonSelected, isFocusVisible && styles.focusRing).className ?? ''}>{variant.id}</ToggleButton>)}
    </span>
    {current !== undefined && <span data-picker-description="" {...stylex.props(styles.pickerDescription)}><Icon name="chevrons-left" size={12} />{current.id}: {current.description}</span>}
  </div>
}

/** The host side of the contract: it owns the request state and records the submitted payload. */
function QuestionHost({ scenario, state, onSubmitted }: { scenario: ScenarioId; state: StateId; onSubmitted: (answers: Answers) => void }) {
  const key = SCENARIO[scenario]
  const [request, setRequest] = React.useState<QuestionRequestView>(() => ({ ...questionScenarios[key], state: questionStateFor(key, PHASE[state]) }))
  const submit = (answers: Answers) => {
    onSubmitted(answers)
    setRequest(current => ({ ...current, state: { _tag: 'answered', answers, by: 'You', at: submittedAt } }))
  }
  return <QuestionForm request={request} variant="transcript" autoFocus onSubmit={submit} onOpenTerminal={scenario === 'S2' ? () => {} : undefined} />
}

interface StoryArgs { scheme: Scheme; scenario: ScenarioId; state: StateId; width: WidthId; explore: boolean; onSubmitted: (answers: Answers) => void }
function QuestionStory({ scheme, scenario: initialScenario, state: initialState, width: initialWidth, explore, onSubmitted }: StoryArgs) {
  const [scenario, setScenario] = React.useState(initialScenario)
  const [state, setState] = React.useState(initialState)
  const [width, setWidth] = React.useState(initialWidth)
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {explore && <section aria-label="Variant controls" {...stylex.props(styles.controls)}>
      <VariantPicker label="Scenario" variants={SCENARIOS} value={scenario} onChange={setScenario} />
      <VariantPicker label="State" variants={STATES} value={state} onChange={setState} />
      <VariantPicker label="Width" variants={WIDTHS} value={width} onChange={setWidth} />
    </section>}
    <div data-testid="question-window" {...stylex.props(styles.window, width === 'W440' ? styles.narrow : styles.wide)}>
      <div data-testid="question-lane" {...stylex.props(styles.lane)}>
        <QuestionHost key={`${scenario}-${state}`} scenario={scenario} state={state} onSubmitted={onSubmitted} />
      </div>
    </div>
  </main>
}

const meta = {
  title: 'Fractal UI/Explore/Question form',
  component: QuestionStory,
  // The pickers keep their own state; keying on the args lets the Controls panel reset them.
  render: args => <QuestionStory key={`${args.scenario}-${args.state}-${args.width}`} {...args} />,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', scenario: 'S2', state: 'P', width: 'W1440', explore: false, onSubmitted: fn() },
  argTypes: {
    scheme: { options: ['dark', 'light'], control: 'radio' },
    scenario: { options: SCENARIOS.map(variant => variant.id), control: 'radio' },
    state: { options: STATES.map(variant => variant.id), control: 'radio' },
    width: { options: WIDTHS.map(variant => variant.id), control: 'radio' },
  },
} satisfies Meta<typeof QuestionStory>
export default meta
type Story = StoryObj<typeof meta>

const formIn = (root: HTMLElement) => {
  const form = root.querySelector<HTMLElement>('[data-question-form]')
  if (form === null) throw new Error('No question form rendered')
  return form
}
/** Native tooltips come from non-empty titles; React Aria's hidden inputs carry `title=""` to suppress them. */
const NATIVE_TITLE = '[title]:not([title=""])'
/** Every surface: no native title tooltips and no placeholder "Unknown" copy. The detector is checked against planted titles first. */
const assertHygiene = async (root: HTMLElement) => {
  const planted = document.createElement('div')
  planted.innerHTML = '<span title="planted"></span><input title="">'
  await expect(planted.querySelectorAll(NATIVE_TITLE)).toHaveLength(1)
  await expect(root.querySelectorAll(NATIVE_TITLE)).toHaveLength(0)
  await expect(root.textContent ?? '').not.toMatch(/unknown/i)
}
type Rect = Pick<DOMRect, 'top' | 'left' | 'bottom' | 'right'>
/** Tab strip layout from tab rectangles: one shared row, a single left-aligned column, or neither. */
const stripLayout = (rects: readonly Rect[]) => {
  const tops = new Set(rects.map(rect => Math.round(rect.top)))
  const lefts = new Set(rects.map(rect => Math.round(rect.left)))
  if (tops.size === 1) return 'row'
  return tops.size === rects.length && lefts.size === 1 ? 'stacked' : 'mixed'
}
const stacked = (form: HTMLElement) => form.getBoundingClientRect().width < geometryNumbers.questionStack
/** Forces the lane to just below or exactly at the stacking threshold, then restores it. */
const withLaneWidth = async (root: HTMLElement, narrow: boolean, check: (form: HTMLElement) => Promise<void>) => {
  const lane = within(root).getByTestId('question-lane')
  const form = formIn(root)
  const before = form.dataset['stacked']
  lane.style.width = `${geometryNumbers.questionStack - (narrow ? 1 : 0)}px`
  try {
    await waitFor(() => expect(form.dataset['stacked']).toBe(String(narrow)))
    await check(form)
  } finally {
    lane.style.width = ''
  }
  await waitFor(() => expect(form.dataset['stacked']).toBe(before))
}
const assertStrip = async (form: HTMLElement) => {
  const rects = within(form).getAllByRole('tab').map(tab => tab.getBoundingClientRect())
  await expect(stripLayout(rects)).toBe(stacked(form) ? 'stacked' : 'row')
  await expect(within(form).getByRole('tablist')).toHaveAttribute('aria-orientation', stacked(form) ? 'vertical' : 'horizontal')
}
const assertPreviewPlacement = async (form: HTMLElement) => {
  const options = form.querySelector<HTMLElement>('[data-question-stop="options"]')!.getBoundingClientRect()
  const preview = form.querySelector<HTMLElement>('[data-question-preview]')!.getBoundingClientRect()
  const beside = preview.left >= options.right && preview.top < options.bottom
  const below = preview.top >= options.bottom
  await expect(beside).toBe(!stacked(form))
  await expect(below).toBe(stacked(form))
}
const markers = (form: HTMLElement) => [...form.querySelectorAll('[data-question-tab-marker]')].map(marker => marker.getAttribute('data-question-tab-marker'))

const oneQuestion: Story = {
  args: { scenario: 'S1' },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const form = formIn(canvasElement)
    await assertHygiene(canvasElement)
    await expect(canvas.queryAllByRole('tab')).toHaveLength(0)
    const radios = canvas.getAllByRole('radio')
    await expect(radios).toHaveLength(4)
    await expect(radios[3]).toHaveAccessibleName(OTHER_LABEL)
    await expect(within(form).getAllByText('Recommended')).toHaveLength(1)
    await expect(radios[0]).toHaveAccessibleName(/Recommended/)
    await waitFor(() => expect(radios[0]).toHaveFocus())
    await expect(form.querySelector('[data-question-preview]')).toHaveAttribute('data-question-preview', 'embedded')
    await assertPreviewPlacement(form)
    await withLaneWidth(canvasElement, !stacked(form), assertPreviewPlacement)

    // Negative control: Enter with nothing chosen neither picks the focused row, submits nor leaves the pending state.
    await userEvent.keyboard('{Enter}')
    await expect(args.onSubmitted).not.toHaveBeenCalled()
    await expect(form.dataset['state']).toBe('pending')
    await expect(radios.filter(radio => (radio as HTMLInputElement).checked)).toHaveLength(0)
    await expect(radios[0]).toHaveFocus()
    await expect(canvas.getByRole('button', { name: 'Submit' })).toBeDisabled()

    await userEvent.keyboard('2')
    await waitFor(() => expect(radios[1]).toBeChecked())
    await expect(radios[1]).toHaveFocus()
    await expect(form.querySelector('[data-question-preview]')).toHaveAttribute('data-question-preview', 'managed')
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(radios[2]).toBeChecked())
    await expect(radios[1]).not.toBeChecked()
    await expect(within(form).getByText('No preview for this option.')).toBeVisible()

    await userEvent.keyboard('4')
    const other = await canvas.findByRole('textbox', { name: /Other answer/ })
    await waitFor(() => expect(other).toHaveFocus())
    await expect(radios[3]).toBeChecked()
    // Negative control: digits typed into the Other field are text, not option picks.
    await userEvent.keyboard('Shared file 1')
    await expect(other).toHaveValue('Shared file 1')
    await expect(radios[0]).not.toBeChecked()
    await expect(radios[3]).toBeChecked()

    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(form.dataset['state']).toBe('answered'))
    await expect(args.onSubmitted).toHaveBeenCalledTimes(1)
    await expect(args.onSubmitted).toHaveBeenCalledWith({ storage: { selected: [], other: 'Shared file 1' } })
    await expect(within(form).getByText('Other: Shared file 1')).toBeVisible()
    await expect(within(form).queryAllByRole('radio')).toHaveLength(0)
    await expect(form.querySelector('[data-question-state-chip]')).toHaveTextContent('Submitted')
    await assertHygiene(canvasElement)
  },
}

const fourTabbed: Story = {
  args: { scenario: 'S2' },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const form = formIn(canvasElement)
    await assertHygiene(canvasElement)
    const tabs = canvas.getAllByRole('tab')
    await expect(tabs.map(tab => tab.textContent)).toEqual(['Storage', 'Features', 'Deploy', 'Review'])
    await expect(markers(form)).toEqual(['open', 'open', 'open', 'open'])
    await assertStrip(form)
    await withLaneWidth(canvasElement, !stacked(form), assertStrip)
    // Negative control: the strip detector rejects a two-row strip as either layout.
    await expect(stripLayout([{ top: 0, left: 0, bottom: 1, right: 1 }, { top: 0, left: 9, bottom: 1, right: 10 }, { top: 9, left: 0, bottom: 10, right: 1 }])).toBe('mixed')

    const storage = canvas.getAllByRole('radio')
    await waitFor(() => expect(storage[0]).toHaveFocus())

    // RAC ignores hover in keyboard modality, so start the pointer check with a click on the plain title.
    await userEvent.click(within(form).getByText('Sample app setup'))
    const terminal = canvas.getByRole('button', { name: "Open the agent's terminal" })
    await userEvent.hover(terminal)
    await expect(await within(canvasElement.ownerDocument.body).findByRole('tooltip')).toHaveTextContent("Open the agent's terminal")
    await userEvent.unhover(terminal)
    await waitFor(() => expect(within(canvasElement.ownerDocument.body).queryByRole('tooltip')).toBeNull())
    storage[0]!.focus()
    await waitFor(() => expect(storage[0]).toHaveFocus())
    await userEvent.keyboard('1')
    await waitFor(() => expect(markers(form)).toEqual(['answered', 'open', 'open', 'open']))
    await expect(tabs[0]).toHaveAccessibleName('Storage, answered')
    await expect(tabs[1]).toHaveAccessibleName('Features')
    await expect(form).toHaveTextContent('1 of 4 answered')

    // Negative control: Enter on an incomplete form moves to the next unanswered question instead of submitting.
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(canvas.getByRole('tab', { name: 'Features' })).toHaveAttribute('aria-selected', 'true'))
    await expect(args.onSubmitted).not.toHaveBeenCalled()
    await expect(form.dataset['state']).toBe('pending')
    const features = canvas.getAllByRole('checkbox')
    await waitFor(() => expect(features[0]).toHaveFocus())
    await userEvent.keyboard('1')
    await waitFor(() => expect(features[0]).toBeChecked())
    await userEvent.keyboard('{ArrowDown}{ArrowDown}')
    await expect(features[2]).toHaveFocus()
    // Negative control: arrows move focus without toggling.
    await expect(features[1]).not.toBeChecked()
    await expect(features[2]).not.toBeChecked()
    await userEvent.keyboard(' ')
    await waitFor(() => expect(features[2]).toBeChecked())

    await userEvent.keyboard('{Tab}')
    await waitFor(() => expect(canvas.getByRole('tab', { name: 'Deploy' })).toHaveAttribute('aria-selected', 'true'))
    const deploy = form.querySelector<HTMLElement>('[data-question-panel="deploy"]')!
    const host = within(deploy).getByRole('textbox', { name: 'Where should the sample deploy?' })
    await waitFor(() => expect(host).toHaveFocus())
    await userEvent.keyboard('Edge workers')
    await expect(host).toHaveValue('Edge workers')
    // Negative control: a text question's own field holds the answer; typing never opens a second Other field.
    await expect(within(deploy).getAllByRole('textbox')).toHaveLength(1)
    await expect(within(deploy).queryAllByRole('radio')).toHaveLength(0)
    await userEvent.keyboard('{Tab}')
    await waitFor(() => expect(canvas.getByRole('tab', { name: 'Review' })).toHaveAttribute('aria-selected', 'true'))
    await userEvent.keyboard('1')
    await waitFor(() => expect(canvas.getAllByRole('radio')[0]).toBeChecked())
    await userEvent.keyboard('{Tab}')
    const note = canvas.getByRole('textbox', { name: 'Note' })
    await expect(note).toHaveFocus()
    await userEvent.keyboard('  Ship behind a flag  ')
    await expect(markers(form)).toEqual(['answered', 'answered', 'answered', 'answered'])
    await expect(canvas.getByRole('button', { name: 'Submit' })).toBeEnabled()

    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(form.dataset['state']).toBe('answered'))
    await expect(args.onSubmitted).toHaveBeenCalledTimes(1)
    await expect(args.onSubmitted).toHaveBeenCalledWith({
      storage: { selected: ['embedded'] },
      features: { selected: ['sign-in', 'export'] },
      deploy: { selected: [], other: 'Edge workers' },
      review: { selected: ['draft'], note: 'Ship behind a flag' },
    })
    await expect(form.querySelectorAll('[data-question-summary]')).toHaveLength(4)
    await expect(form.querySelector('[data-question-summary="features"]')).toHaveTextContent('Sign in, Export')
    // A text answer reads as itself, not as an "Other:" choice.
    await expect(form.querySelector('[data-question-summary="deploy"] dd')).toHaveTextContent(/^Edge workers$/)
    await expect(form.querySelector('[data-question-summary="review"]')).toHaveTextContent('Note: Ship behind a flag')
    await expect(canvas.queryAllByRole('tab')).toHaveLength(0)
    await assertHygiene(canvasElement)
  },
}

const multiOtherNote: Story = {
  args: { scenario: 'S3' },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const form = formIn(canvasElement)
    await assertHygiene(canvasElement)
    const boxes = canvas.getAllByRole('checkbox')
    await expect(boxes).toHaveLength(5)
    await expect(boxes[4]).toHaveAccessibleName(OTHER_LABEL)
    await expect(boxes[0]).toHaveAccessibleName(/Recommended/)
    await waitFor(() => expect(boxes[0]).toHaveFocus())
    await userEvent.keyboard('{ArrowUp}')
    await expect(boxes[4]).toHaveFocus()
    // Negative control: wrapping onto Other does not open its text field.
    await expect(canvas.queryByRole('textbox', { name: /Other answer/ })).toBeNull()
    await userEvent.keyboard('{ArrowDown}')
    await expect(boxes[0]).toHaveFocus()

    await userEvent.keyboard('2')
    await waitFor(() => expect(boxes[1]).toBeChecked())
    await userEvent.keyboard('5')
    const other = await canvas.findByRole('textbox', { name: /Other answer/ })
    await waitFor(() => expect(other).toHaveFocus())
    await userEvent.keyboard('Bundle size 3')
    // Negative control: the typed 3 does not toggle the third option.
    await expect(boxes[2]).not.toBeChecked()
    await expect(boxes[4]).toBeChecked()
    // Toggling Other off with its digit keeps focus on its row; toggling it back on recalls the typed text.
    await userEvent.keyboard('{Shift>}{Tab}{/Shift}')
    await expect(boxes[4]).toHaveFocus()
    await userEvent.keyboard('5')
    await waitFor(() => expect(boxes[4]).not.toBeChecked())
    await expect(boxes[4]).toHaveFocus()
    await expect(canvas.queryByRole('textbox', { name: /Other answer/ })).toBeNull()
    await userEvent.keyboard('5')
    const recalled = await canvas.findByRole('textbox', { name: /Other answer/ })
    await waitFor(() => expect(recalled).toHaveFocus())
    await expect(recalled).toHaveValue('Bundle size 3')
    await userEvent.keyboard('{Tab}')
    const note = canvas.getByRole('textbox', { name: 'Note' })
    await expect(note).toHaveFocus()
    await userEvent.keyboard('  Keep the run under ten minutes  ')

    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(form.dataset['state']).toBe('answered'))
    await expect(args.onSubmitted).toHaveBeenCalledTimes(1)
    await expect(args.onSubmitted).toHaveBeenCalledWith({ checks: { selected: ['unit'], other: 'Bundle size 3', note: 'Keep the run under ten minutes' } })
    await expect(form.querySelector('[data-question-summary="checks"]')).toHaveTextContent('Unit tests, Other: Bundle size 3')
    await expect(form.querySelector('[data-question-summary="checks"]')).toHaveTextContent('Note: Keep the run under ten minutes')
    await expect(canvas.queryAllByRole('checkbox')).toHaveLength(0)
    await assertHygiene(canvasElement)
  },
}

const expired: Story = {
  args: { scenario: 'S2', state: 'E' },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const form = formIn(canvasElement)
    await assertHygiene(canvasElement)
    await expect(form.dataset['state']).toBe('expired')
    await expect(form.querySelector('[data-question-state-chip]')).toHaveTextContent('Expired')
    await expect(form.querySelector('[data-question-settled]')).toHaveTextContent('Expired; the agent continued with the automatic answers below.')
    await expect(form.querySelector('[data-question-summary="storage"]')).toHaveTextContent('Embedded database')
    await expect(form.querySelector('[data-question-summary="features"]')).toHaveTextContent('No answer')
    // Negative control: an expired form has no answering controls, and keys pressed inside it cannot submit.
    await expect(canvas.queryAllByRole('radio')).toHaveLength(0)
    await expect(canvas.queryAllByRole('checkbox')).toHaveLength(0)
    await expect(canvas.queryByRole('button', { name: 'Submit' })).toBeNull()
    const terminal = canvas.getByRole('button', { name: "Open the agent's terminal" })
    terminal.focus()
    await expect(form).toContainElement(terminal)
    await expect(terminal).toHaveFocus()
    await userEvent.keyboard('1{Enter}')
    await expect(args.onSubmitted).not.toHaveBeenCalled()
  },
}

export const OneQuestion440: Story = { ...oneQuestion, args: { ...oneQuestion.args, width: 'W440' } }
export const OneQuestion1440: Story = { ...oneQuestion, args: { ...oneQuestion.args, width: 'W1440' } }
export const FourTabbed440: Story = { ...fourTabbed, args: { ...fourTabbed.args, width: 'W440' } }
export const FourTabbed1440: Story = { ...fourTabbed, args: { ...fourTabbed.args, width: 'W1440' } }
export const MultiOtherNote440: Story = { ...multiOtherNote, args: { ...multiOtherNote.args, width: 'W440' } }
export const MultiOtherNote1440: Story = { ...multiOtherNote, args: { ...multiOtherNote.args, width: 'W1440' } }
export const Expired440: Story = { ...expired, args: { ...expired.args, width: 'W440' } }
export const Expired1440: Story = { ...expired, args: { ...expired.args, width: 'W1440' } }

/** Picker-driven comparison of every scenario, state and width. */
export const Explore: Story = {
  args: { explore: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const picker = (label: string) => within(canvas.getByRole('group', { name: label }))
    const description = (label: string) => canvas.getByRole('group', { name: label }).querySelector('[data-picker-description]')
    await expect(canvas.getAllByRole('tab')).toHaveLength(4)
    await expect(description('Scenario')).toHaveTextContent('S2: Four tabbed questions')

    await userEvent.click(picker('Scenario').getByRole('button', { name: 'S1' }))
    await waitFor(() => expect(canvas.queryAllByRole('tab')).toHaveLength(0))
    await expect(canvas.getAllByRole('radio')).toHaveLength(4)
    await expect(description('Scenario')).toHaveTextContent('S1: One question')
    await expect(picker('Scenario').getByRole('button', { name: 'S1' })).toHaveAttribute('aria-pressed', 'true')
    await expect(picker('Scenario').getByRole('button', { name: 'S2' })).toHaveAttribute('aria-pressed', 'false')

    await userEvent.click(picker('Scenario').getByRole('button', { name: 'S3' }))
    await waitFor(() => expect(canvas.getAllByRole('checkbox')).toHaveLength(5))

    await userEvent.click(picker('State').getByRole('button', { name: 'S' }))
    await waitFor(() => expect(formIn(canvasElement).dataset['state']).toBe('answered'))
    await expect(formIn(canvasElement).querySelector('[data-question-summary="checks"]')).toHaveTextContent('Typecheck, Unit tests, Other: Bundle size budget')
    await expect(canvas.queryAllByRole('checkbox')).toHaveLength(0)

    await userEvent.click(picker('State').getByRole('button', { name: 'E' }))
    await waitFor(() => expect(formIn(canvasElement).dataset['state']).toBe('expired'))
    await expect(formIn(canvasElement).querySelector('[data-question-summary="checks"]')).toHaveTextContent('Typecheck')

    await userEvent.click(picker('Scenario').getByRole('button', { name: 'S2' }))
    await userEvent.click(picker('State').getByRole('button', { name: 'P' }))
    await userEvent.click(picker('Width').getByRole('button', { name: 'W440' }))
    await waitFor(() => expect(formIn(canvasElement).dataset['stacked']).toBe('true'))
    await assertStrip(formIn(canvasElement))
    await assertHygiene(canvasElement)
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas, color: text.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  controls: { display: 'flex', flexDirection: 'column', gap: s.md, width: '100%', maxWidth: g.polishWide, boxSizing: 'border-box', padding: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, backgroundColor: surface.raised },
  picker: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.md, minWidth: 0 },
  pickerLabel: { minWidth: g.statusWord, color: text.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  pickerButtons: { display: 'flex', gap: s.xs },
  pickerButton: { height: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', outlineStyle: 'none', ':hover': { color: text.fg } },
  pickerButtonSelected: { backgroundColor: surface.rowActive, color: text.fg, borderColor: text.fgMuted },
  focusRing: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  pickerDescription: { display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  window: { width: '100%', boxSizing: 'border-box', paddingBlock: s.xl, paddingInline: g.gutter, borderWidth: g.hairline, borderStyle: 'dashed', borderColor: border.borderStrong, borderRadius: r.md },
  narrow: { maxWidth: g.polishNarrow },
  wide: { maxWidth: g.polishWide },
  lane: { maxWidth: g.lane, marginInline: 'auto', minWidth: 0 },
})
