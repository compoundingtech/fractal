import * as React from 'react'

/** The roster's name for an agent ref, when it has one. Hosts with data provide it; without one, alerts keep the ref. */
export type NameOf = (ref: string) => string | undefined

export const AlertNames = React.createContext<NameOf>(() => undefined)
