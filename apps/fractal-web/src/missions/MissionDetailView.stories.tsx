/** A mission as its own tab over a fixture source: overview, agents, the plan graph over the trace, the inspector, Problems and History, and the definition; with the pane's side panel; in each state, light and dark, at 1280 and 440. Synthetic fixture missions only. */
import type * as Fractal from '@fractal/client/schema'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { expect, fireEvent, fn, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { borderVars as border, surfaceVars as surface, textVars as ink } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, waiting, type DataSource, type Feed } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { MissionAside } from './MissionAside.tsx'
import { MissionDetailView } from './MissionDetailView.tsx'
import { missionDetailsFixture, syncAttention, syncDeclaration, syncMission, syncMissionRef, syncSecurityWork, syncStep, withServedHistory } from './detail/fixtures.ts'
import { casMigration, fixtureMissions, flakeTriage, gatewaySchemaV1, sdkInterop, secretsRotation } from './fixtures.ts'
import type { MissionActionPort, MissionActionResult } from './missionActions.ts'

const onOpen = fn()
const onAction = fn()

/** A port that answers after a short wait, so the pending state is visible. */
const port = (result: MissionActionResult, waitMs = 400): MissionActionPort => ({
  run: request => { onAction(request.offer.action, request.offer.target); return new Promise(resolve => setTimeout(() => resolve(result), waitMs)) },
})

const lastSteps = (mission: Fractal.Mission) => Option.getOrElse((mission.run_details ?? []).at(-1)!.steps, () => [])

/** The served presentation's offered actions, as Fractal attaches them to a mission row. */
const withActions = (mission: Fractal.Mission): Fractal.Mission => Object.assign({}, mission, {
  presentation: { actions: [
    { id: 'retry-step', action: 'work.retry', target: lastSteps(mission)[0]?.id ?? `${mission.id}/step`, confirm: false },
    { id: 'cancel-run', action: 'mission.cancel', target: (mission.run_details ?? []).at(-1)!.id, confirm: true },
  ] },
}) as Fractal.Mission

/** One step of the latest run marked failed on its third attempt. */
const failedMission = (mission: Fractal.Mission): Fractal.Mission => {
  const runs = mission.run_details ?? []
  const last = runs.at(-1)!
  const steps = lastSteps(mission)
  const target = steps.find(step => step.state !== 'completed') ?? steps[0]!
  const failed = Object.assign({}, last, { status: 'failed', steps: Option.some(steps.map(step => step === target ? Object.assign({}, step, { state: 'failed', attempt: 3 }) : step)) })
  return Object.assign({}, mission, { state: 'failed', run_details: [...runs.slice(0, -1), failed] }) as Fractal.Mission
}

/** The window row of a finished mission without its steps, as the collection window trims finished runs. */
const trimmed = (mission: Fractal.Mission): Fractal.Mission =>
  Object.assign({}, mission, { run_details: (mission.run_details ?? []).map(run => Object.assign({}, run, { steps: Option.none() })) }) as Fractal.Mission

const served = withServedHistory(syncMission)

type Args = {
  readonly missionRef: string
  readonly scheme?: 'dark' | 'light'
  /** `narrow` is a 440px pane. */
  readonly width?: 'wide' | 'narrow'
  /** What the missions window shows. */
  readonly missions?: Feed<readonly Fractal.Mission[]>
  /** What the mission detail read returns per mission; unset, every window row. */
  readonly detail?: Readonly<Record<string, Feed<Fractal.Mission>>>
  readonly spend?: 'observed' | 'waiting' | 'unavailable'
  readonly readOnly?: boolean
  readonly actionPort?: MissionActionPort
  /** Show the pane's side panel beside the tab. */
  readonly aside?: boolean
}

const allMissions = [...fixtureMissions, served]

