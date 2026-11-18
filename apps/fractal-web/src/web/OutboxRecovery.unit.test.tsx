// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { OutboxCopyOutcome, OutboxRecoveryCopy, OutboxRecoveryItem, OutboxRecoveryPort } from '../data/source.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

import { OutboxRecovery, RECOVERY_UNAVAILABLE } from './OutboxRecovery.tsx'
import { createRecoveryStore, isOutboxConflict } from './outboxRecoveryState.ts'

const DAMAGED = 'storage damaged; delivery unknown'
// A damaged entry may have no readable revision; every other state carries the revision the list read.
const damaged = (id: string): OutboxRecoveryItem => ({ id, revision: undefined, kind: 'corrupt', subject: 'message', status: DAMAGED, actions: ['discard'] })
const unknown = (id: string, subject: OutboxRecoveryItem['subject'] = 'message', revision = 1): OutboxRecoveryItem =>
  ({ id, revision, kind: 'unknown', subject, status: 'delivery unconfirmed', actions: ['copy', 'discard'], text: 'Ship the release notes' })
const notApplied = (id: string, revision = 1): OutboxRecoveryItem => ({ id, revision, kind: 'not-applied', subject: 'message', status: 'Fractal did not apply it', actions: ['retry', 'discard'] })
const pending = (id: string, revision = 1): OutboxRecoveryItem => ({ id, revision, kind: 'pending', subject: 'thread', status: 'waiting to send', actions: ['send', 'discard'] })
const UNCONFIRMED_MESSAGE = 'Fractal has not confirmed this message. Do not send it again.'
const UNCONFIRMED_THREAD = 'Fractal has not confirmed this thread. Do not send it again.'
const MAY_HAVE_ARRIVED = 'This removes the copy on this device only. It does not cancel the original, which may already have arrived.'

/** The outbox's typed refusal, shaped as the data layer rejects it: a tagged error, recognized by tag alone. */
const conflict = (localAttemptId: string, expectedRevision: number | undefined, actualRevision?: number) =>
  Object.assign(new Error('The outbox attempt changed.'), { _tag: 'OutboxConflict' as const, localAttemptId, expectedRevision, ...(actualRevision === undefined ? {} : { actualRevision }) })

/**
 * A full production-shaped port over an in-memory list; `fail` makes lists reject as unavailable storage. Every command
 * compares the observed revision with the stored one atomically and refuses a mismatch with the typed conflict.
 */
const memoryPort = (initial: readonly OutboxRecoveryItem[], copied: Omit<OutboxRecoveryCopy, 'copyKey'> = { text: 'Ship the release notes', images: [] }) => {
  let items = initial
  let copies = 0
  const state = { fail: false }
  const wake = new Set<() => void>()
  const take = (id: string, expectedRevision: number | undefined) => {
    const item = items.find(entry => entry.id === id)
    if (item === undefined || item.revision !== expectedRevision) throw conflict(id, expectedRevision, item?.revision)
    return item
  }
  const remove = (id: string, expectedRevision: number | undefined) => {
    take(id, expectedRevision)
    items = items.filter(item => item.id !== id)
  }
  const port = {
    list: vi.fn(async () => { if (state.fail) throw new Error('storage unavailable'); return items }),
    discard: vi.fn(async (id: string, expectedRevision: number | undefined) => remove(id, expectedRevision)),
    subscribe: vi.fn((listener: () => void) => { wake.add(listener); return () => { wake.delete(listener) } }),
    retry: vi.fn(async (id: string, expectedRevision: number | undefined) => remove(id, expectedRevision)),
    send: vi.fn(async (id: string, expectedRevision: number | undefined) => remove(id, expectedRevision)),
    copy: vi.fn(async (id: string, expectedRevision: number | undefined): Promise<OutboxRecoveryCopy> => { take(id, expectedRevision); return { ...copied, copyKey: `outbox/copy/${++copies}` } }),
    sendCopy: vi.fn(async (_id: string, _copyKey: string, _text: string): Promise<OutboxCopyOutcome> => ({ _tag: 'Recorded' })),
  } satisfies OutboxRecoveryPort
  const replace = (next: readonly OutboxRecoveryItem[]) => { items = next }
  const notify = () => act(async () => { for (const listener of wake) listener() })
  return { port, state, wake, notify, replace }
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
const rowButtons = (id: string) => [...host.querySelectorAll<HTMLButtonElement>(`[data-outbox-item="${id}"] button`)].map(button => button.textContent)
const rowIds = () => [...host.querySelectorAll('[data-outbox-item]')].map(row => row.getAttribute('data-outbox-item'))
const pressIn = (scope: ParentNode, label: string) => act(async () => {
  const target = [...scope.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === label)
  if (target === undefined) throw new Error(`no ${label}`)
  target.click()
})
const pressRow = (id: string, label: string) => pressIn(host.querySelector(`[data-outbox-item="${id}"]`)!, label)
const confirmation = () => host.querySelector<HTMLElement>('[data-outbox-discard-confirmation]')
const composer = () => host.querySelector<HTMLElement>('[data-outbox-copy-composer]')
const named = (button: Element) => (button.getAttribute('aria-labelledby') ?? '').split(' ').map(id => document.getElementById(id)?.textContent).join(' ')
const type = (field: HTMLTextAreaElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, value)
  field.dispatchEvent(new Event('input', { bubbles: true }))
})

