import * as React from 'react'
import { useAtom } from '@effect/atom-react'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { persistedAtom } from '../state/persistence.ts'
import { subjectAgent, type Subject } from '../workspace/subject.ts'
import type { InspectorView } from './AgentDetailsPanel.tsx'

/** A side panel's last tab, per kind: an agent's panel reopens on the tab last used for any agent. */
export const asideTabAtom = Atom.family((kind: string) => persistedAtom({ key: `aside.tab.${kind}`, schema: Schema.String, defaultValue: '' }))

export interface AsidePanelProps {
  readonly subject: Subject
  /** `dock` beside the content in its pane; `fill` in the phone's bottom sheet. */
  readonly width: 'dock' | 'fill'
  readonly scheme: 'dark' | 'light'
  /** Opens a ref as a workspace tab; `split` (Cmd or Ctrl) puts it in a new pane beside this one. */
  readonly onOpen: (ref: string, gesture?: { readonly split: 'right' }) => void
  readonly onClose: () => void
}

const AgentDetailsPanel = React.lazy(() => import('./AgentDetailsPanel.tsx').then(module => ({ default: module.AgentDetailsPanel })))
const views: readonly InspectorView[] = ['details', 'changes', 'resources']

/** An agent's panel: Details, Changes and Resources for the agent this pane shows. */
export const AgentAside = ({ subject, width, scheme, onOpen, onClose }: AsidePanelProps) => {
  const agentRef = subjectAgent(subject)
  const [stored, setStored] = useAtom(asideTabAtom('agent'))
  if (agentRef === undefined) return null
  const view = views.find(each => each === stored) ?? 'details'
  return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading agent details" />}>
    <AgentDetailsPanel key={agentRef} agentRef={agentRef} width={width} scheme={scheme} view={view} onViewChange={setStored} onClose={onClose} onOpen={(uri, split) => onOpen(uri.startsWith('agent:') ? `agent/${uri.slice('agent:'.length)}` : uri, split ? { split: 'right' } : undefined)} />
  </React.Suspense>
}
