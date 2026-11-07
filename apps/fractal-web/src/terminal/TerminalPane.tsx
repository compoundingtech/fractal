/**
 * Session terminal pane chrome around any renderer: live state, title, geometry, incarnation, and
 * the protocol states a viewer must show (connecting, truncated screen, stale-fence end).
 */

import type { TerminalScreen } from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import { Button as AriaButton } from 'react-aria-components'

import {
  Badge,
  Note,
  StatusDot,
  Tooltip,
  TooltipTrigger,
} from '../ui-compat/components.tsx'

import { BrandText } from '../workbench-kit/brand-icons.tsx'
import { TerminalViewport } from './TerminalViewport.tsx'
import { borderVars, interactionVars, surfaceVars, textVars, radiusVars, spaceVars, typeVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** Viewer state of a pane: waiting for the first screen, following frames, quiet, showing an old snapshot, or frozen for good. */
export type PaneState = 'connecting' | 'live' | 'idle' | 'stale' | 'ended'

const statusOf = { connecting: 'building', live: 'ready', idle: 'queued', stale: 'queued', ended: 'error' } as const
const labelOf = { connecting: 'Connecting', live: 'Live', idle: 'Idle', stale: 'Stale', ended: 'Ended' } as const

const styles = stylex.create({
  root: {
    display: 'flex',
    flexDirection: 'column',
    minWidth: 0,
    flexGrow: 1,
    minHeight: 0,
    borderWidth: '1px',
    borderStyle: 'solid',
    borderColor: borderVars.borderStrong,
    borderRadius: 0,
    overflow: 'hidden',
    backgroundColor: surfaceVars.canvas,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    gap: spaceVars.md,
    paddingInline: spaceVars.md,
    height: '1.75rem',
    flexShrink: 0,
    borderBottomWidth: '1px',
    borderBottomStyle: 'solid',
    borderBottomColor: borderVars.borderStrong,
    backgroundColor: surfaceVars.raised,
    fontSize: '0.8125rem',
    color: textVars.fg,
  },
  title: {
    flexGrow: 1,
    minWidth: 0,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontWeight: 500,
  },
  meta: {
    borderWidth: 0,
    borderRadius: radiusVars.xs,
    paddingInline: spaceVars.xs,
    paddingBlock: 0,
    height: '1.5rem',
    backgroundColor: {
      default: 'transparent',
      ':is([data-hovered])': surfaceVars.message,
    },
    cursor: 'pointer',
    outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' },
    outlineWidth: '2px',
    outlineColor: interactionVars.focusRing,
    outlineOffset: '-2px',
    flexShrink: 0,
    whiteSpace: 'nowrap',
    fontFamily: typeVars.fontMono,
    fontSize: '0.75rem',
    color: textVars.fg,
  },
  banner: {
    padding: spaceVars.md,
  },
})

/** Pane chrome around a terminal renderer: status, title, geometry/incarnation inspector and an optional banner. */
export const TerminalPane = ({
  screen,
  state,
  banner,
  controls,
  onLatest,
  children,
}: {
  readonly screen: TerminalScreen | null
  readonly state: PaneState
  readonly banner?: ReactNode
  readonly controls?: ReactNode
  readonly onLatest?: () => void
  readonly children: ReactNode
}) => {
  return (
    <section
      aria-label={screen === null ? 'Terminal' : `Terminal ${screen.title}`}
      {...stylex.props(styles.root)}
    >
      <header {...stylex.props(styles.header)}>
        <StatusDot status={statusOf[state]} size="small" aria-label={labelOf[state]} />
        <span {...stylex.props(styles.title)}>
          <BrandText text={screen?.title ?? 'Terminal'} />
        </span>
        {controls}
        {screen === null ? null : (
          <>
            {screen.modes.alternate_screen ? (
              <Badge variant="gray-subtle" size="sm">
                alt screen
              </Badge>
            ) : null}
            {screen.truncated ? (
              <Badge variant="amber-subtle" size="sm">
                truncated
              </Badge>
            ) : null}
            <TooltipTrigger>
              <AriaButton
                aria-label={`Terminal inspection: ${screen.columns} columns by ${screen.rows} rows, runtime ${screen.runtime_incarnation}`}
                {...stylex.props(styles.meta)}
              >
                {screen.columns}×{screen.rows}
              </AriaButton>
              <Tooltip>Runtime {screen.runtime_incarnation}</Tooltip>
            </TooltipTrigger>
          </>
        )}
      </header>
      {banner === undefined ? null : <div {...stylex.props(styles.banner)}>{banner}</div>}
      <TerminalViewport ended={state === 'ended'} {...(onLatest === undefined ? {} : { onLatest })}>
        {children}
      </TerminalViewport>
    </section>
  )
}

/** Banner for a pane ended by a stale fence: the process behind the terminal ID was replaced. */
export const StaleFenceBanner = () => (
  <Note variant="error" size="sm">
    Terminal restarted (stale-fence). The last screen is frozen; reopen the terminal to follow the
    new process.
  </Note>
)

/** The host running the PTY stopped heartbeating: the pane keeps its last screen and takes no input. */
export const HostDisconnectedBanner = ({
  host,
  lastSeen,
}: {
  readonly host: string
  readonly lastSeen: string
}) => (
  <Note variant="warning" size="sm">
    {host} is disconnected (last heartbeat {lastSeen.slice(11, 16)} UTC). The last screen is frozen;
    input resumes when the host reconnects.
  </Note>
)

/** Banner for a screen the gateway truncated. */
export const TruncatedBanner = () => (
  <Note variant="warning" size="sm">
    The gateway truncated this screen; some rows or row tails are not shown.
  </Note>
)
