import type { FractalClient, OrderedMembershipOperation } from '@fractal/client'
import * as Atom from 'effect/reactivity/Atom'
import { generateKeyBetween } from 'fractional-indexing'
import { followPins, type PinsFollow, type PinsState } from './pins.ts'

export interface SidebarPins {
  readonly state: PinsState
  readonly editable: boolean
  readonly toggle: (ref: string) => void
  readonly retry: () => void
}
export const pinsEditable = (state: PinsState): boolean => state.phase === 'synced' && state.editable === true
export const planPinToggle = (state: PinsState, ref: string): OrderedMembershipOperation[] => {
  if (!pinsEditable(state)) return []
  if (state.members.some((item) => item.member === ref)) return [{ op: 'remove', member: ref }]
  const last = state.members.reduce<string | null>((key, item) => key === null || item.position.key > key ? item.position.key : key, null)
  return [{ op: 'place', member: ref, bucket: null, key: generateKeyBetween(last, null) }]
}

/** One follow per source-owned client; fixtures stay inert. Creation stays off. */
export const sidebarPins: (client?: FractalClient) => Atom.Atom<SidebarPins> = Atom.family((client: FractalClient | undefined) => Atom.make((get) => {
  let active = true
  let generation = 0
  let editing = false
  let follow: PinsFollow | undefined
  let initializing: AbortController | undefined
  let current: SidebarPins
  const publish = (state: PinsState) => {
    if (!active) return
    current = { state, editable: pinsEditable(state), toggle, retry }
    get.setSelf(current)
  }
  const failed = () => publish({ ...current.state, phase: 'unavailable', detail: 'Pins could not be updated. Try again.' })
  const start = async () => {
    if (!active || client === undefined) return
    const version = ++generation
    editing = false
    initializing?.abort()
    initializing = new AbortController()
    follow?.close()
    follow = undefined
    publish({ ...current.state, phase: 'connecting' })
    try {
      const next = await followPins({ gateway: client, signal: initializing.signal, onState: (state) => {
        if (!active || version !== generation) return
        publish(state.phase === 'unavailable' ? { ...state, detail: 'Pins are unavailable. Try again.' } : editing && state.phase === 'synced' ? { ...state, phase: 'pending' } : state)
      } })
      if (!active || version !== generation) next.close()
      else follow = next
    } catch { if (active && version === generation) failed() }
  }
  const retry = () => { void start() }
  const toggle = (ref: string) => {
    const session = follow
    if (!active || editing || client === undefined || session === undefined || !current.editable) return
    const operations = planPinToggle(current.state, ref)
    const version = generation
    editing = true
    publish({ ...current.state, phase: 'pending' })
    void (async () => {
      try {
        const observed = await client.capabilities()
        if (!active || version !== generation || current.state.phase !== 'pending') return
        if (!observed.value.capabilities.some((item) => item.id === 'control.arrangements' && item.state === 'granted')) { failed(); return }
        await session.edit({ id: `action/${crypto.randomUUID()}`, idempotency_key: crypto.randomUUID(), fence: { snapshot_id: observed.snapshot.id, subject_revisions: {} } }, operations)
        if (active && version === generation && current.state.phase === 'pending') publish({ ...current.state, phase: 'synced' })
      } catch { if (active && version === generation) failed() } finally {
        if (active && version === generation) {
          editing = false
        }
      }
    })()
  }
  get.addFinalizer(() => { active = false; generation++; initializing?.abort(); follow?.close() })
  current = { state: { phase: client === undefined ? 'synced' : 'connecting', members: [] }, editable: false, toggle, retry }
  if (client !== undefined) void Promise.resolve().then(start)
  return current
}))
