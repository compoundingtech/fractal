import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { waitFor } from 'storybook/test'
import { LiveAgentWorkspace } from '../../../../../apps/fractal-web/src/web/LiveAgentWorkspace.tsx'
import { assertScreenWorld, ScenarioScreen, screenSlices } from './scenario-data-source.story-harness.tsx'

interface WorkspaceScreenArgs { readonly scheme: 'dark' | 'light' }

// The workspace is client-only (its stores have no server snapshot), so a static render resolves to this boundary.
const WorkspaceScreen = ({ scheme }: WorkspaceScreenArgs) => <ScenarioScreen scheme={scheme}>{world => <React.Suspense fallback={null}><LiveAgentWorkspace initialRef={world.agentRef} persistGeometry={false} /></React.Suspense>}</ScenarioScreen>

const clientOnly = 'The static check cannot render the client-only workspace; the play proves the agent and thread text in the browser.'
const invariant = { roster: clientOnly, details: clientOnly, attention: clientOnly, conversation: clientOnly, terminal: clientOnly, sync: clientOnly } as const
const scenario = (world?: string) => ({ _tag: 'slices', slices: screenSlices, invariant, ...(world === undefined ? {} : { world }) }) as const

/** The real workspace over the selected scenario world. Populated and the failed tools state follow the Scenario toolbar. */
const meta = {
  id: 'screens-workspace',
  title: 'Screens/Workspace',
  component: WorkspaceScreen,
  args: { scheme: 'dark' },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, ...scenarioArgTypes(['roster', 'conversation', 'sync']) },
  parameters: { layout: 'fullscreen', scenario: scenario() },
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      if (canvasElement.querySelector('[data-testid="live-agent-workspace"]') === null) throw new Error('Production workspace missing')
      assertScreenWorld(canvasElement)
    }, { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
} satisfies Meta<typeof WorkspaceScreen>
export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {}
export const FailedTools: Story = { name: 'Failed tools', args: { conversation: 'failed-tools' } as Story['args'] }
export const Empty: Story = { parameters: { scenario: scenario('empty') } }
export const Loading: Story = { parameters: { scenario: scenario('loading') } }
export const Reconnecting: Story = { parameters: { scenario: scenario('offline-reconnect-storm') } }
export const LongThread: Story = { name: 'Long thread', tags: ['gate'], args: { conversation: 'long' } as Story['args'] }
