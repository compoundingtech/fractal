import { useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'

import { useOptionalDataSource } from '../../data/react.tsx'
import { decisionWords } from '../../state/cells/drafts.ts'
import type { AlertAnswers } from './alertAnswers.ts'

const browserPerson = Atom.make<string | undefined>('local')
const unknownPerson = Atom.make('')
const noPerson = Atom.make<string | undefined>(undefined)

/**
 * A decision card's words, kept for the signed-in person across reloads and windows. Before
 * discovery names the person, or outside a data source, they live only in the answers store.
 */
export const useDecisionWords = (store: AlertAnswers, alertId: string): readonly [string, (words: string) => void] => {
  const source = useOptionalDataSource()
  const person = useAtomValue(source === undefined ? noPerson : source.person ?? browserPerson)
  const cell = source === undefined || person === undefined ? undefined : decisionWords(person, `${source.gateway ?? source.mode}/${alertId}`)
  const [stored, setStored] = useAtom(cell ?? unknownPerson)
  if (cell === undefined) return [store.words(alertId), (words) => store.setWords(alertId, words)]
  return [stored, (words) => { store.setWords(alertId, words); setStored(words) }]
}
