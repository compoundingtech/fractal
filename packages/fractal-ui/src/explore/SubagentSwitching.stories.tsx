import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { SwitchingLane, laneId, switcherNames, variants, type SwitchingArgs } from './SubagentSwitching'
import { activeCountName, activeSubagents, scenarios, workspaces, sidebarFixtures, type Scenario } from './switching-fixtures'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t, geometryVars as g, borderVars as border, radiusVars as r, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'

interface Choice { readonly id: string; readonly label?: string; readonly description: string }
const switchChoices: readonly Choice[] = [
  { id: 'S1', description: `${switcherNames.S1}: one chip per thread under the composer, like coding-agent CLIs. Overflow opens the thread drawer.` },
  { id: 'S2', description: `${switcherNames.S2}: one tab per thread above the conversation. Overflow opens the thread drawer.` },
  { id: 'S3', description: `${switcherNames.S3}: the path to the open thread, plus a drawer that lists every thread as a tree. Scales to deep and wide delegation.` },
]
const placementChoices: readonly Choice[] = [
  { id: 'chip', label: 'P1', description: 'Count tag right after the name, read with the row’s metadata. Nothing shows at zero.' },
  { id: 'badge', label: 'P2', description: 'Numeric badge at the trailing edge, beside the status disc. Nothing shows at zero.' },
]
const widthChoices: readonly Choice[] = [{ id: '240', description: 'Compact sidebar; every row stays on one line.' }, { id: '320', description: 'Default-width sidebar.' }]
const scenarioChoices: readonly Choice[] = scenarios.map(state => ({ id: state, description: `${workspaces[state].label}: ${activeCountName(workspaces[state])}. Counts include running and waiting subagents, never finished or failed ones.` }))

