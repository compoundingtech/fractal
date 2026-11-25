// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { useAui } from '@assistant-ui/react'
import { userEvent } from 'storybook/test'
import { expect, it, vi } from 'vitest'
import { EmbraceComposer } from './EmbraceComposer'
import { EmbraceRuntimeProvider } from './EmbraceRuntime'

it('PlainComposer honors sendDisabled for pointer, keyboard and form submission without losing focus or draft', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  Object.defineProperty(window, 'matchMedia', { configurable: true, value: (media: string) => ({ media, matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }) })
  // jsdom has no font-loading API; preserve actual event delivery for the kit's layout observer.
  Object.defineProperty(document, 'fonts', { configurable: true, value: Object.assign(new EventTarget(), { ready: Promise.resolve(), status: 'loaded' }) })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const submit = vi.fn()
  function Composer({ disabled }: { readonly disabled: boolean }) {
    const aui = useAui()
    React.useLayoutEffect(() => { aui.composer().setText('Keep this draft.') }, [aui])
    return <EmbraceComposer plainText variant="C1" sendDisabled={disabled} onRequestSubmit={submit} />
  }
  const render = (disabled: boolean) => act(async () => root.render(<EmbraceRuntimeProvider options={{ messages: [], onNew: async () => {} }}><Composer disabled={disabled} /></EmbraceRuntimeProvider>))
  try {
    await render(true)
    const button = host.querySelector<HTMLButtonElement>('button[aria-label="Send"]')!
    const input = host.querySelector('textarea')!
    expect(button.getAttribute('aria-disabled')).toBe('true')
    expect(button.disabled).toBe(false)
    await act(async () => button.click())
    button.focus()
    expect(document.activeElement).toBe(button)
    await act(async () => userEvent.keyboard('{Enter} '))
    expect(document.activeElement).toBe(button)
    input.focus()
    await act(async () => userEvent.keyboard('{Enter}{Control>}{Enter}{/Control}'))
    await act(async () => host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })))
    expect(submit).not.toHaveBeenCalled()
    expect(input.value).toBe('Keep this draft.')

    // Positive controls prove the same real pointer, keyboard and form paths are live when permitted.
    await render(false)
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Send"]')!.click())
    expect(submit).toHaveBeenCalledTimes(1)
    input.focus()
    await act(async () => userEvent.keyboard('{Enter}'))
    expect(submit).toHaveBeenCalledTimes(2)
    await act(async () => host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })))
    expect(submit).toHaveBeenCalledTimes(3)
  } finally {
    await act(async () => root.unmount())
    host.remove()
    vi.unstubAllGlobals()
  }
})
