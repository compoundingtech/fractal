import { live, telemetry } from './liveRuntime.ts'
import { WorkbenchApp } from './WorkbenchApp.tsx'

export const App = () => (
  <WorkbenchApp
    source={live.source}
    registry={live.registry}
    ux={telemetry.ux}
    onCommit={telemetry.onCommit}
    onSelectConversation={live.selectConversation}
  />
)
