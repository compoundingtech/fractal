import * as React from 'react'
import { useAtomValue } from '@effect/atom-react'
import { DeclarationPanel } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx'
import { useDataSource } from '../data/react.tsx'
import { agentDeclarationState, declarationSubject, missionDeclarationState } from '../declarations/model.ts'

type DeclarationBodyProps = { readonly subject: string; readonly close: React.ReactNode }

/** The declaration sheet's on-demand body: the panel, its subject reads and redaction mapping.
 * A failing declaration render stays inside the sheet, which keeps its close action. */
export class DeclarationBody extends React.Component<DeclarationBodyProps, { readonly failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <><p role="alert">This declaration view could not load.</p>{this.props.close}</>
    return <DeclarationContents {...this.props} />
  }
}

const DeclarationContents = ({ subject, close }: DeclarationBodyProps) => {
  const parsed = declarationSubject(subject)
  return parsed._tag === 'Agent' ? <AgentDeclaration subject={subject} close={close} />
    : parsed._tag === 'Mission' ? <MissionDeclaration subject={subject} close={close} />
    : <DeclarationPanel title={subject} actions={close} state={{ _tag: 'Unavailable', message: 'Only agents and missions have a declaration to show.' }} />
}

const AgentDeclaration = ({ subject, close }: DeclarationBodyProps) => {
  const feed = useAtomValue(useDataSource().subjectReads.agentDeclaration.feed(subject))
  return <DeclarationPanel title={subject} actions={close} state={agentDeclarationState(feed)} />
}

const MissionDeclaration = ({ subject, close }: DeclarationBodyProps) => {
  const feed = useAtomValue(useDataSource().subjectReads.missionDefinition.feed(subject))
  return <DeclarationPanel title={subject} actions={close} state={missionDeclarationState(feed)} />
}
