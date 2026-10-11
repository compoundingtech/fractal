import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { LiveActivity, Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { Workbench } from './assistant-ui/workbench/Workbench'
import { decidedAppearance, onePaneLayout, useWorkbenchFixture } from './assistant-ui/workbench/workbench-fixtures'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, surfaceVars as surface, spaceVars as s } from './assistant-ui/composition-tokens.stylex'
import type { TextItem } from './assistant-ui/embrace-data/model'

const startedAt = '2026-01-15T12:00:00Z'
const clock = Date.parse(startedAt) + 12000
const longPrompt = Array.from({ length: 12 }, (_, index) => `Requirement ${index + 1}: Preserve the selected resource, keyboard focus, and the complete operator message while changing the visible layout.`).join('\n')
const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: 'polish/prompt', role: 'user', text: longPrompt, attachments: [], streaming: false, at: startedAt }
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', backgroundColor: surface.canvas, padding: s.md },
  narrow: { width: g.polishNarrow },
  workbench: { height: g.transcriptFrameHeight },
  transcript: { height: g.transcriptFrameHeight },
})
const meta = {
  title: 'Fractal UI/Live polish', component: LiveActivity,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark' as Scheme, narrow: false, failingControl: false, now: clock, startedAt },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, failingControl: { control: 'boolean', description: 'Remove the run start fact: the elapsed-label assertion must fail.' } },
  render: function Render({ scheme, narrow, failingControl, now }) {
    const fixture = useWorkbenchFixture('idle')
    return <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
      <section aria-label="Live activity example"><LiveActivity startedAt={failingControl ? undefined : startedAt} now={now} /></section>
      <section aria-label="Long prompt example" {...stylex.props(styles.transcript)}><EmbraceRuntimeProvider options={{ messages: [prompt], onNew: async () => {} }}><Transcript title="Long operator message" turns={[{ id: 'polish', prompt, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }]} sync={{ _tag: 'Live', since: clock }} now={clock} observedAt={clock} /></EmbraceRuntimeProvider></section>
      <section aria-label="History action example"><EmbraceRuntimeProvider options={{ messages: [], onNew: async () => {} }}><EmbraceComposer variant="C3" history={['Review the selected resource.']} /></EmbraceRuntimeProvider></section>
      <section aria-label="Composer dock example" {...stylex.props(styles.workbench)}><Workbench layout={onePaneLayout} resources={fixture.resources} appearance={decidedAppearance} scheme={scheme} workspaceId={`live-polish-${scheme}-${narrow}`} /></section>
    </main>
  },
} satisfies Meta<React.ComponentProps<typeof LiveActivity> & { scheme: Scheme; narrow: boolean; failingControl: boolean }>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = { play: async ({ mount, args }) => {
  const canvas = await mount()
  await expect(within(canvas.getByRole('region', { name: 'Live activity example' })).getByTestId('live-work')).toHaveTextContent('Working for 12s')
  const longPromptCanvas = within(canvas.getByRole('region', { name: 'Long prompt example' }))
  const expand = await longPromptCanvas.findByRole('button', { name: 'Show full message' })
  const message = longPromptCanvas.getByTestId('user-message')
  const promptText = message.ownerDocument.getElementById(expand.getAttribute('aria-controls')!)!
  await expect(promptText.textContent!.length).toBeLessThan(longPrompt.length)
  await expect(message.getBoundingClientRect().height).toBeLessThanOrEqual(176)
  await userEvent.click(expand)
  await expect(expand).toHaveFocus()
  await expect(expand).toHaveAttribute('aria-expanded', 'true')
  await expect(promptText.textContent).toBe(longPrompt)
  await userEvent.click(expand)
  await expect(expand).toHaveFocus()
  await expect(expand).toHaveAttribute('aria-expanded', 'false')
  await expect(promptText.textContent!.length).toBeLessThan(longPrompt.length)
  await expect(message.getBoundingClientRect().height).toBeLessThanOrEqual(176)
  const history = within(canvas.getByRole('region', { name: 'History action example' })).getByRole('button', { name: 'Recall previous message' })
  await expect(history).toBeEnabled()
  await expect(getComputedStyle(history).opacity).toBe('1')
  await expect(history.getBoundingClientRect().width).toBe(28)
  await expect(getComputedStyle(history).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  const dock = within(canvas.getByRole('region', { name: 'Composer dock example' })).getByTestId('workbench-composer-dock')
  const composer = within(dock).getByTestId('composer')
  await expect(dock.getBoundingClientRect().height - composer.getBoundingClientRect().height).toBeLessThanOrEqual(12)
  const input = within(dock).getByTestId('composer-input')
  const send = within(dock).getByRole('button', { name: 'Send' })
  await userEvent.clear(input)
  await expect(send).toBeDisabled()
  await userEvent.type(input, 'Keep the selected resource visible.')
  await expect(send).toBeEnabled()
  await expect(getComputedStyle(send).opacity).toBe('1')
  await expect(send.getBoundingClientRect().width).toBe(32)
  if (args.scheme === 'light') await expect(getComputedStyle(dock).backgroundColor).toBe('rgb(252, 252, 252)')
  const advanced = await mount(<meta.render {...args} now={clock + 1000} />)
  await expect(within(advanced.getByRole('region', { name: 'Live activity example' })).getByTestId('live-work')).toHaveTextContent('Working for 13s')
  const restoredDock = within(advanced.getByRole('region', { name: 'Composer dock example' })).getByTestId('workbench-composer-dock')
  await waitFor(() => {
    expect(within(restoredDock).getByTestId('composer-input')).toHaveValue('Keep the selected resource visible.')
    expect(within(restoredDock).queryByTestId('composer-placeholder')).toBeNull()
  })
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
export const AllStates: Story = { ...Dark1440 }
