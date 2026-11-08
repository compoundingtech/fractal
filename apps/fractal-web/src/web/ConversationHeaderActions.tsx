import * as React from 'react'
import { useAgent } from '../data/react.tsx'
import { ConversationOverflowMenu, type OverflowSection } from './AgentActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'

/**
 * The conversation's one overflow menu in the responsive header. Status, sync and Resources live in the side panel.
 * With `onViewDeclaration` View KDL opens a tab; without it, the dialog. `sections` follow the agent's own actions.
 */
export const ConversationHeaderActions = ({ agentRef, onViewDeclaration, sections }: {
  readonly agentRef: string
  readonly onViewDeclaration?: (() => void) | undefined
  readonly sections?: readonly OverflowSection[] | undefined
}) => {
  const feed = useAgent(agentRef)
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  return <>
    <ConversationOverflowMenu agent={feed._tag === 'Observed' ? feed.value : undefined} onViewDeclaration={onViewDeclaration ?? (() => setDeclarationOpen(true))} {...(sections === undefined ? {} : { sections })} />
    {onViewDeclaration === undefined ? <DeclarationDialog subject={agentRef} isOpen={declarationOpen} onOpenChange={setDeclarationOpen} /> : null}
  </>
}
