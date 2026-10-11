import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { BackgroundJobRow } from './assistant-ui/composition/BackgroundJobRow'
import { jobFixture } from './delivery-fixtures'
import { deliveryStoryMatrix } from './delivery-story-frame'
const meta = { title: 'Fractal UI/Background Job Row', component: BackgroundJobRow, args: { job: jobFixture }, render: args => deliveryStoryMatrix(() => <BackgroundJobRow {...args} />), parameters: { layout: 'fullscreen' } } satisfies Meta<typeof BackgroundJobRow>
export default meta
type Story = StoryObj<typeof meta>
const statePlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const rows = canvasElement.querySelectorAll<HTMLElement>('[data-testid="background-job"]')
  await expect(rows).toHaveLength(4)
  const state = args.job.state
  const exit = state._tag === 'Exited' && state.exit._tag === 'Reported' ? state.exit.code : undefined
  for (const row of rows) {
    const canvas = within(row)
    if (exit !== undefined) await expect(canvas.getByText(`exit ${exit}`)).toBeVisible()
    else await expect(row.querySelector('[data-testid="exit-chip"]')).toBeNull()
    if (state._tag === 'Ready') await expect(canvas.getByText('Ready')).toBeVisible()
    else await expect(canvas.queryByText('Ready')).toBeNull()
    if (state._tag === 'Monitor') await expect(canvas.getByText(state.event)).toBeVisible()
    if (state._tag === 'Aborted' || state._tag === 'Cancelled') await expect(canvas.getByText(state.reason)).toBeVisible()
    await expect(canvas.queryByRole('alert')).toBeNull()
    await expect(row.textContent?.toLowerCase()).not.toContain('unknown')
    const disclosure = canvas.getByRole('button', { name: `Output for ${args.job.id}` })
    const automaticallyOpen = state._tag === 'Aborted' || state._tag === 'Cancelled' || state._tag === 'Exited' && (state.exit._tag === 'NotReported' || state.exit.code !== 0)
    await expect(disclosure).toHaveAttribute('aria-expanded', automaticallyOpen || args.expanded ? 'true' : 'false')
  }
  const canvas = within(rows[0]!)
  if (!args.expanded) {
    const disclosure = canvas.getByRole('button', { name: `Output for ${args.job.id}` })
    const previous = disclosure.getAttribute('aria-expanded')
    disclosure.focus()
    await userEvent.keyboard('{Enter}')
    await expect(disclosure).toHaveFocus()
    await expect(disclosure).toHaveAttribute('aria-expanded', previous === 'true' ? 'false' : 'true')
  } else if (args.job.output._tag === 'Preview') {
    const full = canvas.getByRole('button', { name: 'Full output' })
    await expect(full).toHaveAccessibleDescription(args.job.output.fullOutput._tag === 'Unavailable' ? args.job.output.fullOutput.reason : '')
    full.focus()
    await expect(full).toHaveFocus()
  }
}
export const ExitZero: Story = { play: statePlay }
export const ExitNonzero: Story = { args: { job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 2 } } } }, play: statePlay }
export const NoReportedExit: Story = { args: { job: { ...jobFixture, state: { _tag: 'Exited', exit: { _tag: 'NotReported' } } } }, play: statePlay }
export const ServiceExited: Story = { args: { job: { ...jobFixture, kind: 'service', command: 'node scripts/dev-server.mjs' } }, play: statePlay }
export const ServiceReady: Story = { args: { job: { ...jobFixture, kind: 'service', command: 'node scripts/dev-server.mjs', state: { _tag: 'Ready' }, output: { _tag: 'Complete', text: 'Listening on port 4321' } } }, play: statePlay }
export const MonitorEvent: Story = { args: { job: { ...jobFixture, state: { _tag: 'Monitor', event: 'Port 4321 is responding' } } }, play: statePlay }
export const Cancelled: Story = { args: { job: { ...jobFixture, state: { _tag: 'Cancelled', reason: 'Cancelled by the parent.' } } }, play: statePlay }
export const Aborted: Story = { args: { job: { ...jobFixture, state: { _tag: 'Aborted', reason: 'The harness stopped this job.' } } }, play: statePlay }
export const OutputTruncated: Story = { args: { expanded: true, job: { ...jobFixture, output: { _tag: 'Preview', text: 'Last recorded output\nWrote transcript snapshots\nFinished', earlierLines: 600, fullOutput: { _tag: 'Unavailable', reason: 'The harness retained only this preview.' } } } }, play: statePlay }
export const LongCommand: Story = { args: { job: { ...jobFixture, command: 'env -u BASH_ENV -u ENV pnpm -C packages/fractal-web run integration-check -- --project transcript --output-directory artifacts/transcript-delivery-review' } }, play: async context => {
  await statePlay(context)
  const code = context.canvasElement.querySelector('code[title]')
  await expect(code).toHaveAttribute('title', context.args.job.command)
  await expect(code?.textContent).toBe(context.args.job.command)
} }
export const MountedStateTransition: Story = { args: { job: { ...jobFixture, state: { _tag: 'Ready' } } }, render: function Transition(args) {
  const [, updateArgs] = useArgs<React.ComponentProps<typeof BackgroundJobRow>>()
  return deliveryStoryMatrix(() => <><Button onPress={() => updateArgs({ job: { ...args.job, state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 2 } } } })}>Report nonzero exit</Button><Button onPress={() => updateArgs({ job: { ...args.job, state: { _tag: 'Aborted', reason: 'The mounted job was aborted.' } } })}>Report aborted job</Button><BackgroundJobRow {...args} /></>)
}, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelector('[data-testid="terminal-tail"]')).toBeNull()
  for (const label of ['Report nonzero exit', 'Report aborted job']) {
    await userEvent.click(canvas.getAllByRole('button', { name: label })[0]!)
    await waitFor(async () => { await expect(canvasElement.querySelectorAll('[data-testid="terminal-tail"]')).toHaveLength(4) })
    await userEvent.click(canvas.getAllByRole('button', { name: 'Output for bg_40' })[0]!)
    await expect(canvasElement.querySelectorAll('[data-testid="terminal-tail"]')).toHaveLength(3)
  }
} }
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
export const AllStates: Story = { render: () => deliveryStoryMatrix(() => <>{[ExitZero, ExitNonzero, NoReportedExit, ServiceExited, ServiceReady, MonitorEvent, Cancelled, Aborted, OutputTruncated, LongCommand].map((story, index) => <BackgroundJobRow key={index} job={story.args?.job ?? jobFixture} expanded={story.args?.expanded ?? false} />)}</>), play: async ({ canvasElement }) => {
  await expect(canvasElement.querySelectorAll('[data-testid="background-job"]')).toHaveLength(40)
  await expect(canvasElement.querySelectorAll('[data-testid="exit-chip"]')).toHaveLength(20)
  await expect(canvasElement.querySelectorAll('[data-testid="terminal-tail"]')).toHaveLength(20)
  await expect(within(canvasElement).queryByRole('alert')).toBeNull()
} }
export const MountedAbortKeepsOutputContext: Story = { args: { job: { ...jobFixture, state: { _tag: 'Ready' }, output: { _tag: 'Complete', text: Array.from({ length: 205 }, (_, index) => `job-line-${index}`).join('\n') } } }, render: function Transition(args) {
  const [, updateArgs] = useArgs<React.ComponentProps<typeof BackgroundJobRow>>()
  return deliveryStoryMatrix(() => <BackgroundJobRow {...args} job={{ ...args.job, startedBy: { label: 'bash call #9', onPress: () => updateArgs({ job: { ...args.job, state: { _tag: 'Aborted', reason: 'The service stopped.' } } }) } }} />)
}, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getAllByRole('button', { name: 'Output for bg_40' })[0]!)
  await userEvent.click(canvas.getByRole('button', { name: 'Show recorded output' }))
  await userEvent.click(canvas.getByRole('button', { name: 'Next lines' }))
  const origin = canvas.getAllByRole('button', { name: 'Started by bash call #9' })[0]!
  origin.focus()
  await userEvent.keyboard('{Enter}')
  await expect(origin).toHaveFocus()
  await expect(canvas.getByText('Lines 101–200 of 205')).toBeVisible()
  await expect(canvas.getByText('job-line-100')).toBeVisible()
  await expect(canvas.queryByText('job-line-0')).toBeNull()
  await waitFor(async () => { await expect(canvasElement.querySelectorAll('[data-testid="terminal-tail"]')).toHaveLength(4) })
} }
