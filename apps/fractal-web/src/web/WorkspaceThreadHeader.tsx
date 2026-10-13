import * as React from 'react'
import { VisuallyHidden } from 'react-aria-components'
import { ThreadHeader } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { useConversationSync, useNow } from '../data/react.tsx'
import { syncLine } from '@smalltalk/fractal-ui/assistant-ui/sync-status'

/** Keep sync discoverable without spending another band on transport metadata. */
export const WorkspaceThreadHeader = ({ agentRef, ...props }: React.ComponentProps<typeof ThreadHeader> & { readonly agentRef: string }) => {
  const observation = useConversationSync(agentRef)
  const now = useNow()
  const descriptionId = React.useId()
  const description = observation === undefined ? 'Conversation synchronization pending' : syncLine({ ...observation, label: 'conversation', now })?.text ?? 'Conversation is up to date'
  return <div role="group" aria-label="Conversation header" aria-describedby={descriptionId} style={{ display: 'contents' }}><ThreadHeader {...props} layout="status-beside-title" /><VisuallyHidden><span id={descriptionId}>{description}</span></VisuallyHidden></div>
}
