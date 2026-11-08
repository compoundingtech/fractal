import * as React from 'react'
import { useAgent } from '../data/react.tsx'
import { ConversationOverflowMenu } from './AgentActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'

/**
 * The conversation's one overflow menu in the responsive header. Status, sync and Resources live in the right panel.
 * With `onViewDeclaration` View KDL opens a tab; without it, the dialog.
 */
export const ConversationHeaderActions = ({ agentRef, onViewDeclaration }: {
  readonly agentRef: string
  readonly onViewDeclaration?: (() => void) | undefined
}) => {
  const feed = useAgent(agentRef)
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  return <>
    <ConversationOverflowMenu agent={feed._tag === 'Observed' ? feed.value : undefined} onViewDeclaration={onViewDeclaration ?? (() => setDeclarationOpen(true))} />
    {onViewDeclaration === undefined ? <DeclarationDialog subject={agentRef} isOpen={declarationOpen} onOpenChange={setDeclarationOpen} /> : null}
  </>
}
