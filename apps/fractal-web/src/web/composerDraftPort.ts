import { RegistryContext, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'

import type { ComposerDraftPort } from '../../../../packages/fractal-ui/src/assistant-ui/ComposerDraftSync.tsx'
import { useDataSource } from '../data/react.tsx'
import { cellComposerDraftStore } from '../state/cells/drafts.ts'
import { createComposerDraftSession } from './composerDrafts.ts'

/** Fixtures and scenarios have no signed-in person; their drafts belong to this browser. */
const browserPerson = Atom.make<string | undefined>('local')

/**
 * The conversation's draft as the composer's port, over the signed-in person's cells. Undefined
 * until discovery names the person, so nothing is stored under the wrong owner.
 */
export const useComposerDraftPort = (conversation: string): ComposerDraftPort | undefined => {
  const source = useDataSource()
  const registry = React.useContext(RegistryContext)
  const person = useAtomValue(source.person ?? browserPerson)
  const gateway = source.gateway ?? source.mode
  const [port, setPort] = React.useState<ComposerDraftPort | undefined>(undefined)
  // Layout effect: the restored draft is in the composer before the first paint.
  React.useLayoutEffect(() => {
    if (person === undefined) return
    const session = createComposerDraftSession({ store: cellComposerDraftStore({ registry, person, gateway }), conversationId: conversation })
    setPort({ text: () => session.getSnapshot().text, subscribe: session.subscribe, edit: (text) => { session.edit(text) } })
    return () => { session.close(); setPort(undefined) }
  }, [registry, person, gateway, conversation])
  return port
}
