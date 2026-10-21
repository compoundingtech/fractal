// @vitest-environment jsdom
import * as React from 'react'
import { flushSync } from 'react-dom'
import { createRoot } from 'react-dom/client'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { ConnectionState } from '@fractal/sdk/effect'
import { useClockLabel } from '@fractal/ui/human-time/clock'
import { expect, it } from 'vitest'
import { fixtureSource } from './fixtureSource.ts'
import { DataSourceProvider, useClockDeadline, useNowSnapshot, useObservedClockState, usePausedNow } from './react.tsx'
import type { DataSource, NetworkReachability } from './source.ts'

const emptyWorld = { now: 0, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } as const

const mount = (source: DataSource, registry: AtomRegistry.AtomRegistry, node: React.ReactNode) => {
  const container = document.createElement('div')
  const root = createRoot(container)
  flushSync(() => root.render(<DataSourceProvider source={source} registry={registry}>{node}</DataSourceProvider>))
  return { container, unmount: () => { flushSync(() => root.unmount()); registry.dispose() } }
}

it('ticks only the leaf label and still publishes pause/resume to the shell', () => {
  const registry = AtomRegistry.make()
  const now = Atom.make(0)
  const connection = Atom.make<ConnectionState>({ _tag: 'Live' })
  const network = Atom.make<NetworkReachability>({ _tag: 'Online' })
  const source = { ...fixtureSource({ world: emptyWorld }), now, connection, network }
  let shellRenders = 0, leafRenders = 0
  const Leaf = ({ fixedNow }: { readonly fixedNow?: number }) => { leafRenders++; return <time>{useClockLabel(now => String(now), fixedNow)}</time> }
  const Shell = () => { shellRenders++; const state = useObservedClockState(); const paused = usePausedNow(); return <section data-state={state}><Leaf {...(paused === undefined ? {} : { fixedNow: paused })} /></section> }
  const page = mount(source, registry, <Shell />)
  try {
    for (let tick = 1; tick <= 600; tick++) flushSync(() => registry.set(now, tick * 1000))
    expect(shellRenders).toBe(1)
    expect(leafRenders).toBe(601)
    expect(page.container.querySelector('time')?.textContent).toBe('600000')
    flushSync(() => registry.set(network, { _tag: 'Offline' }))
    expect(page.container.querySelector('section')?.dataset.state).toBe('Paused')
    flushSync(() => registry.set(now, 700_000))
    expect(page.container.querySelector('time')?.textContent).toBe('600000')
    flushSync(() => registry.set(network, { _tag: 'Online' }))
    expect(page.container.querySelector('section')?.dataset.state).toBe('Live')
    expect(page.container.querySelector('time')?.textContent).toBe('700000')
  } finally { page.unmount() }
})

it('re-renders a snapshot panel only when its next visible change is reached', () => {
  const registry = AtomRegistry.make()
  const now = Atom.make(0)
  const source = { ...fixtureSource({ world: emptyWorld }), now }
  let panelRenders = 0
  const Panel = () => {
    panelRenders++
    const at = useNowSnapshot()
    // A minute-granular label: its next change is the following whole minute.
    useClockDeadline((Math.floor(at / 60_000) + 1) * 60_000)
    return <span data-testid="panel">{Math.floor(at / 60_000)}m</span>
  }
  const page = mount(source, registry, <Panel />)
  const text = () => page.container.querySelector('[data-testid="panel"]')?.textContent
  try {
    for (let tick = 1; tick < 60; tick++) flushSync(() => registry.set(now, tick * 1000))
    expect(panelRenders).toBe(1)
    expect(text()).toBe('0m')
    flushSync(() => registry.set(now, 60_000))
    expect(text()).toBe('1m')
    for (let tick = 61; tick <= 600; tick++) flushSync(() => registry.set(now, tick * 1000))
    expect(text()).toBe('10m')
    expect(panelRenders).toBe(11)
  } finally { page.unmount() }
})
