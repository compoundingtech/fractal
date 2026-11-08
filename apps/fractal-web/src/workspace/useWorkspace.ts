import * as React from 'react'
import { RegistryContext } from '@effect/atom-react'
import { afterNextPaint } from '../telemetry/ux.ts'
import { useDataSource } from '../data/react.tsx'
import { activeTab, tabsOf, type Command, type WorkspaceState } from './model.ts'
import { policyOf } from './kinds.ts'
import { defaultBudget, desiredHolds, makeResourceManager, planTiers, type Budget, type ResourceStats, type Tier } from './resources.ts'
import type { DataResource } from './registry.ts'
import { createWorkspaceStore, localStorageLayer, memoryStorage, type WorkspaceStore } from './store.ts'
import { paneKey, parsePane, subjectHref, subjectOfLocation, subjectOfRef } from './subject.ts'

export interface WorkspaceNavigation {
  readonly getLocation: () => string
  readonly subscribe: (callback: () => void) => () => void
  readonly navigate: (href: string) => void
  /** Rewrites the current entry; an embed without one navigates. */
  readonly replace?: (href: string) => void
}

const disposal = new WeakMap<WorkspaceStore, number>()

/** Owns the machine's lifetime. The first render already shows what the URL asks for, with no effect-driven flash. */
export const useWorkspaceStore = ({ storageKey, location, initialRef }: {
  /** `undefined` keeps the workspace in memory, for embeds and stories. */
  readonly storageKey: string | undefined
  readonly location: string
  /** What an empty workspace with no subject in its URL starts on: a story's or an embed's known state. */
  readonly initialRef?: string | undefined
}): WorkspaceStore => {
  const [store] = React.useState(() => {
    const created = createWorkspaceStore(storageKey === undefined ? memoryStorage() : localStorageLayer(storageKey))
    const subject = subjectOfLocation(location)
    if (subject !== undefined) created.dispatch({ _tag: 'Open', pane: paneKey(subject) })
    else if (initialRef !== undefined && tabsOf(created.getState().layout).length === 0) created.dispatch({ _tag: 'Open', pane: paneKey(subjectOfRef(initialRef)) })
    return created
  })
  React.useEffect(() => {
    const flush = () => store.flush()
    window.addEventListener('pagehide', flush)
    // StrictMode replays this effect on one store, so disposal waits to see that nothing came back.
    const pending = disposal.get(store)
    if (pending !== undefined) window.clearTimeout(pending)
    return () => {
      window.removeEventListener('pagehide', flush)
      flush()
      disposal.set(store, window.setTimeout(() => store.dispose()))
    }
  }, [store])
  return store
}

export const useWorkspaceState = (store: WorkspaceStore): WorkspaceState =>
  React.useSyncExternalStore(store.subscribe, store.getState, store.getState)

/** What leaves history as it is: nothing about these is a place worth a Back stop. */
const keepsHistory = (command: Command) => command._tag === 'Close' || command._tag === 'CloseOthers' || command._tag === 'SetView' || command._tag === 'Move'

/**
 * Routing is a projection: the URL names the active subject, and a location with a subject opens or
 * focuses it. User activations push a history entry; close, restore and merges replace the current one.
 * A location change is applied before the state is written back, so neither direction can loop.
 *
 * This is the one rule for Back after a close: a history entry names a subject, and arriving at it opens
 * that subject, so Back reopens a closed tab.
 */
