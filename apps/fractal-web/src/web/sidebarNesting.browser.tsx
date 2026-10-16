import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { createRoot } from 'react-dom/client'
import { DataSourceProvider } from '../data/react.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { nestingFolderSource, nestingSource } from './sidebarNesting.fixture.ts'

createRoot(document.getElementById('root')!).render(
  <DataSourceProvider source={nestingSource} registry={AtomRegistry.make()}>
    <LiveAgentWorkspace folderSource={nestingFolderSource} />
  </DataSourceProvider>,
)
