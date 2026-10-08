import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { setTheme } from '../ui-compat/foundations.ts'

import { WorkbenchApp } from './WorkbenchApp.tsx'
import { scenario } from './scenarioRuntime.ts'

setTheme('system')

const rootElement = document.getElementById('root')
if (rootElement === null) {
  throw new Error('Root element not found')
}

const root = createRoot(rootElement)
void scenario.ready.then(() => {
  root.render(
    <StrictMode>
      <WorkbenchApp
        source={scenario.source}
        registry={scenario.registry}
        onSelectConversation={scenario.selectConversation}
      />
    </StrictMode>,
  )
})
