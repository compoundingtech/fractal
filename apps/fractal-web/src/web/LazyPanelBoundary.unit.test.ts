// @vitest-environment jsdom
import { expect, it, vi } from 'vitest'
import { lazyPanel } from './LazyPanelBoundary.tsx'

it('shares idle, trigger and opening demand without evaluating the body twice', async () => {
  const module = { body: 'ready' }
  const load = vi.fn(async () => module)
  const panel = lazyPanel('test', load)
  const changed = vi.fn()
  const unsubscribe = panel.subscribe(changed)
  panel.prefetch()
  panel.prefetch()
  panel.request()
  expect(panel.getSnapshot()._tag).toBe('Pending')
  await vi.waitFor(() => expect(panel.getSnapshot()).toEqual({ _tag: 'Ready', module }))
  panel.request()
  expect(load).toHaveBeenCalledTimes(1)
  expect(changed).toHaveBeenCalledTimes(2)
  unsubscribe()
})

it('keeps speculative failure local and offers one demand retry before reload', async () => {
  const load = vi.fn().mockRejectedValue(new TypeError('chunk unavailable'))
  const panel = lazyPanel('test', load)
  panel.prefetch()
  await vi.waitFor(() => expect(panel.getSnapshot()).toEqual({ _tag: 'Failed', recovery: 'retry' }))
  panel.prefetch()
  expect(load).toHaveBeenCalledTimes(1)
  panel.request()
  await vi.waitFor(() => expect(panel.getSnapshot()).toEqual({ _tag: 'Failed', recovery: 'reload' }))
  panel.request()
  expect(load).toHaveBeenCalledTimes(2)
})
