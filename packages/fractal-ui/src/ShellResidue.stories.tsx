import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { ResizableSplit } from './assistant-ui/composition/Shell.tsx'
import { baselineTheme } from './assistant-ui/neutral-theme.ts'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme.ts'
import { surfaceVars as surface, textVars as ink, spaceVars as s, geometryVars as g, typeVars as t } from './assistant-ui/composition-tokens.stylex.ts'

const SplitSurface = ({ scheme = 'dark', width = 1440, ignoreReverse = false }: { readonly scheme?: Scheme; readonly width?: number; readonly ignoreReverse?: boolean }) => {
  const [value, setValue] = React.useState(300)
  const [locked, setLocked] = React.useState(false)
  const [committed, setCommitted] = React.useState(300)
  return <main data-split-surface="" style={{ width, maxWidth: '100%' }} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.track)}><ResizableSplit id="sample-inspector" reverse={!ignoreReverse} value={value} min={240} max={locked ? 240 : 380} collapsed={false} label="Inspector width" onChange={setValue} onCommit={setCommitted} onToggle={() => setLocked(current => !current)} onReset={() => setValue(300)} /></div>
    <output aria-label="Current width">{value}</output><output aria-label="Committed width">{committed}</output>
    <Button onPress={() => setLocked(current => !current)}>{locked ? 'Allow resizing' : 'Lock resizing'}</Button>
  </main>
}
const meta = { title: 'Fractal UI/Shell/Resizable split', component: SplitSurface, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', width: 1440 }, argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, width: { control: 'select', options: [440, 1440] } } } satisfies Meta<typeof SplitSurface>
export default meta
type Story = StoryObj<typeof meta>
const play: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const split = canvas.getByRole('separator', { name: 'Inspector width' })
  split.focus()
  await userEvent.keyboard('{ArrowRight}')
  await expect(split).toHaveAttribute('aria-valuenow', '292')
  await expect(canvas.getByLabelText('Committed width')).toHaveTextContent('292')
  await userEvent.keyboard('{Shift>}{ArrowLeft}{/Shift}')
  await expect(split).toHaveAttribute('aria-valuenow', '324')
  await userEvent.keyboard('{Home}')
  await expect(split).toHaveAttribute('aria-valuenow', '240')
  await userEvent.keyboard('{End}')
  await expect(split).toHaveAttribute('aria-valuenow', '380')
  await userEvent.click(canvas.getByRole('button', { name: 'Lock resizing' }))
  await expect(canvas.queryByRole('separator')).toBeNull()
  await expect(canvasElement.querySelector('[data-resizer-track]')).toBeInTheDocument()
  await userEvent.click(canvas.getByRole('button', { name: 'Allow resizing' }))
  await expect(canvas.getByRole('separator', { name: 'Inspector width' })).toHaveAttribute('aria-valuenow', '380')
}
export const Dark1440: Story = { play }
export const Light1440: Story = { args: { scheme: 'light' }, play }
export const Dark440: Story = { args: { width: 440 }, play }
export const Light440: Story = { args: { width: 440, scheme: 'light' }, play }
export const WrongDirectionControl: Story = { args: { ignoreReverse: true }, play }
export const AllStates: Story = { render: () => <><SplitSurface width={440} /><SplitSurface width={440} scheme="light" /></> }
const styles = stylex.create({ root: { boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans }, track: { display: 'flex', minHeight: g.band } })