const Surface = ({ missionRef, scheme = 'dark', width = 'wide', missions = observed({ value: allMissions }), detail, spend = 'observed', readOnly = false, actionPort, aside = false }: Args) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState<DataSource>(() => {
    const base = fixtureSource({
      world: {
        ...fixtureProjections,
        missions: allMissions,
        attention: [...fixtureProjections.attention, ...syncAttention],
        subjectReads: {
          work: { [syncStep('security')]: observed({ value: syncSecurityWork }) },
          missionDefinition: { [syncMissionRef]: observed({ value: syncDeclaration }) },
        },
      },
      overrides: {
        missions,
        grants: { actions: readOnly ? 'ungranted' : 'granted', messageSend: 'ungranted', attachmentUpload: 'ungranted', terminalInput: 'granted', terminalControl: 'granted' },
        ...(detail === undefined ? {} : { subjectReads: { mission: detail } }),
      },
    })
    return { ...base, nativeDetails: missionDetailsFixture(spend), ...(actionPort === undefined ? {} : { missionActions: actionPort }) }
  })
  return <DataSourceProvider source={source} registry={registry}>
    <main data-scheme={scheme} {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
      <div data-story-pane={width} {...stylex.props(styles.pane, width === 'narrow' && styles.narrow)}>
        <div {...stylex.props(styles.tab)}><MissionDetailView missionRef={missionRef} visible onOpen={ref => onOpen(ref)} /></div>
        {aside ? <aside aria-label="Side panel" {...stylex.props(styles.aside)}>
          <MissionAside subject={{ _tag: 'Mission', ref: missionRef }} width="dock" scheme={scheme} onOpen={ref => onOpen(ref)} onClose={() => undefined} />
        </aside> : null}
      </div>
    </main>
  </DataSourceProvider>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Missions/Mission detail',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixture missions pin the overview, graph, trace, inspector, side panel and definition for each mission and feed state.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'ActionFails:contract-fixture', 'Brush:contract-fixture', 'CancelConfirmed:contract-fixture', 'CoarseHistory:contract-fixture', 'Definition:contract-fixture',
  'Done:contract-fixture', 'Failed:contract-fixture', 'FinishedRunFromDetailRead:contract-fixture', 'HoverFollows:contract-fixture', 'LinkedHover:contract-fixture',
  'Loading:contract-fixture', 'Missing:contract-fixture', 'Narrow:contract-fixture', 'NarrowLight:contract-fixture', 'NeedsYou:contract-fixture', 'NeedsYouLight:contract-fixture',
  'ReadOnly:contract-fixture', 'RetryPending:contract-fixture', 'Scrub:contract-fixture', 'SidePanel:contract-fixture', 'SidePanelLight:contract-fixture',
  'SpendUnavailable:contract-fixture', 'Stale:contract-fixture', 'Stuck:contract-fixture', 'Working:contract-fixture',
]

const ready = async (canvasElement: HTMLElement) => {
  await waitFor(() => expect(canvasElement.querySelector('[data-mission-detail]')).not.toBeNull())
  return within(canvasElement)
}
const row = (canvasElement: HTMLElement, path: string) => canvasElement.querySelector<HTMLElement>(`[data-trace-row="${syncStep(path)}"]`)!
const inspected = (canvasElement: HTMLElement) => canvasElement.querySelector('[data-inspector-step]')

/** No horizontal overflow outside the trace's own scroller. */
const fitsWidth = async (canvasElement: HTMLElement) => {
  const article = canvasElement.querySelector<HTMLElement>('[data-mission-detail]')!
  await expect(article.scrollWidth).toBeLessThanOrEqual(article.clientWidth + 1)
}

const needsYouPlay: Story['play'] = async ({ canvasElement }) => {
  const canvas = await ready(canvasElement)
  await expect(canvas.getByRole('heading', { level: 1 })).toHaveTextContent('Ship sync v2')
  await expect(canvasElement.querySelector('[data-mission-banner]')).toHaveAttribute('data-mission-banner', 'needs-you')
  await expect(canvasElement.querySelector('[data-mission-spend]')).toHaveTextContent(/^\$\d+ · [\d.]+[kM]? tokens$/)
  await expect(canvas.getByRole('application', { name: /^Plan, 12 steps/ })).toBeVisible()
  await expect(canvas.getByRole('grid', { name: 'Trace of Ship sync v2' })).toBeVisible()
  await expect(canvas.getByRole('slider', { name: 'Replay time' })).toHaveAttribute('aria-valuetext', expect.stringMatching(/^Now/))
  // Who and what it cost on every row's second line; the wait on the person in amber only where there was one.
  await expect(row(canvasElement, 'server').querySelector('[data-trace-line]')).toHaveTextContent('WorkbenchShell · ran 5h 30m · $11 · 3 tries')
  await expect(row(canvasElement, 'security').querySelector('[data-trace-you]')).toHaveTextContent('waited on you 5h')
  await expect(row(canvasElement, 'server').querySelector('[data-trace-you]')).toBeNull()
  // Every span carries its duration, three attempts on the server step.
  await expect(row(canvasElement, 'server').querySelectorAll('[data-trace-span]')).toHaveLength(3)
  await expect(row(canvasElement, 'spec').querySelector('[data-trace-duration]')).toHaveTextContent('40m')
  // Nothing selected yet: the inspector shows the step that needs the person, with the answer already given.
  await expect(inspected(canvasElement)).toHaveAttribute('data-inspector-step', syncStep('security'))
  await expect(canvasElement.querySelector('[data-inspector-answers]')).toHaveTextContent('Hold the canary')
}

