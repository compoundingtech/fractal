import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { borderVars, interactionVars, statusVars, surfaceVars, textVars, typeVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** Controller-owned state; this component cannot create or extend authority. */
export type ClientControlIndicatorState =
  | { readonly _tag: 'Inactive' }
  | { readonly _tag: 'Active'; readonly runId: string }
  | { readonly _tag: 'Revoking'; readonly runId: string }
  | { readonly _tag: 'RevokeFailed'; readonly runId: string }

export const ClientControlIndicator = ({ state, onRevokeEnrollment }: {
  readonly state: ClientControlIndicatorState
  /** End the enrollment and all grants, never just the displayed run's grant. */
  readonly onRevokeEnrollment: () => void
}) => {
  if (state._tag === 'Inactive') return null
  return <section aria-label="Test client control" data-client-control-run={state.runId} {...stylex.props(styles.indicator)}>
    <span role="status" {...stylex.props(styles.description)}>
      Test control <code {...stylex.props(styles.run)}>{state.runId}</code>
    </span>
    <Aria.Button onPress={onRevokeEnrollment} isDisabled={state._tag === 'Revoking'} {...stylex.props(styles.revoke)}>
      {state._tag === 'Revoking' ? 'Revoking enrollment…' : 'Revoke test enrollment'}
    </Aria.Button>
    {state._tag === 'RevokeFailed' && <span role="alert" {...stylex.props(styles.error)}>
      Enrollment revoke was not confirmed. Check the connection and try again.
    </span>}
  </section>
}

const styles = stylex.create({
  indicator: {
    display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 8,
    paddingBlock: 8, paddingInline: 12, backgroundColor: surfaceVars.raised,
    color: textVars.fg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: borderVars.borderStrong,
    fontFamily: typeVars.fontSans, fontSize: typeVars.metaSize, lineHeight: typeVars.metaLeading,
  },
  description: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', gap: 8, minWidth: 0 },
  run: { fontFamily: typeVars.fontMono, overflowWrap: 'anywhere', color: statusVars.attention },
  revoke: {
    flexShrink: 0, paddingBlock: 4, paddingInline: 8, cursor: 'pointer', color: textVars.fg,
    backgroundColor: surfaceVars.raised, borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.borderStrong,
    ':focus-visible': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: 2 },
    ':disabled': { cursor: 'default', opacity: 0.6 },
  },
  error: { flexBasis: '100%', color: statusVars.dangerFg },
})
