// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { RegistryContext } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

import { ThreadPanel } from './ThreadPanel.tsx'
import { UnreadReminderControls } from './UnreadReminderControls.tsx'
import type { UnreadActionPort, UnreadActionResult, UnreadReminderRead, UnreadSeekResult } from './unread-action-port.ts'

beforeEach(() => vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true))
afterEach(() => vi.unstubAllGlobals())

const threadId = 'thread/review'
const reminded: UnreadReminderRead = { _tag: 'Known', reminder: {
  anchor_message_id: 'message/2',
  marks: [{ mark_id: 'operation/mark-1', anchor_message_id: 'message/2' }, { mark_id: 'operation/mark-2', anchor_message_id: 'message/3' }],
} }

const setup = async ({ read, canMark, visible = ['message/2', 'message/3'], withTimeline = true }: {
  readonly read: UnreadReminderRead
  readonly canMark: boolean
  readonly visible?: readonly string[]
  readonly withTimeline?: boolean
}) => {
  const registry = AtomRegistry.make()
  const reminder = Atom.make<UnreadReminderRead>(read)
  const order: string[] = []
  const mark = vi.fn<UnreadActionPort['mark']>(async () => { order.push('mark'); return { _tag: 'Sent' } })
  const clear = vi.fn<UnreadActionPort['clear']>(async () => ({ _tag: 'Sent' }))
  const seek = vi.fn<UnreadActionPort['seek']>(async request => ({ _tag: 'Found', messageId: request.anchor_message_id }))
  const port: UnreadActionPort = { canMark, reminder: () => reminder, mark, clear, seek }
  const marked = vi.fn(() => { order.push('hold') })
  const host = document.createElement('div')
  document.body.append(host)
  const timeline = document.createElement('div')
  for (const id of ['message/1', 'message/2', 'message/3']) {
    const row = document.createElement('article')
    row.setAttribute('data-thread-anchor', id)
    row.textContent = `Reply ${id}`
    const link = document.createElement('a')
    link.href = '#'
    link.textContent = 'link'
    row.append(link)
    timeline.append(row)
  }
  document.body.append(timeline)
  const root = createRoot(host)
  await act(async () => root.render(<RegistryContext.Provider value={registry}>
    <UnreadReminderControls threadId={threadId} port={port} visibleReplies={() => visible}
      {...(withTimeline ? { timeline } : {})} onBeforeMark={marked} />
  </RegistryContext.Provider>))
  const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
  return {
    host, timeline, mark, clear, seek, marked, order, button,
    set: (next: UnreadReminderRead) => act(async () => registry.set(reminder, next)),
    press: async (label: string) => {
      const target = button(label)
      expect(target).not.toBeNull()
      await act(async () => target?.click())
    },
    optionClick: (target: Element) => act(async () => {
      target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, altKey: true, button: 0 }))
    }),
    dispose: async () => {
      await act(async () => root.unmount())
      host.remove()
      timeline.remove()
      registry.dispose()
    },
  }
}

const markUnread = 'Mark unread from the first visible reply'
const markRead = 'Mark read, clearing the unread reminder'
const jump = 'Jump to the reply marked unread'

it('shows the reminder and Jump read-only without the grant, and nothing for an unknown read', async () => {
  const view = await setup({ read: reminded, canMark: false })
  try {
    expect(view.host.querySelector('[data-thread-unread-reminder]')?.textContent).toContain('Marked unread')
    expect(view.button(jump)).not.toBeNull()
    expect(view.button(markUnread)).toBeNull()
    expect(view.button(markRead)).toBeNull()
    await view.optionClick(view.timeline.querySelector('[data-thread-anchor="message/1"]')!)
    expect(view.mark).not.toHaveBeenCalled()

    // No reminder and no grant: no badge, no controls.
    await view.set({ _tag: 'Known', reminder: undefined })
    expect(view.host.querySelector('[data-thread-unread-reminder-controls]')).toBeNull()
    // An unavailable read is not evidence of no reminder, so even the grant offers no mutation.
    await view.set({ _tag: 'Unavailable', detail: 'This connection does not support unread reminders.' })
    expect(view.host.textContent).toBe('')
  } finally { await view.dispose() }
})

