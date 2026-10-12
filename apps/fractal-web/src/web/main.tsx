import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { setTheme } from '../ui-compat/foundations.ts'

import { App } from './App.tsx'
import { live } from './liveRuntime.ts'
import { installPageLifecycle } from './pageLifecycle.ts'

declare global {
  interface Window {
    __wfStylexReady?: Promise<void>
  }
}

setTheme(document.documentElement.dataset.scheme === 'light' ? 'light' : 'dark')

const rootElement = document.getElementById('root')
if (rootElement === null) {
  throw new Error('Root element not found')
}

const root = createRoot(rootElement)
const appStylesheet = document.getElementById('wf-app-style')
const appStylesReady = appStylesheet instanceof HTMLLinkElement && appStylesheet.sheet === null
  ? new Promise<void>(resolve => appStylesheet.addEventListener('load', () => resolve(), { once: true }))
  : Promise.resolve()
// The dev runtime replaces its link with an inline stylesheet; link.sheet is no longer a readiness signal.
const stylesReady = Promise.all([appStylesReady, window.__wfStylexReady])
let disposed = false
installPageLifecycle({
  target: window,
  source: live,
  beforeDispose: () => {
    disposed = true
    root.unmount()
  },
})

void Promise.all([live.ready, stylesReady]).then(() => {
  if (disposed) return
  root.render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
})
