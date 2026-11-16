import * as React from 'react'
import { useAgent } from '../data/react.tsx'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { ConversationOverflowMenu } from './AgentActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'

/**
 * The conversation's one overflow menu in the responsive header. Status, sync and Resources live in the right panel.
 * With `onViewDeclaration` View KDL opens a tab; without it, the dialog.
 */
export const ConversationHeaderActions = ({ agentRef, onViewDeclaration }: {
  readonly agentRef: string
  readonly onViewDeclaration?: (() => void) | undefined
  /** Still passed by the workspace; both go with its call site when the tab shell lands. */
  readonly ux?: UxTelemetry
  readonly resourcesDock?: { readonly host: HTMLElement | null; readonly open: boolean; readonly onOpenChange: (open: boolean) => void }
}) => {
  const feed = useAgent(agentRef)
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  return <>
    <ConversationOverflowMenu agent={feed._tag === 'Observed' ? feed.value : undefined} onViewDeclaration={onViewDeclaration ?? (() => setDeclarationOpen(true))} />
    {onViewDeclaration === undefined ? <DeclarationDialog subject={agentRef} isOpen={declarationOpen} onOpenChange={setDeclarationOpen} /> : null}
  </>
}
