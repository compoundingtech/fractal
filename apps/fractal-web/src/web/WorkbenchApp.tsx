import * as React from 'react'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import type { DataSource } from '../data/source.ts'
import { DevBar } from '../telemetry/DevBar.tsx'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

/**
 * The workbench against any data source: the live page binds its own runtime, and the scenario
 * mode binds the replay transport. The shell itself never selects a transport.
 */
export const WorkbenchApp = ({
  source,
  registry,
  ux,
  onCommit,
  onSelectConversation,
}: {
  readonly source: DataSource
  readonly registry: AtomRegistry.AtomRegistry
  readonly ux?: UxTelemetry
  readonly onCommit?: React.ProfilerOnRenderCallback
  readonly onSelectConversation?: (ref: string) => void
}) => (
  <>
    <DevBar />
    <DataSourceProvider source={source} registry={registry}>
      <React.Profiler id="workbench" onRender={onCommit ?? (() => {})}>
        <LiveAgentWorkspace ux={ux} onSelectConversation={onSelectConversation} />
      </React.Profiler>
    </DataSourceProvider>
  </>
)