export const useWorkspaceRoute = ({ store, state, location, navigation }: {
  readonly store: WorkspaceStore
  readonly state: WorkspaceState
  readonly location: string
  readonly navigation: WorkspaceNavigation
}) => {
  const history = React.useRef<'push' | 'replace'>('replace')
  const applied = React.useRef<string | undefined>(undefined)
  const activePane = activeTab(state)?.pane
  React.useLayoutEffect(() => {
    if (applied.current !== undefined && applied.current !== location) {
      const subject = subjectOfLocation(location)
      if (subject !== undefined) store.dispatch({ _tag: 'Open', pane: paneKey(subject) })
      history.current = 'replace'
    }
    applied.current = location
    const current = activeTab(store.getState())?.pane
    const search = location.split('?')[1] ?? ''
    const params = new URLSearchParams(search)
    params.delete('open')
    const query = params.toString()
    const href = current === undefined ? `/${query === '' ? '' : `?${query}`}` : subjectHref(parsePane(current), search)
    const mode = history.current
    history.current = 'replace'
    if (href === location) return
    applied.current = href
    if (mode === 'push') navigation.navigate(href)
    else (navigation.replace ?? navigation.navigate)(href)
  }, [location, activePane, store, navigation])
  return React.useCallback((command: Command) => {
    if (!keepsHistory(command)) history.current = 'push'
    store.dispatch(command)
  }, [store])
}

export interface WorkspaceDiagnostics {
  readonly state: () => WorkspaceState
  readonly dispatch: WorkspaceStore['dispatch']
  readonly tiers: () => ReadonlyMap<string, Tier>
  readonly stats: () => ResourceStats
  readonly keys: () => readonly string[]
}

declare global {
  interface Window {
    __workspace?: WorkspaceDiagnostics
  }
}

/**
 * Plans tiers and reconciles demand in the commit that changed them, so a socket read overlaps pane
 * evaluation. Warm follows wait until after the first paint and idle: restoring twenty tabs must not
 * open sockets ahead of the visible one.
 */
export const useWorkspaceResources = ({ store, state, budget = defaultBudget }: {
  readonly store: WorkspaceStore
  readonly state: WorkspaceState
  readonly budget?: Budget
}): ReadonlyMap<string, Tier> => {
  const source = useDataSource()
  const registry = React.useContext(RegistryContext)
  const [warm, setWarm] = React.useState(false)
  React.useLayoutEffect(() => {
    let cancelIdle: (() => void) | undefined
    const cancelPaint = afterNextPaint(() => {
      if (typeof window.requestIdleCallback === 'function') {
        const idle = window.requestIdleCallback(() => setWarm(true), { timeout: 2000 })
        cancelIdle = () => window.cancelIdleCallback(idle)
      } else {
        const timer = window.setTimeout(() => setWarm(true), 0)
        cancelIdle = () => window.clearTimeout(timer)
      }
    })
    return () => { cancelPaint(); cancelIdle?.() }
  }, [])
  const tiers = React.useMemo(() => planTiers({ state, budget: warm ? budget : { dom: 0, data: 0 }, policyOf }), [state.layout, state.mru, state.views, budget, warm])
  const [manager] = React.useState(() => makeResourceManager({
    acquire: (resource: DataResource) => {
      if (resource._tag === 'Conversation') {
        const interest = source.conversationInterest?.(resource.ref)
        return interest === undefined ? undefined : registry.mount(interest)
      }
      // The terminal screen follows its own visibility and attach state; the other kinds read through
      // the source's shell-wide feeds. They are declared here so the budget and the counts see them.
      return undefined
    },
  }))
  React.useLayoutEffect(() => {
    // Hidden tabs hold no demand yet, as before tabs: a visible follow is never evicted, so a hidden one
    // could pin a gateway's only slot. The source may keep or drop their data; a reveal follows again.
    const visible = new Map([...desiredHolds({ tabs: tabsOf(state.layout), tiers, views: state.views, policyOf })].filter(([id]) => tiers.get(id) === 'active'))
    manager.reconcile(visible)
  }, [manager, tiers, state.layout, state.views])
  React.useEffect(() => () => manager.dispose(), [manager])
  React.useEffect(() => {
    if (!import.meta.env.DEV) return
    window.__workspace = { state: store.getState, dispatch: store.dispatch, tiers: () => tiers, stats: manager.stats, keys: manager.keys }
    return () => { delete window.__workspace }
  }, [store, tiers, manager])
  return tiers
}
