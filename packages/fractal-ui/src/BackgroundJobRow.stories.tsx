import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { BackgroundJobRow } from './assistant-ui/composition/BackgroundJobRow'
import { jobFixture } from './delivery-fixtures'
import { deliveryStoryMatrix } from './delivery-story-frame'
const meta = { title: 'Fractal UI/Background Job Row', component: BackgroundJobRow, args: { job: jobFixture }, render: args => deliveryStoryMatrix(() => <BackgroundJobRow {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof BackgroundJobRow>
export default meta
type Story = StoryObj<typeof meta>
export const ExitZero: Story = {}
export const ExitNonzero: Story = { args: { job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 2 } } } } }
export const NoReportedExit: Story = { args: { job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'NotReported' } } } } }
export const ServiceExited: Story = { args: { job: { ...jobFixture, kind: 'service', command: 'node scripts/dev-server.mjs' } } }
export const ServiceReady: Story = { args: { job: { ...jobFixture, kind: 'service', command: 'node scripts/dev-server.mjs', state: { _tag: 'Ready' }, output: { _tag: 'Complete', text: 'Listening on port 4321' } } } }
export const MonitorEvent: Story = { args: { job: { ...jobFixture, state: { _tag: 'Monitor', event: 'Port 4321 is responding' } } } }
export const Cancelled: Story = { args: { job: { ...jobFixture, state: { _tag: 'Cancelled', reason: 'Cancelled by the parent.' } } } }
export const Aborted: Story = { args: { job: { ...jobFixture, state: { _tag: 'Aborted', reason: 'The harness stopped this job.' } } } }
export const OutputTruncated: Story = { args: { expanded: true, job: { ...jobFixture, output: { _tag: 'Preview', text: 'Last recorded output\nWrote transcript snapshots\nFinished', earlierLines: 600, fullOutput: { _tag: 'Unavailable', reason: 'The harness retained only this preview.' } } } } }
export const LongCommand: Story = { args: { job: { ...jobFixture, command: 'env -u BASH_ENV -u ENV pnpm -C packages/fractal-web run integration-check -- --project transcript --output-directory artifacts/transcript-delivery-review' } } }
export const Bg40NegativeControl: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getAllByText('exit 0')).toHaveLength(4)
  await expect(canvasElement.querySelector('[data-testid="terminal-tail"]')).toBeNull()
  await userEvent.click(canvas.getAllByRole('button', { name: 'Output for bg_40' })[0]!)
  await expect(canvasElement.querySelectorAll('[data-tinted="true"]')).toHaveLength(2)
  await expect(canvas.queryByRole('alert')).toBeNull()
  await expect(canvas.queryByText('exit 1')).toBeNull()
} }
export const PrintedRcNegativeControl: Story = { args: { job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'NotReported' } }, output: { _tag: 'Complete', text: 'echo rc=1\nrc=1' } } }, play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelector('[data-testid="exit-chip"]')).toBeNull()
  await expect(within(canvasElement).getAllByText('rc=1')).toHaveLength(4)
  await expect(canvasElement.textContent?.toLowerCase()).not.toContain('unknown')
} }
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <>{[ExitZero, ExitNonzero, NoReportedExit, ServiceExited, ServiceReady, MonitorEvent, Cancelled, Aborted, OutputTruncated, LongCommand].map((story, index) => <BackgroundJobRow key={index} job={story.args?.job ?? jobFixture} expanded={story.args?.expanded ?? false} />)}</>) }
