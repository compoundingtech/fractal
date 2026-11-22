/** A mission as its own tab: needs you, stuck, failed, working, done; loading, stale, missing from a truncated feed and read-only actions. Synthetic fixture missions only. */
import type * as Fractal from '@fractal/client/schema'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import { expect, fn, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { observed, waiting, type Feed } from '../data/source.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { casMigration, fixtureAttention, fixtureMissions, fixtureNow, flakeTriage, gatewaySchemaV1, sdkInterop, secretsRotation, webfractalFoundation } from './fixtures.ts'
import { MissionDetail } from './MissionDetailView.tsx'
import type { MissionActionPort, MissionActionResult } from './missionActions.ts'

const onOpen = fn()
const onAction = fn()

/** A port that answers after a short wait, so the pending state is visible. */
const port = (result: MissionActionResult, waitMs = 400): MissionActionPort => ({
  run: request => { onAction(request.offer.action, request.offer.target); return new Promise(resolve => setTimeout(() => resolve(result), waitMs)) },
})

/** The served presentation's offered actions, as Fractal attaches them to a mission row. */
const withActions = (mission: Fractal.Mission): Fractal.Mission => Object.assign({}, mission, {
  presentation: { actions: [
    { id: 'retry-step', action: 'work.retry', target: Option.getOrElse((mission.run_details ?? []).at(-1)!.steps, () => [])[0]?.id ?? `${mission.id}/step`, confirm: false },
    { id: 'cancel-run', action: 'mission.cancel', target: (mission.run_details ?? []).at(-1)!.id, confirm: true },
  ] },
}) as Fractal.Mission

/** One step of the latest run marked failed on its third attempt. */
const failedMission = (mission: Fractal.Mission): Fractal.Mission => {
  const runs = mission.run_details ?? []
  const last = runs.at(-1)!
  const steps = Option.getOrElse(last.steps, () => [])
  const target = steps.find(step => step.state !== 'completed') ?? steps[0]!
  const failed = Object.assign({}, last, { status: 'failed', steps: Option.some(steps.map(step => step === target ? Object.assign({}, step, { state: 'failed', attempt: 3 }) : step)) })
  return Object.assign({}, mission, { state: 'failed', run_details: [...runs.slice(0, -1), failed] }) as Fractal.Mission
}

const Surface = ({ missionRef, feed = observed({ value: fixtureMissions }), readOnly = false, actionPort }: { readonly missionRef: string; readonly feed?: Feed<readonly Fractal.Mission[]>; readonly readOnly?: boolean; readonly actionPort?: MissionActionPort }) =>
  <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>
    <div {...stylex.props(styles.tab)}>
      <MissionDetail missionRef={missionRef} visible onOpen={ref => onOpen(ref)} missions={feed} agents={[]} attention={fixtureAttention.filter(item => item.state === 'open')} readOnly={readOnly} now={fixtureNow} {...(actionPort === undefined ? {} : { actionPort })} />
    </div>
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Missions/Mission detail',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixture missions pin the header, banner, plan levels, activity and actions for each mission state and feed state.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Done:contract-fixture', 'Failed:contract-fixture', 'Loading:contract-fixture', 'ActionFails:contract-fixture', 'CancelConfirmed:contract-fixture', 'Missing:contract-fixture', 'NeedsYou:contract-fixture', 'ReadOnly:contract-fixture', 'RetryPending:contract-fixture', 'Stale:contract-fixture', 'Stuck:contract-fixture', 'Working:contract-fixture']

