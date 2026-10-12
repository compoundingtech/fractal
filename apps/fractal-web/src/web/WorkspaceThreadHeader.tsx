import * as React from 'react'
import { ThreadHeader } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { useConversationSync, useNow } from '../data/react.tsx'
import { syncLine } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/sync-line.ts'

/** Keep sync discoverable without spending another band on transport metadata. */
export const WorkspaceThreadHeader = ({ agentRef, ...props }: React.ComponentProps<typeof ThreadHeader> & { readonly agentRef: string }) => {
  const observation = useConversationSync(agentRef)
  const now = useNow()
  const description = observation === undefined ? 'Conversation synchronization pending' : syncLine({ ...observation, label: 'conversation', now })?.text ?? 'Conversation is up to date'
  return <div data-workspace-header title={description} style={{ display: 'contents' }}><ThreadHeader {...props} /></div>
}
