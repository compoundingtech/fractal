import html from './index.html?raw'

export type ColdOpenControl = 'white-page' | 'missing-skeleton'
/** Exercise the actual pre-hydration document with application modules held back. */
export const coldOpenDocument = (control?: ColdOpenControl, scheme?: 'dark' | 'light'): string => {
  const document = new DOMParser().parseFromString(html, 'text/html')
  for (const node of document.querySelectorAll('script:not([data-wf-theme-bootstrap]), link')) node.remove()
  if (scheme !== undefined) {
    const bootstrap = document.querySelector('[data-wf-theme-bootstrap]')!
    const saved = JSON.stringify({ v: 1, t: 1, w: 'cold-open', d: scheme })
    bootstrap.textContent = `;(() => {
      const previous = localStorage.getItem('wf.c/round2.scheme')
      try {
        localStorage.setItem('wf.c/round2.scheme', ${JSON.stringify(saved)})
        ${bootstrap.textContent}
      } finally {
        if (previous === null) localStorage.removeItem('wf.c/round2.scheme')
        else localStorage.setItem('wf.c/round2.scheme', previous)
      }
    })()`
  }
  if (control === 'white-page') {
    document.querySelector('[data-wf-theme-bootstrap]')?.remove()
    document.querySelector('[data-wf-shell-bootstrap]')?.remove()
  }
  if (control === 'missing-skeleton') document.querySelector('[data-wf-shell-skeleton]')?.remove()
  return `<!doctype html>${document.documentElement.outerHTML}`
}