export const NeedsYou: Story = { args: { missionRef: syncMissionRef }, play: needsYouPlay }
export const NeedsYouLight: Story = { args: { missionRef: syncMissionRef, scheme: 'light' }, play: needsYouPlay }

const narrowPlay: Story['play'] = async ({ canvasElement }) => {
  await ready(canvasElement)
  await fitsWidth(canvasElement)
  // The trace scrolls inside itself; the graph fits the pane.
  const scroller = canvasElement.querySelector<HTMLElement>('[data-trace] [role="grid"]')!.parentElement!
  await expect(scroller.scrollWidth).toBeGreaterThanOrEqual(scroller.clientWidth)
  const graph = canvasElement.querySelector<HTMLElement>('[data-plan-graph]')!
  await expect(graph.getBoundingClientRect().width).toBeLessThanOrEqual(440)
}
export const Narrow: Story = { args: { missionRef: syncMissionRef, width: 'narrow' }, play: narrowPlay }
export const NarrowLight: Story = { args: { missionRef: syncMissionRef, width: 'narrow', scheme: 'light' }, play: narrowPlay }

export const CoarseHistory: Story = {
  args: { missionRef: syncMissionRef, missions: observed({ value: [...fixtureMissions, syncMission] }), detail: { [syncMissionRef]: observed({ value: syncMission }) } },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    // Today's daemon serves the selected attempt only: one bar, and the inspector says what is missing.
    await expect(row(canvasElement, 'server').querySelectorAll('[data-trace-span]')).toHaveLength(1)
    await userEvent.click(within(row(canvasElement, 'server')).getByRole('button'))
    await expect(inspected(canvasElement)).toHaveAttribute('data-inspector-step', syncStep('server'))
    await expect(canvasElement.querySelector('[data-inspector]')).toHaveTextContent('Fractal serves only the latest of 3 attempts so far.')
    await userEvent.unhover(row(canvasElement, 'server'))
  },
}

export const LinkedHover: Story = {
  args: { missionRef: syncMissionRef },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    await userEvent.hover(row(canvasElement, 'docs'))
    // The docs step's lineage stays lit in the trace and the graph; everything else dims.
    await waitFor(() => expect(row(canvasElement, 'web')).toHaveAttribute('data-dimmed', 'true'))
    await expect(row(canvasElement, 'sdk')).not.toHaveAttribute('data-dimmed')
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('web')}"]`)).toHaveAttribute('data-dimmed', 'true')
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('schema')}"]`)).not.toHaveAttribute('data-dimmed')
    await userEvent.unhover(row(canvasElement, 'docs'))
    await waitFor(() => expect(canvasElement.querySelector('[data-trace-row][data-dimmed]')).toBeNull())
  },
}

export const HoverFollows: Story = {
  args: { missionRef: syncMissionRef },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    await userEvent.click(within(row(canvasElement, 'sdk')).getByRole('button'))
    // The pointer still rests on the picked row; once it leaves, the pick holds.
    await userEvent.unhover(row(canvasElement, 'sdk'))
    await waitFor(() => expect(inspected(canvasElement)).toHaveAttribute('data-inspector-source', 'selected'))
    // Hovering a span or a node drives the inspector, with no hover card; leaving snaps back to the pick.
    await userEvent.hover(row(canvasElement, 'load'))
    await waitFor(() => expect(inspected(canvasElement)).toHaveAttribute('data-inspector-step', syncStep('load')))
    await expect(inspected(canvasElement)).toHaveAttribute('data-inspector-source', 'hovered')
    await expect(canvasElement.ownerDocument.querySelector('[role="dialog"]')).toBeNull()
    await userEvent.unhover(row(canvasElement, 'load'))
    await userEvent.hover(canvasElement.querySelector<HTMLElement>(`[data-dag-node="${syncStep('migration')}"]`)!)
    await waitFor(() => expect(inspected(canvasElement)).toHaveAttribute('data-inspector-step', syncStep('migration')))
    await userEvent.unhover(canvasElement.querySelector<HTMLElement>(`[data-dag-node="${syncStep('migration')}"]`)!)
    await waitFor(() => expect(inspected(canvasElement)).toHaveAttribute('data-inspector-step', syncStep('sdk')))
  },
}

