import * as React from 'react'

import { useSubjectList } from '../../data/react.tsx'
import { AlertNames, type NameOf } from './alertNames.ts'

/** Names alert senders as the roster does: an agent ref becomes its title. */
export const LiveAlertNames = ({ children }: { readonly children: React.ReactNode }): React.ReactNode => {
  const subjects = useSubjectList()
  const nameOf = React.useMemo<NameOf>(() => {
    const titles = new Map(subjects.map((subject) => [subject.ref, subject.title]))
    return (ref) => titles.get(ref)
  }, [subjects])
  return <AlertNames.Provider value={nameOf}>{children}</AlertNames.Provider>
}