export const NeedsYou: Story = {
  args: { missionRef: webfractalFoundation.id },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { level: 1 })).not.toHaveTextContent(/[0-9a-f]{16,}/)
    const banner = canvasElement.querySelector('[data-mission-banner]')!
    await expect(banner).toHaveAttribute('data-mission-banner', 'needs-you')
    // The plan keeps dependency order: more than one level, parallel steps side by side.
    await expect(canvasElement.querySelectorAll('[data-plan-level]').length).toBeGreaterThan(1)
    await expect(canvas.getByRole('group', { name: 'Dependency graph' })).toBeVisible()
    await userEvent.click(canvas.getByRole('button', { name: 'View mission KDL' }))
    await expect(onOpen).toHaveBeenCalledWith(`declaration/${webfractalFoundation.id}`)
  },
}
export const Stuck: Story = {
  args: { missionRef: sdkInterop.id },
  play: async ({ canvasElement }) => {
    const banner = canvasElement.querySelector('[data-mission-banner]')
    await expect(banner === null ? 'none' : banner.getAttribute('data-mission-banner')).not.toBe('failed')
    // No step slug leaks into the banner.
    if (banner !== null) await expect(banner.textContent).not.toMatch(/step-run\//)
  },
}
export const Failed: Story = {
  args: { missionRef: flakeTriage.id, feed: observed({ value: fixtureMissions.map(mission => mission.id === flakeTriage.id ? failedMission(mission) : mission) }) },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-mission-banner]')).toHaveAttribute('data-mission-banner', 'failed')
    await expect(canvasElement.querySelector('[data-step-word="failed"]')).not.toBeNull()
    await expect(canvasElement.querySelector('[data-activity="failed"]')).not.toBeNull()
  },
}
export const Working: Story = { args: { missionRef: gatewaySchemaV1.id }, play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-step-word="working"] [data-live-ring]')).not.toBeNull() } }
export const Done: Story = { args: { missionRef: secretsRotation.id }, play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-mission-banner]')).toBeNull() } }
export const Loading: Story = { args: { missionRef: casMigration.id, feed: waiting }, play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-system-state="loading"]')).toHaveTextContent('Loading missions') } }
export const Stale: Story = {
  args: { missionRef: gatewaySchemaV1.id, feed: observed({ value: fixtureMissions, freshness: 'stale' }) },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-mission-detail]')).toHaveAttribute('data-stale', 'true')
    await expect(within(canvasElement).getByText(/^last observed/)).toBeVisible()
  },
}
export const Missing: Story = {
  args: { missionRef: 'mission/beyond/the-byte-bound', feed: observed({ value: fixtureMissions }) },
  play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-mission-missing]')).toHaveTextContent('not in the current missions feed') },
}
export const ReadOnly: Story = {
  args: { missionRef: casMigration.id, readOnly: true, feed: observed({ value: fixtureMissions.map(mission => mission.id === casMigration.id ? withActions(mission) : mission) }) },
  play: async ({ canvasElement }) => {
    for (const button of within(canvasElement).getAllByRole('button', { name: /^(Retry step|Cancel run)$/ })) await expect(button).toBeDisabled()
    await expect(within(canvasElement).getAllByText('This device is read-only').length).toBeGreaterThanOrEqual(2)
  },
}

const withCas = (mission: Fractal.Mission) => mission.id === casMigration.id ? withActions(mission) : mission
export const CancelConfirmed: Story = {
  args: { missionRef: casMigration.id, feed: observed({ value: fixtureMissions.map(withCas) }), actionPort: port({ _tag: 'Done' }) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    onAction.mockClear()
    // A destructive offer asks first and says what it does; nothing is sent until Confirm.
    await userEvent.click(canvas.getByRole('button', { name: 'Cancel run' }))
    const confirm = canvas.getByRole('group', { name: 'Confirm cancel run' })
    await expect(confirm).toHaveTextContent('cannot be resumed')
    await expect(onAction).not.toHaveBeenCalled()
    await userEvent.click(within(confirm).getByRole('button', { name: 'Keep it' }))
    await expect(canvas.queryByRole('group', { name: 'Confirm cancel run' })).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: 'Cancel run' }))
    await userEvent.click(within(canvas.getByRole('group', { name: 'Confirm cancel run' })).getByRole('button', { name: 'Cancel run' }))
    await expect(onAction).toHaveBeenCalledWith('mission.cancel', expect.stringMatching(/^mission-run\//))
    await expect(canvasElement.querySelector('[data-mission-action="cancel-run"]')).toHaveAttribute('data-mission-action-state', 'pending')
    await expect(await canvas.findByText(/^Sent\./)).toBeVisible()
  },
}
export const RetryPending: Story = {
  args: { missionRef: casMigration.id, feed: observed({ value: fixtureMissions.map(withCas) }), actionPort: port({ _tag: 'Done' }, 60_000) },
  play: async ({ canvasElement }) => {
    onAction.mockClear()
    const retry = within(canvasElement).getAllByRole('button', { name: 'Retry step' })[0]!
    await userEvent.click(retry)
    // Not destructive: it sends at once, then every copy of the offer reads pending and cannot send twice.
    await expect(onAction).toHaveBeenCalledTimes(1)
    for (const copy of canvasElement.querySelectorAll('[data-mission-action="retry-step"]')) await expect(copy).toHaveAttribute('data-mission-action-state', 'pending')
    await userEvent.click(canvasElement.querySelector<HTMLElement>('[data-mission-action="retry-step"]')!)
    await expect(onAction).toHaveBeenCalledTimes(1)
  },
}
export const ActionFails: Story = {
  args: { missionRef: casMigration.id, feed: observed({ value: fixtureMissions.map(withCas) }), actionPort: port({ _tag: 'Failed', message: 'Fractal changed while this was on its way, so it was not applied.' }, 50) },
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getAllByRole('button', { name: 'Retry step' })[0]!)
    const alerts = await within(canvasElement).findAllByRole('alert')
    await expect(alerts[0]).toHaveTextContent('was not applied')
    // The button is usable again after a failure.
    await expect(canvasElement.querySelector('[data-mission-action="retry-step"]')).toHaveAttribute('data-mission-action-state', 'failed')
    await expect(canvasElement.querySelector('[data-mission-action="retry-step"]')).not.toBeDisabled()
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
  tab: { display: 'flex', flexDirection: 'column', height: '100vh' },
})