it('marks unread from the first visible reply and clears exactly the observed mark IDs', async () => {
  const view = await setup({ read: reminded, canMark: true })
  try {
    await view.press(markUnread)
    expect(view.mark).toHaveBeenCalledExactlyOnceWith({ threadId, anchor_message_id: 'message/2', observed_marks: ['operation/mark-1', 'operation/mark-2'] })
    expect(view.order).toEqual(['hold', 'mark'])

    await view.press(markRead)
    expect(view.clear).toHaveBeenCalledExactlyOnceWith({ threadId, observed_marks: ['operation/mark-1', 'operation/mark-2'] })

    // Once the reminder is gone, Mark read leaves and Mark unread sends no stale IDs.
    await view.set({ _tag: 'Known', reminder: undefined })
    expect(view.button(markRead)).toBeNull()
    expect(view.host.querySelector('[data-thread-unread-reminder]')).toBeNull()
    await view.press(markUnread)
    expect(view.mark).toHaveBeenLastCalledWith({ threadId, anchor_message_id: 'message/2', observed_marks: [] })
  } finally { await view.dispose() }
})

it('marks unread from an Option-clicked visible reply but not a plain click, a link inside it, or an offscreen reply', async () => {
  const view = await setup({ read: { _tag: 'Known', reminder: undefined }, canMark: true, visible: ['message/1', 'message/2'] })
  try {
    const row = view.timeline.querySelector('[data-thread-anchor="message/1"]')!
    await act(async () => { row.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0 })) })
    await view.optionClick(row.querySelector('a')!)
    await view.optionClick(view.timeline.querySelector('[data-thread-anchor="message/3"]')!)
    expect(view.mark).not.toHaveBeenCalled()
    await view.optionClick(row)
    expect(view.mark).toHaveBeenCalledExactlyOnceWith({ threadId, anchor_message_id: 'message/1', observed_marks: [] })
    expect(view.marked).toHaveBeenCalledOnce()
  } finally { await view.dispose() }
})

it('refuses without a visible reply and shows Fractal refusals as one alert', async () => {
  const view = await setup({ read: reminded, canMark: true, visible: [] })
  try {
    await view.press(markUnread)
    expect(view.mark).not.toHaveBeenCalled()
    expect(view.host.querySelector('[role=alert]')?.textContent).toBe('Scroll to a visible message before marking it unread.')

    view.clear.mockResolvedValueOnce({ _tag: 'Refused', detail: 'The reminder changed. Review it and try again.' } satisfies UnreadActionResult)
    await view.press(markRead)
    expect(view.host.querySelectorAll('[role=alert]')).toHaveLength(1)
    expect(view.host.querySelector('[role=alert]')?.textContent).toBe('The reminder changed. Review it and try again.')
    // Mark read never holds the visit; only a deliberate Mark unread does.
    expect(view.marked).not.toHaveBeenCalled()

    view.clear.mockRejectedValueOnce(new TypeError('The reply was lost'))
    await view.press(markRead)
    expect(view.host.querySelector('[role=alert]')?.textContent).toBe('The unread change could not be confirmed. Refresh to check its current state.')
  } finally { await view.dispose() }
})