describe('OutboxRecovery', () => {
  it('lists a damaged item with its exact status and Discard as its only action', async () => {
    const { port } = memoryPort([damaged('outbox/one')])
    await mount(port)
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-status]')?.textContent).toBe(DAMAGED)
    expect(rowButtons('outbox/one')).toEqual(['Discard'])
    expect(named(host.querySelector('[data-outbox-item="outbox/one"] button')!)).toBe(`Discard ${DAMAGED}`)
  })

  it('says an unknown message or thread is unconfirmed, offers Copy and Discard only, and shows no delivery status', async () => {
    const { port } = memoryPort([unknown('outbox/message'), unknown('outbox/thread', 'thread')])
    await mount(port)
    expect(host.querySelector('[data-outbox-item="outbox/message"] [data-outbox-headline]')?.textContent).toBe(UNCONFIRMED_MESSAGE)
    expect(host.querySelector('[data-outbox-item="outbox/thread"] [data-outbox-headline]')?.textContent).toBe(UNCONFIRMED_THREAD)
    for (const id of ['outbox/message', 'outbox/thread']) expect(rowButtons(id)).toEqual(['Copy', 'Discard'])
    expect(host.querySelector('[data-outbox-kind="unknown"] [data-outbox-status]')).toBeNull()
    expect(host.textContent).not.toContain('delivery unconfirmed')
    for (const absent of ['Retry', 'Revalidate and send', 'Resend']) expect(host.textContent).not.toContain(absent)
  })

  it('never draws an action the port does not implement, even when the outbox grants it', async () => {
    const legacy = { list: vi.fn(async () => [unknown('outbox/one'), notApplied('outbox/two'), pending('outbox/three')]), discard: vi.fn(async () => {}) } satisfies OutboxRecoveryPort
    await mount(legacy)
    for (const id of ['outbox/one', 'outbox/two', 'outbox/three']) expect(rowButtons(id)).toEqual(['Discard'])
  })

  it('retries a not-applied attempt through the port and lists again', async () => {
    const { port } = memoryPort([notApplied('outbox/one')])
    await mount(port)
    expect(rowButtons('outbox/one')).toEqual(['Retry', 'Discard'])
    await pressRow('outbox/one', 'Retry')
    expect(port.retry).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    expect(port.send).not.toHaveBeenCalled()
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(rowIds()).toEqual([])
  })

  it('revalidates and sends a pending attempt through the port and lists again', async () => {
    const { port } = memoryPort([pending('outbox/one')])
    await mount(port)
    expect(rowButtons('outbox/one')).toEqual(['Revalidate and send', 'Discard'])
    await pressRow('outbox/one', 'Revalidate and send')
    expect(port.send).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    expect(port.retry).not.toHaveBeenCalled()
    expect(port.list).toHaveBeenCalledTimes(2)
  })

  it('keeps a pending row and says why when revalidation refuses the send', async () => {
    const { port } = memoryPort([pending('outbox/one')])
    port.send.mockRejectedValueOnce(new Error('recipient changed'))
    await mount(port)
    await pressRow('outbox/one', 'Revalidate and send')
    expect(rowIds()).toEqual(['outbox/one'])
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-failure]')?.getAttribute('role')).toBe('status')
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-failure]')?.getAttribute('data-outbox-failure')).toBe('send')
  })

  it('keeps each row state in one live status that changes only when the attempt state does', async () => {
    const { port, notify, replace } = memoryPort([pending('outbox/one')])
    await mount(port)
    const live = host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-state]')!
    expect(live.getAttribute('role')).toBe('status')
    expect(live.getAttribute('aria-atomic')).toBe('true')
    const before = live.textContent
    // An unchanged re-read leaves the same live node and text, so nothing is re-announced.
    await notify()
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-state]')).toBe(live)
    expect(live.textContent).toBe(before)
    // A wakeup that moves the attempt to unknown updates that same live node.
    replace([unknown('outbox/one', 'thread', 2)])
    await notify()
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-state]')).toBe(live)
    expect(live.textContent).toBe(UNCONFIRMED_THREAD)
  })

  it('asks before discarding; Keep leaves the attempt and Discard removes it', async () => {
    const { port } = memoryPort([unknown('outbox/one'), damaged('outbox/two')])
    await mount(port)
    await pressRow('outbox/one', 'Discard')
    const dialog = confirmation()!
    expect(dialog.getAttribute('role')).toBe('alertdialog')
    expect(document.getElementById(dialog.getAttribute('aria-describedby')!)?.textContent).toBe('This removes the copy on this device only. It does not cancel the original, which may already have arrived.')
    expect(port.discard).not.toHaveBeenCalled()
    await pressIn(dialog, 'Keep')
    expect(port.discard).not.toHaveBeenCalled()
    expect(rowIds()).toEqual(['outbox/one', 'outbox/two'])

    await pressRow('outbox/one', 'Discard')
    await pressIn(confirmation()!, 'Discard')
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(rowIds()).toEqual(['outbox/two'])
  })

  it('refuses a Discard whose pending attempt another tab dispatched, then asks again with its current unknown state', async () => {
    const { port, replace } = memoryPort([pending('outbox/one')])
    await mount(port)
    await pressRow('outbox/one', 'Discard')
    expect(document.getElementById(confirmation()!.getAttribute('aria-describedby')!)?.textContent).toBe('This removes the thread from this device. It will not be sent.')
    // Tab B takes the dispatch permit: the persisted attempt is now unknown at revision 2, and no wakeup has arrived yet.
    replace([unknown('outbox/one', 'thread', 2)])
    await pressIn(confirmation()!, 'Discard')
    // Bound to the revision shown: the outbox refused atomically and removed nothing.
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    expect(rowIds()).toEqual(['outbox/one'])
    expect(host.querySelector('[data-outbox-item="outbox/one"]')?.getAttribute('data-outbox-kind')).toBe('unknown')
    // The dialog stays open on the current state and consequence, and Keep takes focus back.
    const dialog = confirmation()!
    const notice = dialog.querySelector('[data-outbox-discard-changed-notice]')!
    expect(notice.getAttribute('role')).toBe('alert')
    expect(notice.textContent).toBe(`This thread changed in another tab or window. Nothing was discarded. ${UNCONFIRMED_THREAD}`)
    expect(dialog.querySelector('[data-outbox-discard-consequence]')?.textContent).toBe(MAY_HAVE_ARRIVED)
    expect(dialog.getAttribute('aria-describedby')!.split(' ').map(id => document.getElementById(id)?.textContent)).toEqual([notice.textContent, MAY_HAVE_ARRIVED])
    expect(document.activeElement?.textContent).toBe('Keep')
    // Never retried automatically: only a second, deliberate Discard acts, and it names the current revision.
    expect(port.discard).toHaveBeenCalledOnce()
    await pressIn(confirmation()!, 'Discard')
    expect(port.discard.mock.calls).toEqual([['outbox/one', 1], ['outbox/one', 2]])
    expect(confirmation()).toBeNull()
    expect(rowIds()).toEqual([])
  })

  it('Keep after a stale Discard leaves the attempt and says nothing was discarded', async () => {
    const { port, replace } = memoryPort([pending('outbox/one')])
    await mount(port)
    await pressRow('outbox/one', 'Discard')
    replace([unknown('outbox/one', 'thread', 2)])
    await pressIn(confirmation()!, 'Discard')
    await pressIn(confirmation()!, 'Keep')
    expect(confirmation()).toBeNull()
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    const failure = host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-failure]')!
    expect(failure.getAttribute('data-outbox-failure-cause')).toBe('stale')
    expect(failure.textContent).toBe('Not discarded: this entry changed in another tab or window. It shows its current state.')
    // An unknown original is only copied, never retried or re-sent.
    expect(rowButtons('outbox/one')).toEqual(['Copy', 'Discard'])
  })

  it('updates an open confirmation when a wakeup re-reads the attempt at another revision, before anything is discarded', async () => {
    const { port, replace, notify } = memoryPort([pending('outbox/one')])
    await mount(port)
    await pressRow('outbox/one', 'Discard')
    replace([unknown('outbox/one', 'thread', 2)])
    await notify()
    expect(port.discard).not.toHaveBeenCalled()
    expect(confirmation()?.querySelector('[data-outbox-discard-changed-notice]')?.textContent).toBe(`This thread changed in another tab or window. Nothing was discarded. ${UNCONFIRMED_THREAD}`)
    expect(confirmation()?.querySelector('[data-outbox-discard-consequence]')?.textContent).toBe(MAY_HAVE_ARRIVED)
    expect(document.activeElement?.textContent).toBe('Keep')
    await pressIn(confirmation()!, 'Discard')
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one', 2)
  })

  it('refuses a send whose attempt changed in another tab and shows its current state without Retry', async () => {
    const { port, replace } = memoryPort([pending('outbox/one')])
    await mount(port)
    replace([unknown('outbox/one', 'thread', 2)])
    await pressRow('outbox/one', 'Revalidate and send')
    expect(port.send).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-headline]')?.textContent).toBe(UNCONFIRMED_THREAD)
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-failure]')?.textContent).toBe('Not sent: this entry changed in another tab or window. It shows its current state.')
    expect(rowButtons('outbox/one')).toEqual(['Copy', 'Discard'])
  })

  it('keeps the row and says why when Discard is refused', async () => {
    const { port } = memoryPort([damaged('outbox/one')])
    await mount(port)
    port.discard.mockRejectedValueOnce(new Error('storage unavailable'))
    await pressRow('outbox/one', 'Discard')
    await pressIn(confirmation()!, 'Discard')
    expect(port.discard).toHaveBeenCalledExactlyOnceWith('outbox/one', undefined)
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(host.querySelector('[data-outbox-item="outbox/one"] [data-outbox-failure]')?.textContent).toBe('Discard did not finish: outbox unavailable; retry later.')
  })

  it('copies an unknown original into a separate composer that warns, carries its images and sends a new attempt', async () => {
    const draft = document.createElement('textarea')
    draft.value = 'An unrelated conversation draft'
    document.body.append(draft)
    try {
      const { port } = memoryPort([unknown('outbox/one')], { text: 'Ship the release notes', images: [{ name: 'diagram.png', mediaType: 'image/png', bytes: 2048 }] })
      await mount(port)
      await pressRow('outbox/one', 'Copy')
      expect(port.copy).toHaveBeenCalledExactlyOnceWith('outbox/one', 1)
      const dialog = composer()!
      expect(dialog.querySelector('[data-outbox-copy-warning]')?.textContent).toContain('The original may have arrived.')
      expect(document.getElementById(dialog.getAttribute('aria-describedby')!)).toBe(dialog.querySelector('[data-outbox-copy-warning]'))
      expect([...dialog.querySelectorAll('[data-outbox-copy-images] li')].map(item => item.textContent)).toEqual(['diagram.png'])
      const field = dialog.querySelector('textarea')!
      expect(field.value).toBe('Ship the release notes')
      await type(field, 'Ship the release notes (copy)')
      await pressIn(dialog, 'Send copy')
      expect(port.sendCopy).toHaveBeenCalledExactlyOnceWith('outbox/one', 'outbox/copy/1', 'Ship the release notes (copy)')
      expect(composer()).toBeNull()
      // The composer closes on a recorded copy and says where its outcome lives, without claiming delivery.
      const notice = host.querySelector('[data-outbox-copy-notice]')!
      expect(notice.getAttribute('role')).toBe('status')
      expect(notice.textContent).toBe('The copy is saved as its own new message. If Fractal does not confirm it, it is listed here. Do not send it again.')
      // The original is never retried or re-sent, and no other composer changed.
      expect(port.retry).not.toHaveBeenCalled()
      expect(port.send).not.toHaveBeenCalled()
      expect(draft.value).toBe('An unrelated conversation draft')
    } finally { draft.remove() }
  })

  it('keeps the composer open after a refusal and sends again under the same copy key', async () => {
    const { port } = memoryPort([unknown('outbox/one')])
    port.sendCopy.mockResolvedValueOnce({ _tag: 'Refused', detail: 'The recipient could not be verified.' })
    await mount(port)
    await pressRow('outbox/one', 'Copy')
    await pressIn(composer()!, 'Send copy')
    expect(composer()?.querySelector('[role="alert"]')?.textContent).toBe('Fractal refused this copy and nothing new was saved: The recipient could not be verified. Your text is still here.')
    await type(composer()!.querySelector('textarea')!, 'Edited after refusal')
    await pressIn(composer()!, 'Send copy')
    expect(port.sendCopy.mock.calls).toEqual([['outbox/one', 'outbox/copy/1', 'Ship the release notes'], ['outbox/one', 'outbox/copy/1', 'Edited after refusal']])
    expect(composer()).toBeNull()
  })

  it('closes the composer without claiming the copy was not sent when the outcome is unclear', async () => {
    const { port } = memoryPort([unknown('outbox/one')])
    port.sendCopy.mockRejectedValueOnce(new Error('connection lost'))
    await mount(port)
    await pressRow('outbox/one', 'Copy')
    await pressIn(composer()!, 'Send copy')
    // It may have been saved: there is no second chance to send, and the list owns the outcome.
    expect(port.sendCopy).toHaveBeenCalledOnce()
    expect(composer()).toBeNull()
    expect(host.textContent).not.toMatch(/not sent|refused/i)
    expect(host.querySelector('[data-outbox-copy-notice]')?.textContent).toContain('If Fractal does not confirm it, it is listed here.')
  })

  it('gives each opened composer its own copy key', async () => {
    const { port } = memoryPort([unknown('outbox/one')])
    await mount(port)
    await pressRow('outbox/one', 'Copy')
    await pressIn(composer()!, 'Cancel')
    await pressRow('outbox/one', 'Copy')
    await pressIn(composer()!, 'Send copy')
    expect(port.sendCopy).toHaveBeenCalledExactlyOnceWith('outbox/one', 'outbox/copy/2', 'Ship the release notes')
  })

  it('saves nothing and offers no second send when the original changed since the copy opened', async () => {
    const { port } = memoryPort([unknown('outbox/one')])
    port.sendCopy.mockRejectedValueOnce(conflict('outbox/one', 1, 2))
    await mount(port)
    await pressRow('outbox/one', 'Copy')
    await pressIn(composer()!, 'Send copy')
    const alert = composer()!.querySelector('[data-outbox-copy-stale]')!
    expect(alert.getAttribute('role')).toBe('alert')
    expect(alert.textContent).toBe('The original changed in another tab or window, so nothing new was saved. Your text is still here; close to see its current state.')
    expect(composer()!.querySelector('textarea')!.value).toBe('Ship the release notes')
    const send = [...composer()!.querySelectorAll('button')].find(button => button.textContent === 'Send copy')!
    expect(send.disabled).toBe(true)
    await pressIn(composer()!, 'Close')
    expect(port.sendCopy).toHaveBeenCalledOnce()
    expect(host.querySelector('[data-outbox-copy-notice]')?.textContent).toBe('')
  })

  it('renders nothing for an empty outbox', async () => {
    const { port } = memoryPort([])
    await mount(port)
    expect(host.querySelector('[data-outbox-recovery]')).toBeNull()
  })

  it('says storage is unavailable without any action, and a change wakeup only lists again', async () => {
    const { port, state, notify } = memoryPort([damaged('outbox/one')])
    state.fail = true
    await mount(port)
    const notice = host.querySelector('[data-outbox-recovery="unavailable"]')
    expect(notice?.querySelector('[role="status"]')?.textContent).toBe(RECOVERY_UNAVAILABLE)
    expect(host.textContent?.toLowerCase()).not.toMatch(/damaged|corrupt/)
    expect(host.querySelectorAll('button')).toHaveLength(0)
    state.fail = false
    await notify()
    expect(port.list).toHaveBeenCalledTimes(2)
    expect(host.querySelector('[data-outbox-recovery="unavailable"]')).toBeNull()
    expect(rowIds()).toEqual(['outbox/one'])
    for (const command of [port.discard, port.retry, port.send, port.copy, port.sendCopy]) expect(command).not.toHaveBeenCalled()
  })

  it('follows wakeups only while mounted', async () => {
    const { port, wake } = memoryPort([damaged('outbox/one')])
    await mount(port)
    expect(wake.size).toBe(1)
    await act(async () => root.unmount())
    expect(wake.size).toBe(0)
    root = createRoot(host)
  })
})

