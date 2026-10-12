import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { SwitchingLane, variants, scenarios, type SwitchingArgs } from './SubagentSwitching'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t, geometryVars as g, borderVars as border, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'

function Explorer(initial: SwitchingArgs & { probe: boolean }) {
  const [, updateStory] = useArgs<SwitchingArgs>()
  const [local, updateLocal] = React.useState(initial)
  const args = initial.probe ? local : initial
  const update = (patch: Partial<SwitchingArgs>) => initial.probe ? updateLocal(current => ({ ...current, ...patch })) : updateStory(patch)
  const choices = args.compare ? variants : [args.variant]
  const states = args.allStates ? scenarios : [args.state]
  const schemes = args.allLanes ? ['dark', 'light'] as const : [args.scheme]
  const widths = args.allLanes ? [1440, 900] as const : [args.width]
  return <main {...stylex.props(...baselineTheme, styles.root)}><h1>Subagent switching</h1><p>Keep the root conversation close while exploring delegated work. All controls operate on independent thread state.</p>
    <div aria-label="Exploration controls" {...stylex.props(styles.controls)}>{variants.map(variant => <Button key={variant} aria-pressed={args.variant === variant} onPress={() => update({ variant, compare: false })} {...stylex.props(styles.button)}>Variant {variant}</Button>)}{scenarios.map(state => <Button key={state} aria-pressed={args.state === state} onPress={() => update({ state, allStates: false })} {...stylex.props(styles.button)}>State {state}</Button>)}<Button aria-pressed={args.compare} onPress={() => update({ compare: !args.compare })} {...stylex.props(styles.button)}>Compare variants</Button><Button aria-pressed={args.allStates} onPress={() => update({ allStates: !args.allStates })} {...stylex.props(styles.button)}>All states</Button><Button aria-pressed={args.allLanes} onPress={() => update({ allLanes: !args.allLanes })} {...stylex.props(styles.button)}>All themes and widths</Button>{(['dark', 'light'] as const).map(scheme => <Button key={scheme} onPress={() => update({ scheme, allLanes: false })} {...stylex.props(styles.button)}>{scheme}</Button>)}{([1440, 900] as const).map(width => <Button key={width} onPress={() => update({ width, allLanes: false })} {...stylex.props(styles.button)}>{width} width</Button>)}</div>
    <div {...stylex.props(styles.matrix)}>{states.map(state => <section key={state}><h2>{state}</h2>{schemes.flatMap(scheme => widths.map(width => <div key={`${scheme}/${width}`} {...stylex.props(styles.row)}>{choices.map(variant => <SwitchingLane key={`${variant}/${state}/${scheme}/${width}`} variant={variant} state={state} scheme={scheme} width={width} />)}</div>))}</section>)}</div>
  </main>
}
const meta = { title: 'Fractal UI/Explore/SubagentSwitching', component: Explorer, parameters: { layout: 'fullscreen' }, args: { variant: 'A', state: 'running', compare: true, allStates: false, allLanes: true, scheme: 'dark', width: 1440, probe: false }, argTypes: { variant: { options: variants, control: 'radio' }, state: { options: scenarios, control: 'select' }, compare: { control: 'boolean' }, allStates: { control: 'boolean' }, allLanes: { control: 'boolean' }, scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' }, probe: { table: { disable: true } } } } satisfies Meta<typeof Explorer>
export default meta
type Story = StoryObj<typeof meta>
export const Exploration: Story = {}
export const AllStates: Story = { args: { allStates: true } }
export const SubagentSwitchingProbe: Story = { args: { probe: true, compare: false, allLanes: false, state: 'many', width: 900 }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  for (const variant of variants) {
    await userEvent.click(canvas.getByRole('button', { name: `Variant ${variant}`, exact: true }))
    await waitFor(() => expect(canvasElement.querySelector('[data-switching-lane]')).toHaveAttribute('data-variant', variant))
    const lane = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
    const controls = within(lane)
    const input = controls.getByRole('textbox', { name: 'Message', exact: true })
    await userEvent.click(input)
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-1'))
    await expect(controls.getByRole('textbox', { name: 'Message', exact: true })).toBeInTheDocument()
    await expect(lane).toHaveTextContent('Check layout behaviour')
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowLeft}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
    await userEvent.click(controls.getByRole('button', { name: 'All agent threads', exact: true }))
    const drawer = await controls.findByRole('complementary', { name: 'All agent threads', exact: true })
    await expect(drawer.querySelectorAll('[data-row-column="title-text"]')).toHaveLength(15)
    const target = within(drawer).getByText('Recovery', { exact: true })
    await userEvent.click(target)
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'child-14'))
    await expect(lane).toHaveTextContent('Check recovery behaviour')
    await expect(controls.getByRole('textbox', { name: 'Message', exact: true })).toBeInTheDocument()
    await userEvent.click(controls.getByRole('button', { name: 'Close agent threads', exact: true }))
    await waitFor(() => expect(lane.querySelector('[data-thread-drawer]')).toBeNull())
    await userEvent.click(controls.getByRole('textbox', { name: 'Message', exact: true }))
    await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
    await waitFor(() => expect(lane).toHaveAttribute('data-active', 'root'))
  }
  await userEvent.click(canvas.getByRole('button', { name: 'State nested', exact: true }))
  await waitFor(() => expect(canvasElement.querySelector('[data-switching-lane]')).toHaveAttribute('data-state', 'nested'))
  const nested = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
  await expect(nested).toHaveAttribute('data-active', 'great-grandchild')
  await expect(within(nested).getByRole('navigation', { name: 'Agent ancestry' })).toHaveTextContent('WorkspaceLayoutKey mapShortcut audit')
  await userEvent.click(within(nested).getByRole('button', { name: 'Switch to Workspace', exact: true }))
  await waitFor(() => expect(nested).toHaveAttribute('data-active', 'root'))
  await userEvent.click(canvas.getByRole('button', { name: 'State attention', exact: true }))
  const attention = canvasElement.querySelector<HTMLElement>('[data-switching-lane]')!
  await userEvent.click(within(attention).getByRole('textbox', { name: 'Message', exact: true }))
  await userEvent.keyboard('{Control>}{ArrowRight}{/Control}')
  await waitFor(() => expect(attention).toHaveAttribute('data-active', 'child-1'))
  await expect(attention).toHaveTextContent('waiting on your preference')
  const reply = within(attention).getByRole('textbox', { name: 'Message', exact: true })
  await userEvent.type(reply, 'Keep finished agents available.')
  await userEvent.click(within(attention).getByRole('button', { name: 'Send', exact: true }))
  await waitFor(() => expect(attention).toHaveTextContent('Keep finished agents available.'))
} }
const styles = stylex.create({
  root: { padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize }, controls: { display: 'flex', flexWrap: 'wrap', gap: s.sm, marginBottom: s.section }, button: { fontFamily: t.fontSans, fontSize: t.metaSize, color: ink.fgSoft, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, padding: s.md, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, matrix: { overflowX: 'auto' }, row: { display: 'flex', alignItems: 'flex-start', gap: s.section, marginBottom: s.section },
})
