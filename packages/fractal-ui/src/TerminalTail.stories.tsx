import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, within } from 'storybook/test'
import { TerminalTail } from './assistant-ui/composition/TerminalTail'
import { deliveryStoryMatrix } from './delivery-story-frame'
const fullOutput = fn()
const meta = { title: 'Fractal UI/Terminal Tail', component: TerminalTail, args: { output: { _tag: 'Complete', text: 'Compiling transcript\nWriting snapshots\nFinished' } }, render: args => deliveryStoryMatrix(() => <TerminalTail {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof TerminalTail>
export default meta
type Story = StoryObj<typeof meta>
export const ThreeLines: Story = { play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelectorAll('[data-terminal-line]')).toHaveLength(12)
  await expect(within(canvasElement).queryByRole('button', { name: 'Show recorded output' })).toBeNull()
  await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull()
} }
export const LongOutput: Story = { args: { output: { _tag: 'Complete', text: 'Starting\nLoading bindings\nChecking calls\nCompiling transcript\nWriting snapshots\nFinished' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.queryByText('Starting')).toBeNull()
  await expect(canvas.getAllByText('… 3 earlier lines')).toHaveLength(4)
  await userEvent.click(canvas.getAllByRole('button', { name: 'Show recorded output' })[0]!)
  await expect(canvas.getByText('Starting')).toBeVisible()
  await expect(canvas.getAllByRole('button', { name: 'Show recorded output' })).toHaveLength(3)
  await expect(canvas.getByText('Lines 1–6 of 6')).toBeVisible()
} }
export const LargeOutput: Story = { args: { output: { _tag: 'Complete', text: Array.from({ length: 10000 }, (_, index) => `recorded-line-${index}`).join('\n') } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelectorAll('[data-terminal-line]')).toHaveLength(12)
  await userEvent.click(canvas.getAllByRole('button', { name: 'Show recorded output' })[0]!)
  await expect(canvasElement.querySelectorAll('[data-terminal-line]')).toHaveLength(109)
  await expect(canvas.getByText('recorded-line-0')).toBeVisible()
  await expect(canvas.queryByText('recorded-line-100')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Next lines' }))
  await expect(canvas.getByText('recorded-line-100')).toBeVisible()
  await expect(canvas.queryByText('recorded-line-0')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-terminal-line]')).toHaveLength(109)
  await userEvent.click(canvas.getByRole('button', { name: 'Previous lines' }))
  await expect(canvas.getByText('recorded-line-0')).toBeVisible()
} }
export const Preview: Story = { args: { output: { _tag: 'Preview', text: 'Last lines\nRecorded preview\nFinished', earlierLines: 120, fullOutput: { _tag: 'Available', onPress: fullOutput } } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getAllByRole('button', { name: 'Full output' })[0]!)
  await expect(fullOutput).toHaveBeenCalled()
  await expect(canvas.queryByRole('button', { name: 'Show recorded output' })).toBeNull()
} }
export const FullOutputUnavailable: Story = { args: { output: { _tag: 'Preview', text: 'Recorded preview', earlierLines: 20, fullOutput: { _tag: 'Unavailable', reason: 'The full log was not retained.' } } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const button = canvas.getAllByRole('button', { name: 'Full output' })[0]!
  await expect(button).toHaveAttribute('aria-disabled', 'true')
  await expect(button).toHaveAccessibleDescription('The full log was not retained.')
  button.focus()
  await expect(button).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await expect(canvasElement.querySelectorAll('[data-terminal-line]')).toHaveLength(4)
  await expect(canvas.queryByRole('button', { name: 'Next lines' })).toBeNull()
} }
export const DiffNegativeControl: Story = { args: { output: { _tag: 'Complete', text: '- removed setting\n+ added setting\n✖ diagnostic text only' } }, play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelectorAll('pre code')).toHaveLength(4)
  await expect(canvasElement.querySelector('ul, [data-testid="markdown"]')).toBeNull()
  await expect(within(canvasElement).getAllByText('- removed setting')).toHaveLength(4)
  await expect(canvasElement.querySelectorAll('[data-tinted="true"]')).toHaveLength(4)
  await expect(canvasElement.querySelectorAll('[data-tinted="false"]')).toHaveLength(8)
} }
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <><TerminalTail output={meta.args.output} /><TerminalTail output={LongOutput.args!.output!} /><TerminalTail output={Preview.args!.output!} /><TerminalTail output={FullOutputUnavailable.args!.output!} /><TerminalTail output={DiffNegativeControl.args!.output!} /></>) }