export const Brush: Story = {
  args: { missionRef: syncMissionRef },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    const rows = canvasElement.querySelector<HTMLElement>('[data-trace-rows]')!
    const header = row(canvasElement, 'spec').querySelector<HTMLElement>('[role="rowheader"]')!.getBoundingClientRect()
    const cell = row(canvasElement, 'spec').querySelector<HTMLElement>('[role="gridcell"]')!
    const left = header.right + 1
    const y = cell.getBoundingClientRect().top + 10
    // Dragging across the rows (not the axis) brushes the start of the run: steps that did not run then dim.
    fireEvent.pointerDown(cell, { button: 0, buttons: 1, pointerId: 1, clientX: left, clientY: y })
    fireEvent.pointerMove(rows, { buttons: 1, pointerId: 1, clientX: left + 40, clientY: y })
    fireEvent.pointerUp(rows, { button: 0, pointerId: 1, clientX: left + 40, clientY: y })
    await waitFor(() => expect(canvasElement.querySelector('[data-trace-brush]')).not.toBeNull())
    await expect(canvasElement.querySelector('[data-trace-brush-label]')).toBeVisible()
    await expect(row(canvasElement, 'spec')).not.toHaveAttribute('data-dimmed')
    await expect(row(canvasElement, 'web')).toHaveAttribute('data-dimmed', 'true')
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('web')}"]`)).toHaveAttribute('data-dimmed', 'true')
    // The playhead did not move: brushing and scrubbing never mix.
    await expect(canvasElement.querySelector('[data-replay-cursor]')).toBeNull()
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Clear time range' }))
    await expect(canvasElement.querySelector('[data-trace-brush]')).toBeNull()
  },
}

export const Scrub: Story = {
  args: { missionRef: syncMissionRef },
  play: async ({ canvasElement }) => {
    const canvas = await ready(canvasElement)
    const slider = canvas.getByRole('slider', { name: 'Replay time' })
    slider.focus()
    await userEvent.keyboard('{Home}')
    // At the run's start the graph replays: only the first step can have begun.
    await expect(canvasElement.querySelector('[data-replay-cursor]')).not.toBeNull()
    await expect(canvasElement.querySelector('[data-graph-replaying]')).not.toBeNull()
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('web')}"]`)).toHaveAttribute('data-state', 'pending')
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('spec')}"]`)).toHaveAttribute('data-state', 'running')
    // Clicking the axis scrubs there (about four hours in, while the server step's first attempt ran); End returns to now.
    const axis = canvasElement.querySelector<HTMLElement>('[data-trace-axis]')!.getBoundingClientRect()
    fireEvent.pointerDown(canvasElement.querySelector('[data-trace-axis]')!, { button: 0, buttons: 1, pointerId: 1, clientX: axis.left + axis.width * 0.12, clientY: axis.top + 4 })
    fireEvent.pointerUp(canvasElement.querySelector('[data-trace-axis]')!, { button: 0, pointerId: 1 })
    await expect(slider).toHaveAttribute('data-trace-playhead', 'replay')
    await expect(canvasElement.querySelector(`[data-dag-node="${syncStep('server')}"]`)).toHaveAttribute('data-state', 'running')
    slider.focus()
    await userEvent.keyboard('{End}')
    await expect(slider).toHaveAttribute('data-trace-playhead', 'now')
    await expect(canvasElement.querySelector('[data-replay-cursor]')).toBeNull()
  },
}

