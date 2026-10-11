import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { HarnessDeliveryGroup } from './assistant-ui/composition/HarnessDeliveryGroup'
import { deliveryFixture, resultFixture, jobFixture } from './delivery-fixtures'
import { deliveryStoryMatrix } from './delivery-story-frame'
import type { HarnessDeliveryData } from './assistant-ui/composition/delivery-model'
import { deliveryWeightExamples } from './delivery-weight-examples'
const allOk: HarnessDeliveryData = { id: 'delivery-eight', time: '15:15', children: Array.from({ length: 8 }, (_, index) => ({ _tag: 'Agent', result: { ...resultFixture, id: `agent-${index}`, name: `Review ${index + 1}` } })) }
const meta = { title: 'Fractal UI/Harness Delivery Group', component: HarnessDeliveryGroup, args: { delivery: deliveryFixture }, render: args => deliveryStoryMatrix(() => <HarnessDeliveryGroup {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof HarnessDeliveryGroup>
export default meta
type Story = StoryObj<typeof meta>
const statePlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const groups = canvasElement.querySelectorAll<HTMLElement>('[data-testid="delivery-group"]')
  await expect(groups).toHaveLength(4)
  for (const group of groups) {
    const canvas = within(group)
    await expect(canvas.getByText(`${args.delivery.children.length} background results`)).toBeVisible()
    for (const child of args.delivery.children) {
      if (child._tag === 'Agent') {
        const card = Array.from(group.querySelectorAll<HTMLElement>('[data-testid="agent-result"]')).find(element => element.textContent?.includes(child.result.name))!
        const failed = ['Failed', 'Cancelled', 'Aborted'].includes(child.result.state._tag)
        await expect(within(card).getByRole('button', { name: failed ? 'Hide result' : 'Show result' })).toHaveAttribute('aria-expanded', failed ? 'true' : 'false')
      } else {
        const row = group.querySelector(`[data-job-id="${child.job.id}"]`)!
        const state = child.job.state
        if (state._tag === 'Exited' && state.exit._tag === 'Reported') await expect(within(row as HTMLElement).getByText(`exit ${state.exit.code}`)).toBeVisible()
        else await expect(row.querySelector('[data-testid="exit-chip"]')).toBeNull()
        await expect(within(row as HTMLElement).getByRole('button', { name: `Output for ${child.job.id}` })).toHaveAttribute('aria-expanded', 'true')
      }
    }
  }
  const first = within(groups[0]!)
  await userEvent.click(first.getByRole('button', { name: 'Expand all' }))
  for (const button of first.getAllByRole('button', { name: 'Hide result' })) await expect(button).toHaveAttribute('aria-expanded', 'true')
  await expect(first.queryByRole('button', { name: 'Show result' })).toBeNull()
}
export const FiveJobMixed: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getAllByText('5 background results')).toHaveLength(4)
  await expect(canvas.getAllByText('4 agents · 1 command')).toHaveLength(4)
  await expect(canvas.queryByText('4 agents · 1 commands')).toBeNull()
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
export const CancelledFailedAborted: Story = { args: { delivery: { id: 'delivery-stopped', children: [deliveryFixture.children[1]!, deliveryFixture.children[2]!, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Aborted', reason: 'The harness aborted the process.' } } }] } }, play: statePlay }
export const AllFailed: Story = { args: { delivery: { id: 'delivery-failed', children: [{ _tag: 'Agent', result: { ...resultFixture, state: { _tag: 'Failed', error: 'Child exited before submission.', exitCode: 1 } } }, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 2 } } } }] } }, play: statePlay }
export const PreviewTruncatedChild: Story = { args: { delivery: { ...deliveryFixture, children: [{ _tag: 'Agent', result: { ...resultFixture, report: { _tag: 'Preview', text: 'Recorded report preview.', earlierLines: 200, fullOutput: { _tag: 'Unavailable', reason: 'Only a preview was retained.' } } } }, { _tag: 'Job', job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'NotReported' } }, output: { _tag: 'Preview', text: 'Recorded tail.', earlierLines: 600, fullOutput: { _tag: 'Unavailable', reason: 'Only a preview was retained.' } } } }] } }, play: statePlay }
export const SingleJobNegativeControl: Story = { args: { delivery: { id: 'delivery-single', children: [{ _tag: 'Agent', result: resultFixture }] } }, play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelector('[data-testid="delivery-group"]')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-testid="agent-result"]')).toHaveLength(4)
  await expect(within(canvasElement).queryByRole('button', { name: 'Expand all' })).toBeNull()
} }
export const SingleCommand: Story = { args: { delivery: { id: 'delivery-command', children: [{ _tag: 'Job', job: jobFixture }] } }, play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelector('[data-testid="delivery-group"]')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-testid="background-job"]')).toHaveLength(4)
  await expect(within(canvasElement).getAllByText('exit 0')).toHaveLength(4)
  await expect(within(canvasElement).queryByRole('button', { name: 'Copy delivery id' })).toBeNull()
} }
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <>{[FiveJobMixed, AllOkEightCollapsed, CancelledFailedAborted, AllFailed, PreviewTruncatedChild, SingleJobNegativeControl, SingleCommand].map((story, index) => <HarnessDeliveryGroup key={index} delivery={story.args?.delivery ?? deliveryFixture} />)}</>), play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelectorAll('[data-testid="delivery-group"]')).toHaveLength(20)
  await expect(canvasElement.querySelectorAll('[data-testid="background-job"]')).toHaveLength(20)
  await expect(canvasElement.querySelectorAll('[data-result-state="Failed"] [data-testid="markdown"]')).toHaveLength(12)
  await expect(within(canvasElement).queryByRole('alert')).toBeNull()
} }
export const WeightSpecimens: Story = { render: () => deliveryStoryMatrix(() => <>{deliveryWeightExamples.map(example => <section key={example.name} data-weight-class={example.weight}><h3>{example.name} · {example.weight}</h3>{example.render()}</section>)}</>), play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelectorAll('[data-weight-class="card"]')).toHaveLength(8)
  await expect(canvasElement.querySelectorAll('[data-weight-class="row"]')).toHaveLength(8)
  await expect(canvasElement.querySelectorAll('[data-testid="delivery-group"]')).toHaveLength(4)
  for (const tail of canvasElement.querySelectorAll('[data-testid="terminal-tail"]')) await expect(tail.querySelector('[data-testid="markdown"], ul')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-tinted="true"]')).toHaveLength(8)
} }
