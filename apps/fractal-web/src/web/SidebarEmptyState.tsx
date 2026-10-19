// RM1 roster states (Loading, EmptyRoster, NoMatch, RosterError, Stale): one inline message at the top of the list,
// fixed copy only; source diagnostics stay in data-wf-* attributes.
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { ObservedAge } from './lastObserved.tsx'

/** Screen readers hear the wait; the skeleton beside it is decorative. */
export const RosterLoading = () =>
  <p role="status" data-roster-notice="loading" data-system-state="loading" {...stylex.props(styles.visuallyHidden)}>Loading agents…</p>

/** An observed roster with no agents, or a filter that hides every observed agent. */
export const SidebarEmptyState = (props: { readonly kind: 'empty' } | { readonly kind: 'no-match'; readonly query: string; readonly onClear: () => void }) =>
  props.kind === 'empty'
    ? <div data-roster-empty {...stylex.props(styles.message)}>
      <p role="status" {...stylex.props(styles.messageTitle)}>No agents yet</p>
      <p {...stylex.props(styles.messageBody)}>Agents appear here as soon as one starts.</p>
    </div>
    : <div data-roster-no-match {...stylex.props(styles.message)}>
      <p role="status" {...stylex.props(styles.messageTitle)}>{props.query.trim() === '' ? 'No agents match these filters' : `No agents match “${props.query.trim()}”`}</p>
      <Aria.Button onPress={props.onClear} {...stylex.props(styles.actionButton)}>Clear filter</Aria.Button>
    </div>

const unavailableCopy = {
  failed: { title: 'Couldn’t load agents', body: 'The agent list did not load. Open conversations keep working.' },
  ungranted: { title: 'No access to the agent roster', body: 'Read access has not been granted.' },
  unsupported: { title: 'Agent roster not supported', body: 'This connection does not offer it.' },
} as const

/**
 * The roster read itself failed. Only a failed follow is announced as an alert in the danger tone; a refused or
 * unsupported read is a standing fact. Retry renders only when the source supports it.
 */
export const RosterUnavailable = ({ reason, onRetry }: { readonly reason: keyof typeof unavailableCopy; readonly onRetry: (() => void) | undefined }) =>
  <div role={reason === 'failed' ? 'alert' : 'status'} data-roster-error data-system-state="error" data-wf-roster-reason={reason} {...stylex.props(styles.message, styles.messageStart)}>
    <p {...stylex.props(styles.messageTitle, reason === 'failed' && styles.errorTitle)}>{reason === 'failed' && <Icon name="alert" size={14} />}{unavailableCopy[reason].title}</p>
    <p {...stylex.props(styles.messageBody)}>{unavailableCopy[reason].body}</p>
    {onRetry === undefined ? null : <div {...stylex.props(styles.actions)}><Aria.Button onPress={onRetry} {...stylex.props(styles.actionButton)}>Retry</Aria.Button></div>}
  </div>

/**
 * A last-observed roster: the age counts from the source's last actual observation on the shared clock, and
 * nothing here reads as live. A failed newer read keeps these rows and says so, with Retry when supported.
 */
export const RosterStaleNotice = ({ observedAt, partial, refreshFailed, onRetry }: {
  readonly observedAt: number | undefined
  readonly partial: boolean
  readonly refreshFailed: boolean
  readonly onRetry: (() => void) | undefined
}) => <>
  <p role="status" data-roster-notice="stale" data-system-state="stale" {...stylex.props(styles.notice)}>
    <Icon name="clock" size={12} />
    <span>{observedAt === undefined
      ? partial ? 'Partial roster' : 'Last observed list'
      : <ObservedAge prefix={partial ? 'Partial roster · last observed ' : 'Last observed '} at={observedAt} />}</span>
  </p>
  {refreshFailed ? <div role="alert" data-roster-error data-system-state="error" data-wf-roster-reason="failed" {...stylex.props(styles.message, styles.messageStart)}>
    <p {...stylex.props(styles.messageTitle, styles.errorTitle)}><Icon name="alert" size={14} />Couldn’t refresh agents</p>
    <p {...stylex.props(styles.messageBody)}>The list shows the last observation. Open conversations keep working.</p>
    {onRetry === undefined ? null : <div {...stylex.props(styles.actions)}><Aria.Button onPress={onRetry} {...stylex.props(styles.actionButton)}>Retry</Aria.Button></div>}
  </div> : null}
</>

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  notice: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.controlSm, margin: 0, paddingInline: s.md, color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  message: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.xs, margin: 0, paddingInline: s.md, paddingBlock: s.lg },
  messageStart: { paddingBlockStart: s.md },
  messageTitle: { display: 'flex', alignItems: 'center', gap: s.sm, margin: 0, color: ink.fg, fontSize: t.metaSize, fontWeight: t.weightMedium },
  errorTitle: { color: tone.dangerFg },
  messageBody: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading },
  actions: { display: 'flex', alignItems: 'flex-start', gap: s.md, marginBlockStart: s.xs },
  actionButton: { height: g.controlSm, paddingInline: s.md, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  visuallyHidden: { position: 'absolute', width: '1px', height: '1px', margin: '-1px', padding: 0, overflow: 'hidden', clip: 'rect(0 0 0 0)', whiteSpace: 'nowrap', borderWidth: 0 },
})