export const Definition: Story = {
  args: { missionRef: syncMissionRef },
  play: async ({ canvasElement }) => {
    const canvas = await ready(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: 'Definition' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-mission-definition]')).toHaveTextContent('mission "platform/sync-v2"'))
    // The TypeScript source has its place, saying what is missing rather than showing nothing.
    await expect(canvasElement.querySelector('[data-definition-source]')).toHaveAttribute('data-definition-source', 'unknown')
  },
}

const sidePanelPlay: Story['play'] = async ({ canvasElement }) => {
  await ready(canvasElement)
  const panel = within(canvasElement.querySelector<HTMLElement>('aside')!)
  await waitFor(() => expect(panel.getByRole('tab', { name: 'Details' })).toBeVisible())
  // The panel shows the inspector, so the tab drops its own.
  await expect(canvasElement.querySelector('[data-inspector-inline]')).toBeNull()
  await userEvent.hover(row(canvasElement, 'docs'))
  await waitFor(() => expect(canvasElement.querySelector('aside [data-inspector-step]')).toHaveAttribute('data-inspector-step', syncStep('docs')))
  await userEvent.unhover(row(canvasElement, 'docs'))
  await userEvent.click(panel.getByRole('tab', { name: /^Plan/ }))
  await userEvent.click(panel.getByRole('button', { name: /Load test at 3× peak/ }))
  await expect(row(canvasElement, 'load')).toHaveAttribute('data-selected', 'true')
  await userEvent.click(panel.getByRole('tab', { name: /^Agents/ }))
  await userEvent.click(panel.getByRole('button', { name: /^WorkbenchShell/ }))
  await expect(onOpen).toHaveBeenCalledWith('agent/build-host-a/workbench-shell')
  await userEvent.click(panel.getByRole('tab', { name: 'Details' }))
}
export const SidePanel: Story = { args: { missionRef: syncMissionRef, aside: true }, play: sidePanelPlay }
export const SidePanelLight: Story = { args: { missionRef: syncMissionRef, aside: true, scheme: 'light' }, play: sidePanelPlay }

export const FinishedRunFromDetailRead: Story = {
  args: { missionRef: secretsRotation.id, missions: observed({ value: fixtureMissions.map(mission => mission.id === secretsRotation.id ? trimmed(mission) : mission) }) },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    // The window trims a finished run's steps; the detail read keeps them.
    await expect(canvasElement.querySelectorAll('[data-trace-row]').length).toBeGreaterThan(0)
    await expect(canvasElement.querySelector('[data-mission-banner]')).toBeNull()
    await expect(canvasElement.querySelector('[data-mission-time]')).toHaveTextContent(/^took /)
  },
}
export const SpendUnavailable: Story = {
  args: { missionRef: syncMissionRef, spend: 'unavailable' },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    // Unknown, never $0.
    await expect(canvasElement.querySelector('[data-mission-spend]')).toHaveTextContent('Spend not reported')
    await expect(row(canvasElement, 'server').querySelector('[data-trace-line]')).not.toHaveTextContent('$')
    await expect(canvasElement.querySelector('[data-inspector-spend]')).toHaveTextContent('Not reported here')
  },
}
export const Stuck: Story = {
  args: { missionRef: sdkInterop.id },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    const banner = canvasElement.querySelector('[data-mission-banner]')
    await expect(banner === null ? 'none' : banner.getAttribute('data-mission-banner')).not.toBe('failed')
    if (banner !== null) await expect(banner.textContent).not.toMatch(/step-run\//)
  },
}
const failedFeed = observed({ value: fixtureMissions.map(mission => mission.id === flakeTriage.id ? failedMission(mission) : mission) })
export const Failed: Story = {
  args: { missionRef: flakeTriage.id, missions: failedFeed, detail: { [flakeTriage.id]: observed({ value: failedMission(flakeTriage) }) } },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    await expect(canvasElement.querySelector('[data-mission-banner]')).toHaveAttribute('data-mission-banner', 'failed')
    await expect(canvasElement.querySelector('[data-trace-row][data-state="failed"]')).not.toBeNull()
    await expect(canvasElement.querySelector('[data-problem="failure"]')).not.toBeNull()
  },
}
export const Working: Story = {
  args: { missionRef: gatewaySchemaV1.id },
  play: async ({ canvasElement }) => {
    await ready(canvasElement)
    await expect(canvasElement.querySelector('[data-trace-row][data-state="running"]')).not.toBeNull()
    await expect(canvasElement.querySelector('[data-mission-banner]')).toBeNull()
  },
}
export const Done: Story = { args: { missionRef: secretsRotation.id }, play: async ({ canvasElement }) => { await ready(canvasElement); await expect(canvasElement.querySelector('[data-mission-banner]')).toBeNull() } }
export const Loading: Story = {
  args: { missionRef: casMigration.id, missions: waiting, detail: { [casMigration.id]: waiting } },
  play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-system-state="loading"]')).toHaveTextContent('Loading mission') },
}
export const Stale: Story = {
  args: { missionRef: gatewaySchemaV1.id, missions: observed({ value: fixtureMissions, freshness: 'stale' }), detail: { [gatewaySchemaV1.id]: observed({ value: gatewaySchemaV1, freshness: 'stale' }) } },
  play: async ({ canvasElement }) => {
    const canvas = await ready(canvasElement)
    await expect(canvasElement.querySelector('[data-mission-detail]')).toHaveAttribute('data-stale', 'true')
    await expect(canvas.getByText(/^last observed/)).toBeVisible()
  },
}
export const Missing: Story = {
  args: { missionRef: 'mission/beyond/the-byte-bound' },
  play: async ({ canvasElement }) => { await waitFor(() => expect(canvasElement.querySelector('[data-mission-missing]')).toHaveTextContent('Fractal does not know this mission')) },
}

