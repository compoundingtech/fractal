import type { ComponentProps } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { waitFor } from 'storybook/test'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { AppStoryState, prepareAppStory, type AppStoryArgs } from '../fixtures/AppStoryHarness.tsx'
import { assertProductionAppState } from '../fixtures/appStoryAssertions.ts'

const meta = {
  title: 'Fractal/App/LiveAgentWorkspace',
  component: LiveAgentWorkspace,
  parameters: { layout: 'fullscreen' },
  args: { state: 'populated', scheme: 'dark', width: 1440 },
  beforeEach: ({ args }) => prepareAppStory(args.scheme),
  render: args => <AppStoryState {...args} renderSurface={fixture => <LiveAgentWorkspace />} />,
  play: async ({ canvasElement, args }) => {
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 15000 })
    canvasElement.dataset.appPlay = 'passed'
  },
} satisfies Meta<ComponentProps<typeof LiveAgentWorkspace> & AppStoryArgs>
export default meta
type Story = StoryObj<typeof meta>

export const Loading: Story = { args: { state: 'loading', scheme: 'dark', width: 1440 } }
export const LoadingLight: Story = { args: { state: 'loading', scheme: 'light', width: 1440 } }
export const Loading600: Story = { args: { state: 'loading', scheme: 'dark', width: 600 } }
export const LoadingLight600: Story = { args: { state: 'loading', scheme: 'light', width: 600 } }
export const Empty: Story = { args: { state: 'empty', scheme: 'dark', width: 1440 } }
export const EmptyLight: Story = { args: { state: 'empty', scheme: 'light', width: 1440 } }
export const Empty600: Story = { args: { state: 'empty', scheme: 'dark', width: 600 } }
export const EmptyLight600: Story = { args: { state: 'empty', scheme: 'light', width: 600 } }
export const Populated: Story = { args: { state: 'populated', scheme: 'dark', width: 1440 } }
export const PopulatedLight: Story = { args: { state: 'populated', scheme: 'light', width: 1440 } }
export const Populated600: Story = { args: { state: 'populated', scheme: 'dark', width: 600 } }
export const PopulatedLight600: Story = { args: { state: 'populated', scheme: 'light', width: 600 } }
export const FailedTool: Story = { args: { state: 'failed-tool', scheme: 'dark', width: 1440 } }
export const FailedToolLight: Story = { args: { state: 'failed-tool', scheme: 'light', width: 1440 } }
export const FailedTool600: Story = { args: { state: 'failed-tool', scheme: 'dark', width: 600 } }
export const FailedToolLight600: Story = { args: { state: 'failed-tool', scheme: 'light', width: 600 } }
export const Offline: Story = { args: { state: 'offline', scheme: 'dark', width: 1440 } }
export const OfflineLight: Story = { args: { state: 'offline', scheme: 'light', width: 1440 } }
export const Offline600: Story = { args: { state: 'offline', scheme: 'dark', width: 600 } }
export const OfflineLight600: Story = { args: { state: 'offline', scheme: 'light', width: 600 } }
export const Unavailable: Story = { args: { state: 'unavailable', scheme: 'dark', width: 1440 } }
export const UnavailableLight: Story = { args: { state: 'unavailable', scheme: 'light', width: 1440 } }
export const Unavailable600: Story = { args: { state: 'unavailable', scheme: 'dark', width: 600 } }
export const UnavailableLight600: Story = { args: { state: 'unavailable', scheme: 'light', width: 600 } }
export const ScriptedRepair: Story = { args: { state: 'scripted-repair', scheme: 'dark', width: 1440 } }
export const ScriptedRepairLight: Story = { args: { state: 'scripted-repair', scheme: 'light', width: 1440 } }
export const ScriptedRepair600: Story = { args: { state: 'scripted-repair', scheme: 'dark', width: 600 } }
export const ScriptedRepairLight600: Story = { args: { state: 'scripted-repair', scheme: 'light', width: 600 } }
