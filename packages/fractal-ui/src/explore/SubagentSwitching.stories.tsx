import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { SwitchingLane, variants, type SwitchingArgs } from './SubagentSwitching'
import { activeSubagents, scenarios, workspaces, sidebarFixtures, type Scenario } from './switching-fixtures'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t, geometryVars as g, borderVars as border, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'

const switchChoices = [{ id: 'S1', description: 'Bottom agent rail, beside the typing workflow.' }, { id: 'S2', description: 'Top tabs, with every delegated thread in the tab list.' }, { id: 'S3', description: 'Ancestry breadcrumb and a scalable thread drawer.' }] as const
const placementChoices = [{ id: 'chip', label: 'P1', description: 'Active count chip in the row’s inline metadata.' }, { id: 'badge', label: 'P2', description: 'Trailing count badge next to the status disc.' }] as const
function Picker({ label, choices, value, onChange }: { label: string; choices: readonly { id: string; label?: string; description: string }[]; value: string; onChange: (id: string) => void }) {
  return <section aria-label={label} {...stylex.props(styles.picker)}><strong>{label}</strong><div {...stylex.props(styles.pickerLine)}>{choices.map(choice => <Button key={choice.id} aria-pressed={choice.id === value} onPress={() => onChange(choice.id)} {...stylex.props(styles.button, choice.id === value && styles.selected)}>{choice.label ?? choice.id}</Button>)}<span data-picker-description {...stylex.props(styles.description)}>{choices.find(choice => choice.id === value)?.description}</span></div></section>
}
function Explorer(initial: SwitchingArgs & { probe: boolean; onUpdate?: (patch: Partial<SwitchingArgs>) => void }) {
  const [local, updateLocal] = React.useState(initial)
  const args = initial.probe ? local : initial
  const update = (patch: Partial<SwitchingArgs>) => initial.probe ? updateLocal(current => ({ ...current, ...patch })) : initial.onUpdate!(patch)
  const schemes = args.allLanes ? ['dark', 'light'] as const : [args.scheme]
  const widths = args.allLanes ? [1440, 900] as const : [args.width]
  return <main data-switching-explorer {...stylex.props(...baselineTheme, styles.root)}>
    <h1 {...stylex.props(styles.title)}>Subagent switching</h1>
    <p {...stylex.props(styles.description)}>Choose a switcher and an active-count placement. The sidebar and conversations use the same agent fixtures.</p>
    <div {...stylex.props(styles.pickers)}>
      <Picker label="S · Switcher" choices={switchChoices} value={args.switcher} onChange={switcher => update({ switcher: switcher as SwitchingArgs['switcher'] })} />
      <Picker label="P · Active count" choices={placementChoices} value={args.placement} onChange={placement => update({ placement: placement as SwitchingArgs['placement'] })} />
      <Picker label="W · Sidebar width" choices={[{ id: '240', description: 'Compact sidebar; every agent stays on one line.' }, { id: '320', description: 'More room for the agent name and count.' }]} value={String(args.sidebarWidth)} onChange={sidebarWidth => update({ sidebarWidth: Number(sidebarWidth) as 240 | 320 })} />
      <Picker label="Scenario" choices={scenarios.map(state => ({ id: state, description: `${workspaces[state].label}: ${activeSubagents(workspaces[state])} active subagents. All rows stay available in the sidebar.` }))} value={args.state} onChange={state => update({ state: state as Scenario, focusRequest: 0 })} />
    </div>
    <div {...stylex.props(styles.controls)}>{(['dark', 'light'] as const).map(scheme => <Button key={scheme} aria-pressed={args.scheme === scheme} onPress={() => update({ scheme, allLanes: false })} {...stylex.props(styles.button)}>{scheme}</Button>)}<Button aria-pressed={args.compare} onPress={() => update({ compare: !args.compare })} {...stylex.props(styles.button)}>Compare all S</Button><Button aria-pressed={args.allLanes} onPress={() => update({ allLanes: !args.allLanes })} {...stylex.props(styles.button)}>Light + dark / 1440 + 900</Button>{([1440, 900] as const).map(width => <Button key={width} aria-pressed={args.width === width} onPress={() => update({ width, allLanes: false })} {...stylex.props(styles.button)}>{width} viewport</Button>)}</div>
    <div {...stylex.props(styles.matrix)}>{schemes.flatMap(scheme => widths.map(width => <div key={`${scheme}/${width}`} {...stylex.props(styles.previewRow)}>{(args.compare ? variants : [args.switcher]).map(switcher => <SwitchingLane key={`${switcher}/${args.state}/${scheme}/${width}`} {...args} switcher={switcher} scheme={scheme} width={width} onOpenWorkspace={state => update({ state, focusRequest: args.focusRequest + 1 })} />)}</div>))}</div>
  </main>
}
const meta = {
  title: 'Fractal UI/Explore/SubagentSwitching', component: Explorer,
  render: args => {
    const [, updateArgs] = useArgs<SwitchingArgs>()
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
  args: { switcher: 'S1', placement: 'chip', sidebarWidth: 240, state: 'running', compare: false, allLanes: false, scheme: 'dark', width: 1440, focusRequest: 0, probe: false },
  argTypes: { switcher: { options: variants, control: 'radio' }, placement: { options: ['chip', 'badge'], control: 'radio' }, sidebarWidth: { options: [240, 320], control: 'radio' }, state: { options: scenarios, control: 'select' }, compare: { control: 'boolean' }, allLanes: { control: 'boolean' }, scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' }, focusRequest: { table: { disable: true } }, probe: { table: { disable: true } }, onUpdate: { table: { disable: true } } },
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
function countMatches(row: HTMLElement, state: Scenario): boolean {
  const button = row.querySelector<HTMLElement>('[data-active-count]')!
  const visible = row.querySelector('[data-count-text]')!.textContent!
  return Number.parseInt(visible, 10) === activeSubagents(workspaces[state]) && button.getAttribute('aria-label')!.includes(visible)
}
function switcherFocused(lane: HTMLElement): boolean {
  const target = lane.querySelector('[data-switcher]')
  return target === document.activeElement || target?.contains(document.activeElement) === true
}
export const SubagentSwitchingProbe: Story = { args: { probe: true, state: 'many', width: 900 }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  for (const switcher of variants) {
    await userEvent.click(canvas.getByRole('button', { name: switcher, exact: true }))
    await waitFor(() => expect(canvasElement.querySelector('[data-switching-lane]')).toHaveAttribute('data-variant', switcher))
    for (const placement of ['P1', 'P2']) {
      await userEvent.click(canvas.getByRole('button', { name: placement, exact: true }))
      for (const fixture of sidebarFixtures) {
        const lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
        const row = lane.querySelector<HTMLElement>(`[data-count-row="${fixture.state}"]`)!
        await expect(singleLine(row)).toBe(true)
        await expect(row.getBoundingClientRect().width).toBeLessThanOrEqual(240)
        await expect(countMatches(row, fixture.state)).toBe(true)
        const count = row.querySelector<HTMLElement>('[data-active-count]')!
        await userEvent.click(count)
        await waitFor(() => {
          const updated = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
          expect(updated).toHaveAttribute('data-state', fixture.state)
          expect(switcherFocused(updated)).toBe(true)
        })
      }
    }
    await userEvent.click(canvas.getByRole('button', { name: 'many', exact: true }))
    let lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
    const controls = within(lane)
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-1'))
    await expect(lane).toHaveTextContent('Check layout behaviour')
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowLeft}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
    await userEvent.click(controls.getByRole('button', { name: 'All agent threads', exact: true }))
    const drawer = await controls.findByRole('complementary', { name: 'All agent threads', exact: true })
    await expect(drawer.querySelectorAll('[data-row-column="title-text"]')).toHaveLength(workspaces.many.agents.length)
    await userEvent.click(within(drawer).getByText('Recovery', { exact: true }))
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-14'))
    await expect(lane).toHaveTextContent('Check recovery behaviour')
    await userEvent.click(controls.getByRole('button', { name: 'Close agent threads', exact: true }))
    await waitFor(() => expect(lane.querySelector('[data-thread-drawer]')).toBeNull())
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
  }
  let lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
  let row = lane.querySelector<HTMLElement>('[data-count-row="running"]')!
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
  const inert = document.createElement('button')
  inert.textContent = 'Inert count control'
  row.append(inert)
  await userEvent.click(inert)
  await expect(switcherFocused(lane)).toBe(false)
  inert.remove()
  await userEvent.click(row.querySelector<HTMLElement>('[data-active-count]')!)
  await waitFor(() => expect(switcherFocused(canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!)).toBe(true))
  await userEvent.click(canvas.getByRole('button', { name: 'nested', exact: true }))
  lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
  await expect(lane).toHaveAttribute('data-active', 'great-grandchild')
  await expect(within(lane).getByRole('navigation', { name: 'Agent ancestry' })).toHaveTextContent('Key map reviewLayoutKey mapShortcut audit')
  await userEvent.click(within(lane).getByRole('button', { name: 'Switch to Key map review', exact: true }))
  await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
  await userEvent.click(canvas.getByRole('button', { name: 'attention', exact: true }))
  lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
  await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-1'))
  await expect(lane).toHaveTextContent('waiting on your preference')
  await userEvent.type(within(lane).getByRole('textbox', { name: 'Message', exact: true }), 'Keep finished agents available.')
  await userEvent.click(within(lane).getByRole('button', { name: 'Send', exact: true }))
  await waitFor(() => expect(lane).toHaveTextContent('Keep finished agents available.'))
} }
const styles = stylex.create({
  root: { padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize }, title: { margin: s.zero, marginBottom: s.md, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightMedium }, description: { margin: s.zero, color: ink.fgSoft, lineHeight: t.uiLeading }, pickers: { display: 'flex', flexDirection: 'column', gap: s.section, marginBlock: s.section }, picker: { display: 'flex', flexDirection: 'column', gap: s.md }, pickerLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md }, controls: { display: 'flex', flexWrap: 'wrap', gap: s.md, marginBottom: s.section }, button: { fontFamily: t.fontSans, fontSize: t.metaSize, color: ink.fgSoft, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, padding: s.md, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, selected: { borderColor: accent.primary, color: ink.fg }, matrix: { overflowX: 'auto' }, previewRow: { display: 'flex', alignItems: 'flex-start', gap: s.section, marginBottom: s.section },
})
