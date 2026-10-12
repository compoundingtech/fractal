import html from './index.html?raw'

export type ColdOpenControl = 'white-page' | 'missing-skeleton'
/** Exercise the actual pre-hydration document with application modules held back. */
export const coldOpenDocument = (control?: ColdOpenControl): string => {
  const document = new DOMParser().parseFromString(html, 'text/html')
  for (const node of document.querySelectorAll('script:not([data-wf-theme-bootstrap]), link')) node.remove()
  if (control === 'white-page') {
    document.querySelector('[data-wf-theme-bootstrap]')?.remove()
    document.querySelector('[data-wf-shell-bootstrap]')?.remove()
  }
  if (control === 'missing-skeleton') document.querySelector('[data-wf-shell-skeleton]')?.remove()
  return `<!doctype html>${document.documentElement.outerHTML}`
}
