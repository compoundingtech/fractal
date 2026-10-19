import { live, telemetry } from './liveRuntime.ts'
import * as React from 'react'
import { DataSourceProvider } from '../data/react.tsx'
import { DevBar } from '../telemetry/DevBar.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { UNSAFE_PortalProvider } from 'react-aria'
import { usePortalContainerRegistration } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'

export const App = () => {
  const overlayHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => overlayHost.current, [])
  usePortalContainerRegistration(getContainer)
  return (
    <UNSAFE_PortalProvider getContainer={getContainer}>
      <DevBar />
      <DataSourceProvider source={live.source} registry={live.registry}>
        <React.Profiler id="workbench" onRender={telemetry.onCommit}>
          <LiveAgentWorkspace ux={telemetry.ux} onSelectConversation={live.selectConversation} />
        </React.Profiler>
      </DataSourceProvider>
      <div ref={overlayHost} data-app-overlay-host="" />
    </UNSAFE_PortalProvider>
  )
}
