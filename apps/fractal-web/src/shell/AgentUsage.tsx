import * as stylex from '@stylexjs/stylex'
import { Schema } from 'effect'
import type { ReactNode } from 'react'
import { Button } from 'react-aria-components'
import { extensionViews } from '../extensions/build.ts'
import type { AgentRowProps } from '../extensions/contract.ts'
import { SubjectAddress } from '../shell/subjectAddress.ts'
import { AgentAttentionSignals } from './AgentAttention.tsx'
import type { WindowAction, Workspace } from './workspaces.ts'
import { geometryVars, interactionVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

const styles = stylex.create({
  row: { display: 'flex', width: '100%', textAlign: 'left', outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' }, outlineWidth: geometryVars.focusRing, outlineColor: interactionVars.focusRing, outlineOffset: geometryVars.focusInset },
})

/** Public attention survives without private spend attribution. */
export const AgentUsageSignals = extensionViews.AgentSignals ?? AgentAttentionSignals
export const AgentUsageRow = extensionViews.AgentRow ?? (({ onOpen, children }: AgentRowProps): ReactNode =>
  <Button onPress={onOpen} {...stylex.props(styles.row)}>{children}</Button>)

/** Uses the window-owned transition, never a second navigation state. */
export const openAgentUsage = ({ workspace, dispatch }: {
  readonly workspace: Pick<Workspace, 'id'>
  readonly dispatch: (action: WindowAction) => void
}): void => dispatch({
  _tag: 'Layout', workspace: workspace.id,
  action: { _tag: 'Open', input: Schema.decodeSync(SubjectAddress)({
    ref: workspace.id, presentation: extensionViews.AgentRow === undefined ? 'detail' : 'overview',
  }) },
})