describe('recovery store', () => {
  it('applies only the newest list answer', async () => {
    const first = Promise.withResolvers<readonly OutboxRecoveryItem[]>()
    const second = Promise.withResolvers<readonly OutboxRecoveryItem[]>()
    const answers = [first.promise, second.promise]
    const listeners = new Set<() => void>()
    const store = createRecoveryStore({ list: vi.fn(() => answers.shift()!), discard: vi.fn(async () => {}), subscribe: listener => { listeners.add(listener); return () => { listeners.delete(listener) } } })
    const release = store.acquire()
    for (const listener of listeners) listener()
    second.resolve([damaged('outbox/new')])
    await second.promise
    first.resolve([damaged('outbox/old')])
    await first.promise
    await Promise.resolve()
    const list = store.snapshot().list
    expect(list._tag === 'Listed' ? list.items.map(item => item.id) : list._tag).toEqual(['outbox/new'])
    release()
  })

  it('refuses every command bound to a revision the latest list has moved past, without calling the port', async () => {
    let items: readonly OutboxRecoveryItem[] = [pending('outbox/one')]
    const listeners = new Set<() => void>()
    const port = {
      list: vi.fn(async () => items), discard: vi.fn(async () => {}), send: vi.fn(async () => {}), retry: vi.fn(async () => {}),
      subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    } satisfies OutboxRecoveryPort
    const store = createRecoveryStore(port)
    const release = store.acquire()
    await vi.waitFor(() => expect(store.snapshot().list._tag).toBe('Listed'))
    items = [unknown('outbox/one', 'thread', 2)]
    // Another tab's wakeup lists again; the pending row the person saw no longer authorizes anything.
    for (const listener of listeners) listener()
    await vi.waitFor(() => expect(store.snapshot().list).toEqual({ _tag: 'Listed', items }))
    expect(await store.send('outbox/one', 1)).toEqual({ _tag: 'Stale' })
    expect(await store.retry('outbox/one', 1)).toEqual({ _tag: 'Stale' })
    expect(await store.discard('outbox/one', 1)).toEqual({ _tag: 'Stale' })
    for (const command of [port.send, port.retry, port.discard]) expect(command).not.toHaveBeenCalled()
    expect(store.snapshot().failed.get('outbox/one')).toEqual({ operation: 'discard', cause: 'stale' })
    release()
  })

  it.each([
    ['retry', notApplied('outbox/retry'), notApplied('outbox/retry', 2)],
    ['send', pending('outbox/send'), unknown('outbox/send', 'thread', 2)],
    ['copy', unknown('outbox/copy'), unknown('outbox/copy', 'message', 2)],
    ['discard', pending('outbox/discard'), unknown('outbox/discard', 'thread', 2)],
  ] as const)('answers the typed conflict as Stale for %s, removes nothing, and lists the current state', async (command, initial, current) => {
    const { port, replace } = memoryPort([initial])
    const store = createRecoveryStore(port)
    const release = store.acquire()
    await vi.waitFor(() => expect(store.snapshot().list._tag).toBe('Listed'))
    replace([current])
    expect(await store[command](initial.id, initial.revision)).toEqual({ _tag: 'Stale' })
    expect(port[command]).toHaveBeenCalledExactlyOnceWith(initial.id, initial.revision)
    expect(store.snapshot().list).toEqual({ _tag: 'Listed', items: [current] })
    expect(store.snapshot().failed.get(initial.id)).toEqual({ operation: command, cause: 'stale' })
    release()
  })

  it('tells a stale copy from an unclear one: only the typed conflict saved nothing', async () => {
    const { port } = memoryPort([unknown('outbox/one')])
    const store = createRecoveryStore(port)
    port.sendCopy.mockRejectedValueOnce(conflict('outbox/one', 1, 2)).mockRejectedValueOnce(new Error('connection lost'))
    expect(await store.sendCopy('outbox/one', 'outbox/copy/1', 'text')).toEqual({ _tag: 'Stale' })
    expect(await store.sendCopy('outbox/one', 'outbox/copy/1', 'text')).toEqual({ _tag: 'Recorded' })
  })

  it('recognizes the outbox conflict by its tag alone', () => {
    expect(isOutboxConflict(conflict('outbox/one', undefined))).toBe(true)
    expect(isOutboxConflict({ _tag: 'OutboxConflict', localAttemptId: 'outbox/one', expectedRevision: 1 })).toBe(true)
    for (const other of [new Error('OutboxConflict'), { _tag: 'OutboxStorageError' }, 'OutboxConflict', null, undefined]) expect(isOutboxConflict(other)).toBe(false)
  })
})
