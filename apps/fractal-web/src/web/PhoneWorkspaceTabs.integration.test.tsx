// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { PhoneWorkspaceTabs } from './PhoneWorkspaceTabs.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const tabs = ['Thread', 'Changes', 'Rollback plan', 'On-call for release day', 'Smoke tests after deploy'].map(label => ({ id: label, label }))
const descriptors = {
  scrollLeft: Object.getOwnPropertyDescriptor(Element.prototype, 'scrollLeft')!,
  rect: Object.getOwnPropertyDescriptor(Element.prototype, 'getBoundingClientRect')!,
}
afterEach(() => {
  Object.defineProperty(Element.prototype, 'scrollLeft', descriptors.scrollLeft)
  Object.defineProperty(Element.prototype, 'getBoundingClientRect', descriptors.rect)
  vi.unstubAllGlobals()
})

it('scrolls only the tab row to show a tab selected from elsewhere', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  // A 300px row of 120px tabs: the row scrolls by its own scrollLeft; tabs are laid out from its left edge.
  const scrolled = new WeakMap<Element, number>()
  Object.defineProperty(Element.prototype, 'scrollLeft', {
    configurable: true,
    get(this: Element) { return scrolled.get(this) ?? 0 },
    set(this: Element, value: number) { scrolled.set(this, value) },
  })
  Object.defineProperty(Element.prototype, 'getBoundingClientRect', {
    configurable: true,
    value(this: Element) {
      if (this.getAttribute('role') === 'tablist') return DOMRect.fromRect({ x: 0, width: 300, height: 44 })
      const row = this.parentElement!
      const left = [...row.children].indexOf(this) * 120 - row.scrollLeft
      return DOMRect.fromRect({ x: left, width: 120, height: 44 })
    },
  })
  const host = document.body.appendChild(document.createElement('div'))
  const root = createRoot(host)
  const render = (selected: string) => act(async () => root.render(
    <PhoneWorkspaceTabs tabs={tabs} selected={selected} onSelect={() => {}}><p>panel</p></PhoneWorkspaceTabs>,
  ))
  const row = () => host.querySelector('[role="tablist"]')!
  try {
    await render('Thread')
    expect(row().scrollLeft).toBe(0)
    await render('Smoke tests after deploy')
    // The fifth tab spans 480–600: its end meets the row's end.
    expect(row().scrollLeft).toBe(300)
    await render('Changes')
    expect(row().scrollLeft).toBe(120)
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
