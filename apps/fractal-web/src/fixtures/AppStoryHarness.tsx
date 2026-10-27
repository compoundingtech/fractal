import * as React from 'react'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { createAppStoryFixture, type AppStoryFixture, type AppStoryState } from './appStoryStates.ts'

export interface AppStoryArgs {
  readonly state: AppStoryState
  readonly scheme: 'dark' | 'light'
}

interface SpecimenProps {
  readonly state: AppStoryState
  readonly children: (fixture: AppStoryFixture) => React.ReactNode
}

const Specimen = ({ state, children }: SpecimenProps) => {
  const [fixture] = React.useState(() => createAppStoryFixture(state))
  const [registry] = React.useState(() => AtomRegistry.make())
  return <div data-app-state={state} style={{ display: 'contents' }}>
    <DataSourceProvider source={fixture.source} registry={registry}>{children(fixture)}</DataSourceProvider>
  </div>
}

/**
 * Exactly one production boundary per story document, over its own fixture source and registry;
 * changing the state control remounts it. The marker carries the state for the play assertion
 * and is never evidence by itself.
 */
export const AppStorySpecimen = (props: SpecimenProps) => <Specimen key={props.state} {...props} />
