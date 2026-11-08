import type { ComponentProps } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { waitFor } from 'storybook/test'
import { AppStorySpecimen, type AppStoryArgs } from '../fixtures/AppStoryHarness.tsx'
import { assertProductionAppState } from '../fixtures/appStoryAssertions.ts'
import { appStoryAgent, appStoryStates } from '../fixtures/appStoryStates.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { writeCellsLikeAnotherWindow } from '../state/cell/testing.ts'

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
  // state. Storage events make the already-running cell store adopt it, as it does another window's write.
  beforeEach: ({ args }) => {
    const agentRef = appStoryAgent(args.state)
    writeCellsLikeAnotherWindow({
      'round2.scheme': { value: args.scheme },
      'round2.agent': { value: agentRef },
      'panels.leftSidebar': { value: { open: true, ratio: 256 / 1440 } },
      [`round2.panes.${agentRef}`]: { value: [] },
    })
  },
  render: ({ state }) => <AppStorySpecimen state={state}>{() => <LiveAgentWorkspace initialRef={appStoryAgent(state)} persistGeometry={false} />}</AppStorySpecimen>,
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
  'FailedTool:contract-fixture',
  'Loading:contract-fixture',
  'Offline:contract-fixture',
  'Populated:contract-fixture',
  'ScriptedRepair:contract-fixture',
  'Unavailable:contract-fixture',
]
type Story = StoryObj<typeof meta>

// One story per state; the scheme control switches light and dark. The kit play gate runs each in one
// dark desktop case (APP_CASE_OVERRIDES in packages/fractal-ui/scripts/kit-play-commit-set.mjs).
export const Loading: Story = { args: { state: 'loading' } }
export const Empty: Story = { args: { state: 'empty' } }
export const Populated: Story = { args: { state: 'populated' } }
export const FailedTool: Story = { args: { state: 'failed-tool' } }
export const Offline: Story = { args: { state: 'offline' } }
export const Unavailable: Story = { args: { state: 'unavailable' } }
export const ScriptedRepair: Story = { args: { state: 'scripted-repair' } }
