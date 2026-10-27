import type { ComponentProps } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { waitFor } from 'storybook/test'
import { AppStorySpecimen, type AppStoryArgs } from '../fixtures/AppStoryHarness.tsx'
import { assertProductionAppState } from '../fixtures/appStoryAssertions.ts'
import { appStoryAgent, appStoryStates } from '../fixtures/appStoryStates.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

/** The production workspace module, as the app mounts it, over one availability state of the fixture world. */
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/LiveAgentWorkspace',
  component: LiveAgentWorkspace,
  args: { state: 'populated', scheme: 'dark' },
  argTypes: {
    state: { control: 'select', options: appStoryStates },
    scheme: { control: 'radio', options: ['dark', 'light'] },
  },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The production LiveAgentWorkspace module, with its lazily loaded ConversationPane, over fixtureSource feeds of the deterministic fixture world pins loading, empty, populated, failed-tool, offline, unavailable and streaming repair states without a gateway.' } },
  // The workspace keeps its scheme, selected agent and panes in persisted UI state; every story starts from the same
  // state. A storage event makes the already-loaded persistence merge it, as it does for another tab's write.
  beforeEach: ({ args }) => {
    const agentRef = appStoryAgent(args.state)
    const entry = (value: unknown) => ({ version: 1, value, clock: Date.now(), writer: 'app-story' })
    const raw = JSON.stringify({ version: 1, entries: { 'round2.scheme': entry(args.scheme), 'round2.agent': entry(agentRef), 'round2.sidebarClosed': entry(false), [`round2.panes.${agentRef}`]: entry([]) } })
    window.localStorage.setItem('wf.ui@1', raw)
    window.dispatchEvent(new StorageEvent('storage', { key: 'wf.ui@1', newValue: raw }))
  },
  render: ({ state }) => <AppStorySpecimen state={state}>{() => <LiveAgentWorkspace />}</AppStorySpecimen>,
  play: async ({ canvasElement, args }) => {
    // On a cold dev server the lazily loaded pane takes longer than waitFor's default second.
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
} satisfies Meta<ComponentProps<typeof LiveAgentWorkspace> & AppStoryArgs>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Empty:contract-fixture',
  'EmptyLight:contract-fixture',
  'FailedTool:contract-fixture',
  'FailedToolLight:contract-fixture',
  'Loading:contract-fixture',
  'LoadingLight:contract-fixture',
  'Offline:contract-fixture',
  'OfflineLight:contract-fixture',
  'Populated:contract-fixture',
  'PopulatedLight:contract-fixture',
  'ScriptedRepair:contract-fixture',
  'ScriptedRepairLight:contract-fixture',
  'Unavailable:contract-fixture',
  'UnavailableLight:contract-fixture',
]
type Story = StoryObj<typeof meta>

export const Loading: Story = { args: { state: 'loading' } }
export const LoadingLight: Story = { args: { state: 'loading', scheme: 'light' } }
export const Empty: Story = { args: { state: 'empty' } }
export const EmptyLight: Story = { args: { state: 'empty', scheme: 'light' } }
export const Populated: Story = { args: { state: 'populated' } }
export const PopulatedLight: Story = { args: { state: 'populated', scheme: 'light' } }
export const FailedTool: Story = { args: { state: 'failed-tool' } }
export const FailedToolLight: Story = { args: { state: 'failed-tool', scheme: 'light' } }
export const Offline: Story = { args: { state: 'offline' } }
export const OfflineLight: Story = { args: { state: 'offline', scheme: 'light' } }
export const Unavailable: Story = { args: { state: 'unavailable' } }
export const UnavailableLight: Story = { args: { state: 'unavailable', scheme: 'light' } }
export const ScriptedRepair: Story = { args: { state: 'scripted-repair' } }
export const ScriptedRepairLight: Story = { args: { state: 'scripted-repair', scheme: 'light' } }
