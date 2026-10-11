import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, within } from 'storybook/test'
import { AgentResultCard } from './assistant-ui/composition/AgentResultCard'
import { resultFixture } from './delivery-fixtures'
import { deliveryStoryMatrix } from './delivery-story-frame'
const openThread = fn()
const meta = { title: 'Fractal UI/Agent Result Card', component: AgentResultCard, args: { result: resultFixture }, render: args => deliveryStoryMatrix(() => <AgentResultCard {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof AgentResultCard>
export default meta
type Story = StoryObj<typeof meta>
export const Completed: Story = {}
export const Cancelled: Story = { args: { result: { ...resultFixture, state: { _tag: 'Cancelled', reason: 'The parent cancelled this assignment.' }, lifecycle: { _tag: 'Aborted' } } } }
export const Aborted: Story = { args: { result: { ...resultFixture, state: { _tag: 'Aborted', reason: 'The harness aborted the child.' }, lifecycle: { _tag: 'Aborted' } } } }
export const Failed: Story = { args: { result: { ...resultFixture, state: { _tag: 'Failed', error: 'Type checking exited before a result could be submitted.', exitCode: 2 } } } }
export const SchemaInvalid: Story = { args: { result: { ...resultFixture, schema: { _tag: 'Invalid', reason: 'The report is missing the required checks field.' } } } }
export const PreviewTruncated: Story = { args: { expanded: true, result: { ...resultFixture, report: { _tag: 'Preview', text: '## Review preview\nThe recorded result ends here.', earlierLines: 128, fullOutput: { _tag: 'Unavailable', reason: 'Full output was not retained by the harness.' } } } } }
export const Running: Story = { args: { result: { ...resultFixture, state: { _tag: 'Running' }, lifecycle: { _tag: 'Isolated' }, excerpt: 'Reviewing the remaining transcript paths.' } } }
export const StoppedResumable: Story = { args: { result: { ...resultFixture, lifecycle: { _tag: 'Stopped', resumable: true } } } }
export const Stopped: Story = { args: { result: { ...resultFixture, lifecycle: { _tag: 'Stopped', resumable: false } } } }
export const RanIsolated: Story = { args: { result: { ...resultFixture, lifecycle: { _tag: 'Isolated' } } } }
export const ResultSubmitted: Story = { args: { result: { ...resultFixture, lifecycle: { _tag: 'Submitted' } } } }
export const MarkdownOnly: Story = { args: { expanded: true, result: { ...resultFixture, facts: [], schema: { _tag: 'NotProvided' } } } }
export const OpenThreadUnavailable: Story = {}
export const OpenThread: Story = { args: { result: { ...resultFixture, openThread: { _tag: 'Available', onPress: openThread } } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getAllByRole('button', { name: 'Open thread' })[0]!)
  await expect(openThread).toHaveBeenCalled()
  await expect(canvas.getAllByRole('button', { name: 'Message' })[0]).toBeDisabled()
  await expect(canvas.getAllByRole('button', { name: 'Resume' })[0]).toBeDisabled()
  await expect(canvas.getAllByRole('button', { name: 'Open thread' })[0]).not.toBeDisabled()
} }
export const RawJsonTree: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelector('[aria-label="Raw result JSON"]')).toBeNull()
  await userEvent.click(canvas.getAllByRole('button', { name: 'Show result' })[0]!)
  await expect(canvasElement.querySelector('[data-testid="markdown"]')).not.toBeNull()
  await userEvent.click(canvas.getAllByRole('button', { name: 'Show raw JSON' })[0]!)
  await userEvent.click(canvas.getByRole('button', { name: /result \{4\}/ }))
  await expect(canvas.getByText('commit: "a41d90b"')).toBeVisible()
  await expect(canvas.queryByText('typecheck: "passed"')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: /checks \{2\}/ }))
  await expect(canvas.getByText('typecheck: "passed"')).toBeVisible()
} }
export const ProseFailureNegativeControl: Story = { args: { expanded: true }, play: async ({ canvasElement }) => {
  const cards = canvasElement.querySelectorAll('[data-testid="agent-result"]')
  await expect(cards).toHaveLength(4)
  for (const card of cards) { await expect(card).toHaveAttribute('data-result-state', 'Completed'); await expect(within(card as HTMLElement).queryByText('Failed')).toBeNull() }
  await expect(canvasElement.textContent).toContain('failed check')
  await expect(canvasElement.querySelector('[role="alert"]')).toBeNull()
} }
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <>{[Completed, Cancelled, Aborted, Failed, SchemaInvalid, PreviewTruncated, Running, StoppedResumable, Stopped, RanIsolated, ResultSubmitted, MarkdownOnly].map((story, index) => <AgentResultCard key={index} result={story.args?.result ?? resultFixture} expanded={story.args?.expanded ?? false} />)}</>) }