const withCas = (mission: Fractal.Mission) => mission.id === casMigration.id ? withActions(mission) : mission
const casFeeds = { missions: observed({ value: fixtureMissions.map(withCas) }), detail: { [casMigration.id]: observed({ value: withActions(casMigration) }) } }
const firstCasStep = lastSteps(casMigration)[0]!.id as string
const pickFirstCasStep = async (canvasElement: HTMLElement) => {
  await ready(canvasElement)
  await userEvent.click(within(canvasElement.querySelector<HTMLElement>(`[data-trace-row="${firstCasStep}"]`)!).getByRole('button'))
}
export const ReadOnly: Story = {
  args: { missionRef: casMigration.id, readOnly: true, ...casFeeds },
  play: async ({ canvasElement }) => {
    await pickFirstCasStep(canvasElement)
    for (const button of within(canvasElement).getAllByRole('button', { name: /^(Retry step|Cancel run)$/ })) await expect(button).toBeDisabled()
    await expect(within(canvasElement).getAllByText('This device is read-only').length).toBeGreaterThanOrEqual(2)
  },
}
export const CancelConfirmed: Story = {
  args: { missionRef: casMigration.id, ...casFeeds, actionPort: port({ _tag: 'Done' }) },
  play: async ({ canvasElement }) => {
    const canvas = await ready(canvasElement)
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
  args: { missionRef: casMigration.id, ...casFeeds, actionPort: port({ _tag: 'Done' }, 60_000) },
  play: async ({ canvasElement }) => {
    onAction.mockClear()
    await pickFirstCasStep(canvasElement)
    await userEvent.click(within(canvasElement).getAllByRole('button', { name: 'Retry step' })[0]!)
    // Not destructive: it sends at once, then every copy of the offer reads pending and cannot send twice.
    await expect(onAction).toHaveBeenCalledTimes(1)
    for (const copy of canvasElement.querySelectorAll('[data-mission-action="retry-step"]')) await expect(copy).toHaveAttribute('data-mission-action-state', 'pending')
    await userEvent.click(canvasElement.querySelector<HTMLElement>('[data-mission-action="retry-step"]')!)
    await expect(onAction).toHaveBeenCalledTimes(1)
  },
}
export const ActionFails: Story = {
  args: { missionRef: casMigration.id, ...casFeeds, actionPort: port({ _tag: 'Failed', message: 'Fractal changed while this was on its way, so it was not applied.' }, 50) },
  play: async ({ canvasElement }) => {
    await pickFirstCasStep(canvasElement)
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
  pane: { display: 'flex', height: '100vh', minWidth: 0 },
  narrow: { width: '440px', borderInlineEndWidth: '1px', borderInlineEndStyle: 'solid', borderInlineEndColor: border.border },
  tab: { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minWidth: 0, height: '100vh' },
  aside: { flex: '0 0 360px', minWidth: 0, height: '100vh', borderInlineStartWidth: '1px', borderInlineStartStyle: 'solid', borderInlineStartColor: border.border },
})
