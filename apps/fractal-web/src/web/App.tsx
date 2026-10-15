import { live, telemetry } from './liveRuntime.ts'
import * as React from 'react'
import { DataSourceProvider } from '../data/react.tsx'
import { DevBar } from '../telemetry/DevBar.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { DiagnosticsBoundary } from './DiagnosticsBoundary.tsx'

export const App = () => (
  <>
    <DiagnosticsBoundary>
      <DevBar />
    </DiagnosticsBoundary>
    <DataSourceProvider source={live.source} registry={live.registry}>
      <React.Profiler id="workbench" onRender={telemetry.onCommit}>
        <LiveAgentWorkspace ux={telemetry.ux} onSelectConversation={live.selectConversation} />
      </React.Profiler>
    </DataSourceProvider>
  </>
)
