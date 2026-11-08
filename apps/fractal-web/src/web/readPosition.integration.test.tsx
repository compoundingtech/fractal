// @vitest-environment jsdom
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import * as React from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { UnreadItem } from '../conversation/unread.ts'
import { DataSourceProvider } from '../data/react.tsx'
import type { Agent, DataSource } from '../data/source.ts'
import * as StateCell from '../state/cell/StateCell.ts'
import { memoryOrigin } from '../state/cell/testing.ts'
import { useReadPosition, useUnreadMarks } from './readPosition.ts'

const answer = (id: string, ms: number): UnreadItem => ({ _tag: 'Text', id, role: 'assistant', at: new Date(ms).toISOString() })
const prompt = (id: string, ms: number): UnreadItem => ({ _tag: 'Text', id, role: 'user', at: new Date(ms).toISOString() })
const agent = { ref: 'agent/scout', activity: 'idle', lastActivityAt: { _tag: 'Unknown' } } as unknown as Agent

let root: Root
let container: HTMLDivElement
let uninstall: () => Promise<void>
let registry: AtomRegistry.AtomRegistry
const source = { mode: 'fixtures', gateway: 'gw', now: Atom.make(0) } as unknown as DataSource
let latest: ReturnType<typeof useReadPosition> = {}
let marks: ReadonlyMap<string, number | 'some'> = new Map()

function Pane(props: { readonly items?: readonly UnreadItem[]; readonly visible: boolean; readonly sendKey?: string }) {
  latest = useReadPosition({ conversation: 'agent/scout', items: props.items, visible: props.visible, enabled: true, ...(props.sendKey === undefined ? {} : { sendKey: props.sendKey }) })
  return null
}
function Tabs({ agents }: { readonly agents: readonly Agent[] }) {
  marks = useUnreadMarks(agents)
  return null
}
const agents = [agent]
const render = async (pane: React.ComponentProps<typeof Pane> | undefined) => {
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>
    {pane === undefined ? null : <Pane {...pane} />}
    <Tabs agents={agents} />
  </DataSourceProvider>))
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  uninstall = StateCell.install(StateCell.layer(memoryOrigin().window().environment, { debounceMs: 5 }))
  registry = AtomRegistry.make()
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  await uninstall()
  vi.unstubAllGlobals()
})

it('starts at the tail on the first look, so nothing in a conversation never opened is new', async () => {
  await render({ items: [prompt('p1', 10), answer('a1', 20)], visible: true })
  expect(latest.unreadFrom).toBeUndefined()
  expect(marks.get('agent/scout')).toBeUndefined()
})

it('counts what arrives while away, and places the divider there when the person comes back', async () => {
  await render({ items: [answer('a1', 20)], visible: true })
  await render({ items: [answer('a1', 20), answer('a2', 30), answer('a3', 40)], visible: false })
  expect(marks.get('agent/scout')).toBe(2)
  await render({ items: [answer('a1', 20), answer('a2', 30), answer('a3', 40)], visible: true })
  expect(latest.unreadFrom).toBe('a2')
  // Reading moves the position, but the divider stays put while the person is still here.
  await act(async () => latest.onSeen?.(['a2', 'a3']))
  expect(marks.get('agent/scout')).toBeUndefined()
  expect(latest.unreadFrom).toBe('a2')
  // Leaving and coming back places it afresh.
  await render({ items: [answer('a1', 20), answer('a2', 30), answer('a3', 40)], visible: false })
  await render({ items: [answer('a1', 20), answer('a2', 30), answer('a3', 40)], visible: true })
  expect(latest.unreadFrom).toBeUndefined()
})

it('clears the divider and the count on a send', async () => {
  await render({ items: [answer('a1', 20)], visible: true })
  await render({ items: [answer('a1', 20), answer('a2', 30)], visible: false })
  await render({ items: [answer('a1', 20), answer('a2', 30)], visible: true })
  expect(latest.unreadFrom).toBe('a2')
  await render({ items: [answer('a1', 20), answer('a2', 30), prompt('p2', 35)], visible: true, sendKey: 'send-1' })
  expect(latest.unreadFrom).toBeUndefined()
  expect(marks.get('agent/scout')).toBeUndefined()
})

it('shows a dot for a closed conversation whose roster reports newer prose', async () => {
  await render({ items: [answer('a1', 20)], visible: true })
  await render(undefined)
  agents[0] = { ...agent, lastProseAt: { _tag: 'Known', value: 90 } } as Agent
  await render(undefined)
  expect(marks.get('agent/scout')).toBe('some')
  agents[0] = agent
})
