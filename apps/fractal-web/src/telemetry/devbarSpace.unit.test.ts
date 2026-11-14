// @vitest-environment jsdom
import { afterEach, expect, it } from 'vitest'
import { reserveDevbarSpace } from './devbarSpace.ts'

afterEach(() => { document.body.replaceChildren() })

const devbarHost = (height: number): HTMLElement => {
  const host = document.createElement('div')
  host.innerHTML = '<aside><section aria-label="Counters"></section><div role="toolbar" aria-label="Developer bar"></div></aside>'
  host.querySelector('[role="toolbar"]')!.getBoundingClientRect = () => ({ height }) as DOMRect
  host.querySelector('section')!.getBoundingClientRect = () => ({ height: 500 }) as DOMRect
  document.body.append(host)
  return host
}

it('publishes the bar row height, not an open panel, and clears it on unmount', () => {
  const release = reserveDevbarSpace(devbarHost(32))
  expect(document.body.style.getPropertyValue('--wf-devbar-space')).toBe('32px')
  release()
  expect(document.body.style.getPropertyValue('--wf-devbar-space')).toBe('')
})