/** One picker row per dimension: short IDs, then a one-line description of the current pick. */
function Picker({ label, choices, value, onChange }: { label: string; choices: readonly Choice[]; value: string; onChange: (id: string) => void }) {
  const id = React.useId()
  return <div role="group" aria-labelledby={id} {...stylex.props(styles.picker)}>
    <span id={id} {...stylex.props(styles.pickerLabel)}>{label}</span>
    <div {...stylex.props(styles.pickerLine)}>
      {choices.map(choice => <Button key={choice.id} aria-pressed={choice.id === value} onPress={() => onChange(choice.id)} {...stylex.props(styles.button, choice.id === value && styles.selected)}>{choice.label ?? choice.id}</Button>)}
      <span data-picker-description {...stylex.props(styles.description)}>{choices.find(choice => choice.id === value)?.description}</span>
    </div>
  </div>
}
function Toggle({ pressed, onPress, children }: { pressed: boolean; onPress: () => void; children: React.ReactNode }) {
  return <Button aria-pressed={pressed} onPress={onPress} {...stylex.props(styles.button, pressed && styles.selected)}>{children}</Button>
}
function Explorer(initial: SwitchingArgs & { probe: boolean; onUpdate?: (patch: Partial<SwitchingArgs>) => void }) {
  const [local, updateLocal] = React.useState(initial)
  const args = initial.probe ? local : initial
  const update = (patch: Partial<SwitchingArgs>) => initial.probe ? updateLocal(current => ({ ...current, ...patch })) : initial.onUpdate!(patch)
  const schemes = args.allLanes ? ['dark', 'light'] as const : [args.scheme]
  const widths = args.allLanes ? [1440, 900] as const : [args.width]
  const switchers = args.compare ? variants : [args.switcher]
  const multiple = args.allLanes || args.compare
  return <main data-switching-explorer {...stylex.props(...baselineTheme, styles.root)}>
    <h1 {...stylex.props(styles.title)}>Subagent switching</h1>
    <p {...stylex.props(styles.lede)}>Pick a switcher and where the sidebar shows each agent’s active subagents. The sidebar rows and every switcher read the same fixture. Pressing a count opens that agent with its switcher focused.</p>
    <div {...stylex.props(styles.pickers)}>
      <Picker label="S · Switcher" choices={switchChoices} value={args.switcher} onChange={switcher => update({ switcher: switcher as SwitchingArgs['switcher'], focusRequest: 0, focusLane: '' })} />
      <Picker label="P · Active count" choices={placementChoices} value={args.placement} onChange={placement => update({ placement: placement as SwitchingArgs['placement'] })} />
      <Picker label="W · Sidebar width" choices={widthChoices} value={String(args.sidebarWidth)} onChange={sidebarWidth => update({ sidebarWidth: Number(sidebarWidth) as SwitchingArgs['sidebarWidth'] })} />
      <Picker label="Scenario" choices={scenarioChoices} value={args.state} onChange={state => update({ state: state as Scenario, focusRequest: 0, focusLane: '' })} />
    </div>
    <div role="group" aria-label="View" {...stylex.props(styles.view)}>
      <span {...stylex.props(styles.pickerLabel)}>View</span>
      <div {...stylex.props(styles.segment)}>{(['dark', 'light'] as const).map(scheme => <Toggle key={scheme} pressed={!args.allLanes && args.scheme === scheme} onPress={() => update({ scheme, allLanes: false, focusRequest: 0, focusLane: '' })}>{scheme}</Toggle>)}</div>
      <div {...stylex.props(styles.segment)}>{([1440, 900] as const).map(width => <Toggle key={width} pressed={!args.allLanes && args.width === width} onPress={() => update({ width, allLanes: false, focusRequest: 0, focusLane: '' })}>{width} wide</Toggle>)}</div>
      <div {...stylex.props(styles.segment)}>
        <Toggle pressed={args.compare} onPress={() => update({ compare: !args.compare })}>Compare S1–S3</Toggle>
        <Toggle pressed={args.allLanes} onPress={() => update({ allLanes: !args.allLanes })}>Light + dark × 1440 + 900</Toggle>
      </div>
    </div>
    <div {...stylex.props(styles.matrix)}>{schemes.flatMap(scheme => widths.map(width => <div key={`${scheme}/${width}`} {...stylex.props(styles.previewRow)}>{switchers.map(switcher => <SwitchingLane key={`${switcher}/${args.state}/${scheme}/${width}`} {...args} switcher={switcher} scheme={scheme} width={width} landmarkContext={multiple ? `${switcher} ${scheme} ${width}` : undefined} onOpenWorkspace={(state, focusSwitcher) => update(focusSwitcher ? { state, focusRequest: args.focusRequest + 1, focusLane: laneId({ switcher, scheme, width }) } : { state, focusRequest: 0, focusLane: '' })} />)}</div>))}</div>
  </main>
}
const meta = {
  title: 'Fractal UI/Explore/SubagentSwitching', component: Explorer,
  render: args => {
    const [, updateArgs] = useArgs<SwitchingArgs>()
    // Picks live in the story args and the URL, so every combination is a shareable link, also from the bare iframe.
    const update = (patch: Partial<SwitchingArgs>) => {
      updateArgs(patch)
      const next = { ...args, ...patch }
      const url = new URL(window.location.href)
      url.searchParams.set('args', Object.entries(next).filter(([key]) => key !== 'onUpdate' && key !== 'probe').map(([key, value]) => `${key}:${typeof value === 'boolean' ? `!${value}` : value}`).join(';'))
      window.history.replaceState(window.history.state, '', url)
    }
    return <Explorer {...args} onUpdate={update} />
  },
  parameters: { layout: 'fullscreen' },
  args: { switcher: 'S1', placement: 'chip', sidebarWidth: 240, state: 'running', compare: false, allLanes: false, scheme: 'dark', width: 1440, focusRequest: 0, focusLane: '', probe: false },
  argTypes: { switcher: { options: variants, control: 'radio' }, placement: { options: ['chip', 'badge'], control: 'radio' }, sidebarWidth: { options: [240, 320], control: 'radio' }, state: { options: scenarios, control: 'select' }, compare: { control: 'boolean' }, allLanes: { control: 'boolean' }, scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' }, focusRequest: { table: { disable: true } }, focusLane: { table: { disable: true } }, probe: { table: { disable: true } }, onUpdate: { table: { disable: true } } },
} satisfies Meta<typeof Explorer>
export default meta
type Story = StoryObj<typeof meta>
export const Exploration: Story = {}

function singleLine(row: HTMLElement): boolean {
  return [...row.querySelectorAll('[data-row-name], [data-count-text]')].every(element => {
    const range = document.createRange()
    range.selectNodeContents(element)
    return range.getClientRects().length === 1
  }) && row.scrollWidth <= row.clientWidth && row.getBoundingClientRect().height <= 32
}
/** The visible number equals the fixture's active subagents, its text starts the spoken name, and the name is the fixture's words. Zero shows no control. */
function countMatches(row: HTMLElement, state: Scenario): boolean {
  const count = row.querySelector<HTMLElement>('[data-active-count]')
  const expected = activeSubagents(workspaces[state])
  if (expected === 0) return count === null
  const visible = row.querySelector('[data-count-text]')?.textContent ?? ''
  const name = activeCountName(workspaces[state])
  return Number.parseInt(visible, 10) === expected && name.startsWith(visible) && within(row).queryByRole('button', { name, exact: true }) === count
}
function switcherFocused(lane: HTMLElement): boolean {
  const focused = document.activeElement
  return focused !== null && lane.contains(focused) && focused.closest('[data-switcher]') !== null
}
const currentLane = (canvasElement: HTMLElement) => canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
export const SubagentSwitchingProbe: Story = { args: { probe: true, state: 'many', width: 900 }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  for (const switcher of variants) {
    await userEvent.click(canvas.getByRole('button', { name: switcher, exact: true }))
    await waitFor(() => expect(currentLane(canvasElement)).toHaveAttribute('data-variant', switcher))
    for (const placement of ['P1', 'P2']) {
      await userEvent.click(canvas.getByRole('button', { name: placement, exact: true }))
      for (const fixture of sidebarFixtures) {
        const row = currentLane(canvasElement).querySelector<HTMLElement>(`[data-count-row="${fixture.state}"]`)!
        await expect(singleLine(row)).toBe(true)
        await expect(row.getBoundingClientRect().width).toBeLessThanOrEqual(240)
        await expect(countMatches(row, fixture.state)).toBe(true)
        const count = row.querySelector<HTMLElement>('[data-active-count]')
        // Pressing the name opens the agent without moving focus to the switcher; pressing the count does both.
        await userEvent.click(count ?? row.querySelector<HTMLElement>('[data-row-name]')!)
        await waitFor(() => {
          const lane = currentLane(canvasElement)
          expect(lane).toHaveAttribute('data-state', fixture.state)
          if (count !== null) expect(lane).toHaveAttribute('data-active', 'root')
          expect(switcherFocused(lane)).toBe(count !== null)
        })
      }
    }
    await userEvent.click(canvas.getByRole('button', { name: 'many', exact: true }))
    const lane = currentLane(canvasElement)
    const controls = within(lane)
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-1'))
    await expect(lane).toHaveTextContent('Check layout behaviour')
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowLeft}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
    await userEvent.click(lane.querySelector<HTMLElement>('[data-thread-drawer-toggle]')!)
    const drawer = await controls.findByRole('complementary', { name: 'All agent threads', exact: true })
    await expect(drawer.querySelectorAll('[data-thread-item]')).toHaveLength(workspaces.many.agents.length)
    await userEvent.click(within(drawer).getByRole('button', { name: 'Recovery, running', exact: true }))
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-14'))
    await expect(lane).toHaveTextContent('Check recovery behaviour')
    await userEvent.click(controls.getByRole('button', { name: 'Close agent threads', exact: true }))
    await waitFor(() => expect(lane.querySelector('[data-thread-drawer]')).toBeNull())
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
  }
  // Negative controls: each assertion must fail on a broken row, then pass again once restored.
  let lane = currentLane(canvasElement)
  const row = lane.querySelector<HTMLElement>('[data-count-row="running"]')!
  const name = row.querySelector<HTMLElement>('[data-row-name]')!
  const originalName = name.textContent
  name.append(document.createElement('br'), document.createTextNode('Second line'))
  await expect(singleLine(row)).toBe(false)
  name.textContent = originalName
  await expect(singleLine(row)).toBe(true)
  const countText = row.querySelector<HTMLElement>('[data-count-text]')!
  const originalCount = countText.textContent
  countText.textContent = '99'
  await expect(countMatches(row, 'running')).toBe(false)
  countText.textContent = originalCount
  await expect(countMatches(row, 'running')).toBe(true)
  const spoken = row.querySelector<HTMLElement>('[data-active-count] > span:last-child')!
  const originalSpoken = spoken.textContent
  spoken.textContent = ''
  await expect(countMatches(row, 'running')).toBe(false)
  spoken.textContent = originalSpoken
  await expect(countMatches(row, 'running')).toBe(true)
  await expect(countMatches(lane.querySelector<HTMLElement>('[data-count-row="zero"]')!, 'running')).toBe(false)
  const inert = document.createElement('button')
  inert.textContent = 'Inert count control'
  row.append(inert)
  await userEvent.click(inert)
  await expect(switcherFocused(lane)).toBe(false)
  inert.remove()
  await userEvent.click(row.querySelector<HTMLElement>('[data-active-count]')!)
  await waitFor(() => expect(switcherFocused(currentLane(canvasElement))).toBe(true))
  await userEvent.click(canvas.getByRole('button', { name: 'nested', exact: true }))
  lane = currentLane(canvasElement)
  await expect(lane).toHaveAttribute('data-active', 'great-grandchild')
  const ancestry = within(lane).getByRole('navigation', { name: 'Agent ancestry' })
  await expect([...ancestry.querySelectorAll('li')].map(item => item.textContent)).toEqual(['Key map review', 'Layout', 'Key map', 'Shortcut audit, finished'])
  await expect(ancestry.querySelector('[aria-current="page"]')).toHaveTextContent('Shortcut audit')
  await userEvent.click(within(ancestry).getByRole('button', { name: 'Key map review', exact: true }))
  await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
  await userEvent.click(canvas.getByRole('button', { name: 'attention', exact: true }))
  lane = currentLane(canvasElement)
  await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-1'))
  await expect(lane.querySelector('[data-needs-you]')).toHaveTextContent('Layout is waiting on your preference')
  await userEvent.type(within(lane).getByRole('textbox', { name: 'Message', exact: true }), 'Keep finished agents available.')
  await userEvent.click(within(lane).getByRole('button', { name: 'Send', exact: true }))
  await waitFor(() => expect(lane).toHaveTextContent('Keep finished agents available.'))
} }
const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  title: { margin: s.zero, marginBottom: s.md, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightMedium },
  lede: { margin: s.zero, maxWidth: g.lane, color: ink.fgSoft },
  pickers: { display: 'flex', flexDirection: 'column', gap: s.xl, marginBlock: s.section },
  picker: { display: 'flex', flexDirection: 'column', gap: s.md },
  pickerLabel: { color: ink.fg, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  pickerLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md },
  description: { marginInlineStart: s.md, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading },
  view: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.lg, marginBottom: s.section, paddingTop: s.xl, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  segment: { display: 'flex', gap: s.md },
  button: { minHeight: g.controlLg, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  selected: { backgroundColor: surface.rowActive, borderColor: ink.fgMuted, color: ink.fg, ':hover': { backgroundColor: surface.rowActive } },
  matrix: { display: 'flex', flexDirection: 'column', gap: s.section, overflowX: 'auto' },
  previewRow: { display: 'flex', alignItems: 'flex-start', gap: s.section },
})