it('jumps to the reminded reply, and names an anchor it cannot show yet instead of guessing', async () => {
  const scrolled = vi.fn()
  Element.prototype.scrollIntoView = scrolled
  const view = await setup({ read: reminded, canMark: false })
  try {
    await view.press(jump)
    expect(view.seek).toHaveBeenCalledExactlyOnceWith({ threadId, anchor_message_id: 'message/2' })
    expect(scrolled).toHaveBeenCalledOnce()
    expect(scrolled.mock.contexts[0]).toBe(view.timeline.querySelector('[data-thread-anchor="message/2"]'))
    expect(view.host.querySelector('[role=status]')).toBeNull()

    view.seek.mockResolvedValueOnce({ _tag: 'Unavailable', detail: 'That reply is no longer in this thread.' } satisfies UnreadSeekResult)
    await view.press(jump)
    expect(view.host.querySelector('[role=status]')?.textContent).toBe('That reply is no longer in this thread.')
    expect(view.host.querySelector('[role=alert]')).toBeNull()

    // Pending supplies no body and changes no badge; only the reason shows.
    view.seek.mockResolvedValueOnce({ _tag: 'Pending', detail: 'Fractal is still locating that reply.' } satisfies UnreadSeekResult)
    await view.press(jump)
    expect(view.host.querySelector('[role=status]')?.textContent).toBe('Fractal is still locating that reply.')
    expect(view.host.querySelectorAll('[data-thread-unread-reminder]')).toHaveLength(1)
    expect(view.timeline.querySelectorAll('[data-thread-anchor]')).toHaveLength(3)
    expect(scrolled).toHaveBeenCalledOnce()

    // Found before the hydrated page commits: say so, then reveal the row once it renders.
    view.seek.mockResolvedValueOnce({ _tag: 'Found', messageId: 'message/older' })
    await view.press(jump)
    expect(view.host.querySelector('[role=status]')?.textContent).toBe('The reply marked unread is not loaded in this view.')
    expect(scrolled).toHaveBeenCalledOnce()
    const older = document.createElement('article')
    older.setAttribute('data-thread-anchor', 'message/older')
    await act(async () => { view.timeline.prepend(older) })
    expect(scrolled).toHaveBeenCalledTimes(2)
    expect(scrolled.mock.contexts[1]).toBe(older)
    expect(view.host.querySelector('[role=status]')).toBeNull()

    // A newer jump disposes the earlier watch: a late row for the superseded answer is not revealed.
    view.seek.mockResolvedValueOnce({ _tag: 'Found', messageId: 'message/first' })
    await view.press(jump)
    view.seek.mockResolvedValueOnce({ _tag: 'Unavailable', detail: 'That reply is no longer in this thread.' })
    await view.press(jump)
    const late = document.createElement('article')
    late.setAttribute('data-thread-anchor', 'message/first')
    await act(async () => { view.timeline.prepend(late) })
    expect(scrolled).toHaveBeenCalledTimes(2)
    expect(view.host.querySelector('[role=status]')?.textContent).toBe('That reply is no longer in this thread.')
  } finally {
    await view.dispose()
    Reflect.deleteProperty(Element.prototype, 'scrollIntoView')
  }
})

it('offers no Jump without a timeline to show the reply in', async () => {
  const view = await setup({ read: reminded, canMark: true, withTimeline: false })
  try {
    expect(view.host.querySelector('[data-thread-unread-reminder]')).not.toBeNull()
    expect(view.button(jump)).toBeNull()
    expect(view.button(markUnread)).not.toBeNull()
  } finally { await view.dispose() }
})

it('places reminder controls in the expanded heading without changing natural Mark read', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const markVisible = vi.fn(async () => undefined)
  try {
    await act(async () => root.render(<ThreadPanel thread={{ title: 'PR #42', state: 'open' }} unread onMarkRead={markVisible} canMarkRead
      unreadControls={<span data-reminder-slot="">Reminder</span>}>
      <textarea aria-label="Thread message" />
    </ThreadPanel>))
    expect(host.querySelector('[data-reminder-slot]')).toBeNull()
    const header = host.querySelector<HTMLButtonElement>('button[aria-expanded]')!
    await act(async () => header.click())
    const heading = host.querySelector('[data-thread-read-heading]')
    expect(heading?.querySelector('[data-reminder-slot]')).not.toBeNull()
    expect(host.querySelector('button[aria-expanded]')).toBe(header)
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Mark visible replies read"]')?.click())
    expect(markVisible).toHaveBeenCalledOnce()

    // Reminder controls alone still pin the heading.
    await act(async () => root.render(<ThreadPanel thread={{ title: 'PR #42', state: 'open' }} unreadControls={<span data-reminder-slot="">Reminder</span>}>
      <textarea aria-label="Thread message" />
    </ThreadPanel>))
    expect(host.querySelector('[aria-label="Mark visible replies read"]')).toBeNull()
    expect(host.querySelector('[data-thread-read-heading] [data-reminder-slot]')).not.toBeNull()
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
