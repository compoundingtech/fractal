// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { OutboxRecoveryPort } from '../data/source.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

import { OutboxRecovery } from './ConversationStates.tsx'

type Items = readonly { readonly id: string; readonly status: string; readonly actions: readonly ['discard'] }[]
const DAMAGED = 'storage damaged; delivery unknown'
const damaged = (id: string) => ({ id, status: DAMAGED, actions: ['discard'] as const })

/** A port over an in-memory list; `fail` makes the next lists reject as unavailable storage. */
const memoryPort = (initial: Items) => {
  let items = initial
  const state = { fail: false }
  const port = {
    list: vi.fn(async () => { if (state.fail) throw new Error('storage unavailable'); return items }),
    discard: vi.fn(async (id: string) => { items = items.filter(item => item.id !== id) }),
  } satisfies OutboxRecoveryPort
  return { port, state }
}

let host: HTMLElement
let root: Root
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

const mount = (port: OutboxRecoveryPort) => act(async () => root.render(<OutboxRecovery port={port} />))
const buttons = () => [...host.querySelectorAll<HTMLButtonElement>('button')]
const press = (label: string) => act(async () => {
  const target = buttons().find(button => button.textContent === label)
  if (target === undefined) throw new Error(`no ${label}`)
  target.click()
})

describe('OutboxRecovery', () => {
  it('lists a damaged item with its exact status and Discard as its only action', async () => {
    const { port } = memoryPort([damaged('outbox/one')])
    await mount(port)
    const row = host.querySelector('[data-outbox-item="outbox/one"]')
    expect(row?.querySelector('[data-outbox-status]')?.textContent).toBe(DAMAGED)
    expect(buttons().map(button => button.textContent)).toEqual(['Discard'])
    for (const absent of ['Copy', 'Send', 'Retry', 'Resend']) expect(host.textContent).not.toContain(absent)
    // The button's accessible name carries the row's status.
    const discard = buttons()[0]!
    const named = (discard.getAttribute('aria-labelledby') ?? '').split(' ').map(id => document.getElementById(id)?.textContent).join(' ')
    expect(named).toBe(`Discard ${DAMAGED}`)
  })

  it('discards the pressed item, lists again and drops its row', async () => {
    const { port } = memoryPort([damaged('outbox/one'), damaged('outbox/two')])
    await mount(port)
    expect(host.querySelectorAll('[data-outbox-item]')).toHaveLength(2)
    await act(async () => host.querySelector<HTMLButtonElement>('[data-outbox-item="outbox/one"] button')!.click())
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one')
    expect(port.list).toHaveBeenCalledTimes(2)
    expect([...host.querySelectorAll('[data-outbox-item]')].map(row => row.getAttribute('data-outbox-item'))).toEqual(['outbox/two'])
    await press('Discard')
    expect(host.querySelector('[data-outbox-recovery]')).toBeNull()
  })

  it('renders nothing for an empty outbox', async () => {
    const { port } = memoryPort([])
    await mount(port)
    expect(host.innerHTML).toBe('')
  })

  it('says storage is unavailable when listing fails, never damaged, and Retry lists again', async () => {
    const { port, state } = memoryPort([damaged('outbox/one')])
    state.fail = true
    await mount(port)
    const notice = host.querySelector('[data-outbox-recovery="unavailable"]')
    expect(notice?.querySelector('[role="status"]')?.textContent).toBe('Outbox on this device is unavailable; retry later.')
    expect(host.textContent?.toLowerCase()).not.toMatch(/damaged|corrupt/)
    expect(buttons().map(button => button.textContent)).toEqual(['Retry'])
    expect(port.discard).not.toHaveBeenCalled()
    // Still unavailable: the notice stays and nothing is called damaged.
    await press('Retry')
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(host.querySelector('[data-outbox-recovery="unavailable"]')).not.toBeNull()
    state.fail = false
    await press('Retry')
    expect(port.list).toHaveBeenCalledTimes(3)
    expect(host.querySelector('[data-outbox-recovery="unavailable"]')).toBeNull()
    expect(host.querySelector('[data-outbox-status]')?.textContent).toBe(DAMAGED)
  })

  it('keeps the row and says why when Discard is refused', async () => {
    const { port } = memoryPort([damaged('outbox/one')])
    await mount(port)
    port.discard.mockRejectedValueOnce(new Error('storage unavailable'))
    await press('Discard')
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(host.querySelector('[data-outbox-item="outbox/one"] [role="status"]')?.textContent).toBe('Discard did not finish: outbox unavailable; retry later.')
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one')
  })

  it('drops a list answer that arrives after the port changed or the view unmounted', async () => {
    const stale = Promise.withResolvers<Items>()
    const first = { list: vi.fn(() => stale.promise), discard: vi.fn(async () => {}) } satisfies OutboxRecoveryPort
    const { port: second } = memoryPort([damaged('outbox/second')])
    await mount(first)
    await mount(second)
    await act(async () => stale.resolve([damaged('outbox/first')]))
    expect([...host.querySelectorAll('[data-outbox-item]')].map(row => row.getAttribute('data-outbox-item'))).toEqual(['outbox/second'])

    const late = Promise.withResolvers<Items>()
    const third = { list: vi.fn(() => late.promise), discard: vi.fn(async () => {}) } satisfies OutboxRecoveryPort
    await mount(third)
    await act(async () => root.unmount())
    await act(async () => late.resolve([damaged('outbox/late')]))
    expect(host.innerHTML).toBe('')
    root = createRoot(host)
  })
})
