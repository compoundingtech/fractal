import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { HarnessDeliveryGroup } from './assistant-ui/composition/HarnessDeliveryGroup'
import { deliveryFixture, resultFixture, jobFixture } from './delivery-fixtures'
import { deliveryStoryMatrix } from './delivery-story-frame'
import type { HarnessDeliveryData } from './assistant-ui/composition/delivery-model'
const allOk: HarnessDeliveryData = { id: 'delivery-eight', time: '15:15', children: Array.from({ length: 8 }, (_, index) => ({ _tag: 'Agent', result: { ...resultFixture, id: `agent-${index}`, name: `Review ${index + 1}` } })) }
const meta = { title: 'Fractal UI/Harness Delivery Group', component: HarnessDeliveryGroup, args: { delivery: deliveryFixture }, render: args => deliveryStoryMatrix(() => <HarnessDeliveryGroup {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof HarnessDeliveryGroup>
export default meta
type Story = StoryObj<typeof meta>
export const FiveJobMixed: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getAllByText('5 background results')).toHaveLength(4)
  await expect(canvas.getAllByRole('button', { name: 'Show result' })).toHaveLength(8)
  await expect(canvas.getAllByRole('button', { name: 'Hide result' })).toHaveLength(8)
  await expect(canvasElement.querySelectorAll('[data-result-state="Failed"] [data-testid="markdown"]')).toHaveLength(4)
  await expect(canvasElement.querySelectorAll('[data-result-state="Completed"] [data-testid="markdown"]')).toHaveLength(0)
} }
export const AllOkEightCollapsed: Story = { args: { delivery: allOk }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull()
  await userEvent.click(canvas.getAllByRole('button', { name: 'Expand all' })[0]!)
  await expect(canvasElement.querySelectorAll('[data-testid="markdown"]')).toHaveLength(8)
  await expect(canvas.getAllByRole('button', { name: 'Expand all' })).toHaveLength(3)
  await userEvent.click(canvas.getByRole('button', { name: 'Collapse successful results' }))
  await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull()
} }
export const CancelledFailedAborted: Story = { args: { delivery: { id: 'delivery-stopped', children: [deliveryFixture.children[1]!, deliveryFixture.children[2]!, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Aborted', reason: 'The harness aborted the process.' } } }] } } }
export const AllFailed: Story = { args: { delivery: { id: 'delivery-failed', children: [{ _tag: 'Agent', result: { ...resultFixture, state: { _tag: 'Failed', error: 'Child exited before submission.', exitCode: 1 } } }, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 2 } } } }] } } }
export const PreviewTruncatedChild: Story = { args: { delivery: { ...deliveryFixture, children: [{ _tag: 'Agent', result: { ...resultFixture, report: { _tag: 'Preview', text: 'Recorded report preview.', earlierLines: 200, fullOutput: { _tag: 'Unavailable', reason: 'Only a preview was retained.' } } } }, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'NotReported' } }, output: { _tag: 'Preview', text: 'Recorded tail.', earlierLines: 600, fullOutput: { _tag: 'Unavailable', reason: 'Only a preview was retained.' } } } }] } } }
export const SingleJobNegativeControl: Story = { args: { delivery: { id: 'delivery-single', children: [{ _tag: 'Agent', result: resultFixture }] } }, play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelector('[data-testid="delivery-group"]')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-testid="agent-result"]')).toHaveLength(4)
  await expect(within(canvasElement).queryByRole('button', { name: 'Expand all' })).toBeNull()
} }
export const SingleCommand: Story = { args: { delivery: { id: 'delivery-command', children: [{ _tag: 'Job', job: jobFixture }] } } }
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <>{[FiveJobMixed, AllOkEightCollapsed, CancelledFailedAborted, AllFailed, PreviewTruncatedChild, SingleJobNegativeControl, SingleCommand].map((story, index) => <HarnessDeliveryGroup key={index} delivery={story.args?.delivery ?? deliveryFixture} />)}</>) }
