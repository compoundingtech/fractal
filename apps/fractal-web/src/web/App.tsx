import { live, telemetry } from './liveRuntime.ts'
import * as React from 'react'
import { DataSourceProvider } from '../data/react.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { UNSAFE_PortalProvider } from 'react-aria'
import { usePortalContainerRegistration } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import { DiagnosticsBoundary } from './DiagnosticsBoundary.tsx'

// A stored preference must never pull diagnostic modules into a production build.
const Diagnostics = import.meta.env.DEV || import.meta.env.PERF
  ? React.lazy(() => import('../telemetry/DevBar.tsx').then(({ DevBar }) => ({ default: DevBar })))
  : undefined

export const App = () => {
  const overlayHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => overlayHost.current, [])
  usePortalContainerRegistration(getContainer)
  return (
    <UNSAFE_PortalProvider getContainer={getContainer}>
      {Diagnostics !== undefined && (
        <DiagnosticsBoundary>
          <React.Suspense fallback={null}>
            <Diagnostics defaultVisible />
          </React.Suspense>
        </DiagnosticsBoundary>
      )}
      <DataSourceProvider source={live.source} registry={live.registry}>
        <React.Profiler id="workbench" onRender={telemetry.onCommit}>
          <LiveAgentWorkspace ux={telemetry.ux} onSelectConversation={live.selectConversation} />
        </React.Profiler>
      </DataSourceProvider>
      <div ref={overlayHost} data-app-overlay-host="" />
    </UNSAFE_PortalProvider>
  )
}
