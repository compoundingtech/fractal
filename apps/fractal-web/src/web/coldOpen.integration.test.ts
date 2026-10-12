// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import html from './index.html?raw'
import { coldOpenDocument, type ColdOpenControl } from './coldOpen.fixtures.ts'

const render = (saved?: string, control?: ColdOpenControl) => {
  localStorage.clear()
  if (saved !== undefined) localStorage.setItem('wf.ui@1', saved)
  const parsed = new DOMParser().parseFromString(coldOpenDocument(control), 'text/html')
  document.documentElement.innerHTML = parsed.documentElement.innerHTML
  document.documentElement.removeAttribute('style')
  delete document.documentElement.dataset.scheme
  const script = document.querySelector('[data-wf-theme-bootstrap]')?.textContent
  if (typeof script === 'string') new Function('document', 'localStorage', script)(document, localStorage)
  return document
}
const savedTheme = (value: unknown) => JSON.stringify({ version: 1, entries: { 'round2.scheme': { version: 1, value, clock: 1, writer: 'fixture' } } })
afterEach(() => { document.documentElement.innerHTML = '<head></head><body></body>'; document.documentElement.removeAttribute('style'); delete document.documentElement.dataset.scheme; localStorage.clear() })
describe('first-paint document', () => {
  it.each(['dark', 'light'] as const)('restores persisted %s before application code executes', scheme => {
    const document = render(savedTheme(scheme))
    expect(document.documentElement.dataset.scheme).toBe(scheme)
    expect(document.documentElement.style.colorScheme).toBe(scheme)
    expect(document.documentElement.style.getPropertyValue('--wf-boot-canvas')).toBe(scheme === 'dark' ? '#0a0a0a' : '#fcfcfc')
    expect(document.querySelectorAll('[data-wf-sidebar-skeleton] > div')).toHaveLength(8)
    expect(document.querySelector('[data-wf-shell-skeleton]')?.getAttribute('aria-busy')).toBe('true')
  })
  it.each([undefined, '{bad', savedTheme('unexpected')])('uses the product dark default for missing or unreadable preference', saved => {
    expect(render(saved).documentElement.dataset.scheme).toBe('dark')
  })
  it('does not block the shell paint on the application stylesheet', () => {
    const document = new DOMParser().parseFromString(html, 'text/html')
    expect(document.querySelector('#wf-app-style')?.getAttribute('media')).toBe('print')
    expect(document.querySelector('[data-wf-stylesheet-bootstrap]')?.textContent).toContain("sheet.media = 'all'")
  })
  it('rejects both original cold-open controls', () => {
    expect(render(savedTheme('dark'), 'white-page').documentElement.style.colorScheme).not.toBe('dark')
    expect(render(savedTheme('dark'), 'missing-skeleton').querySelector('[data-wf-sidebar-skeleton]')).toBeNull()
  })
})
