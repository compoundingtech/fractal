import { useAtom } from '@effect/atom-react'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import type { Meta, StoryObj } from '@storybook/react'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'

import { fixtureSource } from '../../data/fixtureSource.ts'
import { DataSourceProvider, useSubjectList } from '../../data/react.tsx'
import { fixtureProjections } from '../../fixtures/projections.ts'
import { SubjectAddress } from '../../resources/contract.ts'
import { AgentFolders } from '../AgentFolders.tsx'
import { WorkbenchContextProvider } from '../context.tsx'
import { dailyDriver } from '../fixtures/layouts.ts'
import { createWindow, projectWorkspaces, reduceWindow, type WindowAction } from '../workspaces.ts'
import { defaultFilters, fixtureSidebarState } from './state.ts'

const hosts = ['dev3', 'dev5', 'laptop24'] as const
const world = {
  ...fixtureProjections,
  agents: fixtureProjections.agents.slice(0, hosts.length).map((agent, index) =>
    decodeUnknownSync(Agent)({
      ...Schema.encodeSync(Agent)(agent),
      name: `Machine ${hosts[index]!} session`,
      host_id: `host/${hosts[index]!}`,
    }),
  ),
  attention: [],
}

const SidebarSurface = () => {
  const subjects = useSubjectList()
  const [windowAtom] = React.useState(() =>
    Atom.make(createWindow({ seed: dailyDriver, subjects, attention: [] })),
  )
  const [window, setWindow] = useAtom(windowAtom)
  const selection = React.useMemo(() => Atom.map(windowAtom, (current) => current.selected), [windowAtom])
  const dispatch = (action: WindowAction) =>
    setWindow((current) => reduceWindow({ window: current, action, subjects, attention: [] }))
  return (
    <WorkbenchContextProvider value={{
      subjects: new Map(subjects.map((subject) => [subject.ref, subject])),
      focusedRef: null,
      platform: 'other',
      open: (request) => dispatch({
        _tag: 'Layout',
        action: {
          _tag: 'Open',
          input: Schema.decodeSync(SubjectAddress)({ ref: request.ref, presentation: request.presentation ?? 'detail' }),
          ...(request.side === undefined ? {} : { side: request.side }),
        },
      }),
    }}>
      <div style={{ width: 340, maxWidth: '100%', minHeight: '100vh' }}>
        <AgentFolders
          workspaces={projectWorkspaces({ window, subjects, attention: [] })}
          selection={selection}
          dispatch={dispatch}
        />
      </div>
    </WorkbenchContextProvider>
  )
}

const MachineFilterSurface = () => {
  const [runtime] = React.useState(() => {
    const registry = AtomRegistry.make()
    const source = fixtureSource({ world })
    registry.set(fixtureSidebarState(source.gateway ?? source.mode).filters, defaultFilters)
    return { registry, source }
  })
  return <DataSourceProvider {...runtime}><SidebarSurface /></DataSourceProvider>
}

const meta = {
  title: 'Fractal/Shell',
  component: MachineFilterSurface,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof MachineFilterSurface>
export default meta
type Story = StoryObj<typeof meta>

export const MachineFilter: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = canvas.getByRole('textbox', { name: 'Search agents and folders' })
    const rows = () => [...canvasElement.querySelectorAll<HTMLElement>('[data-wf-agent-ref]')]
    const assertHosts = async (expected: readonly string[]) => {
      await waitFor(async () => {
        await expect(rows()).toHaveLength(expected.length)
        for (const host of hosts) {
          const matching = rows().filter((row) => row.textContent?.includes(`Machine ${host} session`))
          await expect(matching).toHaveLength(expected.includes(host) ? 1 : 0)
        }
      })
    }
    const search = async (query: string) => {
      await userEvent.clear(input)
      if (query !== '') await userEvent.type(input, query)
    }
    await expect(input).toHaveAccessibleDescription(/machine:.*host:/)
    await assertHosts(hosts)
    const initialCount = rows().length
    await search('machine:dev3')
    await assertHosts(['dev3'])
    await expect(rows().length).toBeLessThan(initialCount)
    await expect(canvas.getByRole('button', { name: 'Filter agents · 1 active' })).toBeInTheDocument()
    await expect(canvasElement.querySelectorAll('mark')).toHaveLength(0)
    await search('machine:dev3,dev5')
    await assertHosts(['dev3', 'dev5'])
    await search('HOST:DEV')
    await assertHosts(['dev3', 'dev5'])
    await search('machine:dev host:dev3')
    await assertHosts(['dev3'])
    await expect(canvas.getByRole('button', { name: 'Filter agents · 2 active' })).toBeInTheDocument()
    await search('dev5 machine:dev')
    await assertHosts(['dev5'])
    await search('machine:dev3')
    await userEvent.click(canvas.getByRole('button', { name: 'Filter agents · 1 active' }))
    const hostMenu = within(canvasElement.ownerDocument.body).getByRole('combobox', { name: 'Filter by host' })
    await userEvent.selectOptions(hostMenu, 'dev5')
    await assertHosts([])
    await userEvent.selectOptions(hostMenu, 'dev3')
    await assertHosts(['dev3'])
    await userEvent.selectOptions(hostMenu, '')
    await userEvent.keyboard('{Escape}')
    await search('machine:dev9')
    await assertHosts([])
    await expect(rows().length).not.toBe(initialCount)
    await search('machine:')
    await assertHosts(hosts)
    await expect(canvas.getByRole('button', { name: 'Filter agents' })).toBeInTheDocument()
    await search('machine:dev3,dev5')
    await assertHosts(['dev3', 'dev5'])
  },
}
