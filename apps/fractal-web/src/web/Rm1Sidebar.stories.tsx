import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, waitFor } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Option from 'effect/Option'
import * as DateTime from 'effect/DateTime'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

/** Real application navigation, with deterministic observations rather than a story-only sidebar. */
function AppSidebar() {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => fixtureSource({ world: {
    ...fixtureProjections,
    agents: fixtureProjections.agents.map((agent, index) => ({
      ...agent,
      host_id: Option.some(index % 2 === 0 ? 'host/build-1' as const : 'host/build-2' as const),
      checkout: Option.some({ repository: 'example/product', base: 'main', branch: 'sidebar' }),
      ...(index === 0 ? { subagents: [{
        id: 'reviewer', driver: 'codex', description: Option.some('Review the sidebar rows'),
        session_id: Option.none(), started_at: Option.some(DateTime.makeUnsafe(fixtureProjections.now - 120_000)),
        subagent_type: Option.some('reviewer'), work_id: Option.none(),
        lease_expires_at: DateTime.makeUnsafe(fixtureProjections.now + 600_000),
      }] } : {}),
    })),
  } }))
  return <DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>
}
const meta = { title: 'Fractal/App/Shell/RM1 sidebar', component: AppSidebar, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof AppSidebar>
export default meta
type Story = StoryObj<typeof meta>
export const Sidebar: Story = {
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-milestone-row]').length).toBeGreaterThan(0))
    const row = canvasElement.querySelector('[data-milestone-row]')!
    expect(row.querySelector('[data-row-column="title"]')).not.toBeNull()
    expect(row.querySelector('[data-row-column="line2"]')).not.toBeNull()
    expect(row.getBoundingClientRect().height).toBe(48)
  },
}
export const AllStates: Story = Sidebar
