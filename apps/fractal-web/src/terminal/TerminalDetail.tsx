import type { PtySize, PtyTarget } from '@st3/sdk/effect'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { Badge, type BadgeVariant, Note } from '../ui-compat/components.tsx'
import { scale, tokens } from '../ui-compat/tokens.stylex.ts'
import { incrDebugRuntime, RenderProfiler } from '../telemetry/meters.tsx'

import type { UnobservedFeed } from '../data/FeedState.tsx'
import { useDataSource, useFeedInterest, useTerminal, useTerminalConnected } from '../data/react.tsx'
import type { NativeSubjectProps } from '../resources/react.tsx'
import { DomTerminal } from './DomTerminal.tsx'
import { unavailableTerminalHistory } from './historySource.ts'
import { defaultTerminalPalette } from './palette.ts'
import { RawTerminalSurface, useRawTerminal } from './RawTerminal.tsx'
import { terminalChip, terminalPresentation, type TerminalChip } from './rawTerminalState.ts'
import { TerminalHistory } from './TerminalHistory.tsx'
import { TerminalPane, type PaneState } from './TerminalPane.tsx'

/** A terminal source publishes a human reason; a blank or placeholder detail falls back to fixed copy. */
export const terminalUnobservedCopy = (feed: UnobservedFeed): string => {
  if (feed._tag === 'Waiting') return 'Loading terminal…'
  const reason = feed.detail.trim()
  if (reason !== '' && !/^(undefined|unknown|null)$/i.test(reason)) return reason
  return feed.reason === 'ungranted'
    ? 'This terminal is not available to this view.'
    : feed.reason === 'unsupported'
      ? 'This gateway does not provide terminals.'
      : 'The terminal could not be opened.'
}

const chipVariant: Record<TerminalChip['tone'], BadgeVariant> = {
  live: 'green-subtle',
  pending: 'amber-subtle',
  readonly: 'gray-subtle',
  ended: 'red-subtle',
}

/** The terminal identity a raw session attaches to, pinned to the address it was observed under. */
interface PinnedTarget {
  readonly ref: string
  readonly target: PtyTarget
  readonly size: PtySize
  readonly title: string
}

/**
 * Native terminals are interactive through the raw PTY stream when the source offers it. The
 * projected screen supplies the terminal identity and is the read-only fallback when the stream is
 * refused or keeps failing; while the raw session is live the projected feed is not followed.
 */
export const TerminalDetail = React.memo(
  ({ address, visibility }: NativeSubjectProps) => {
    incrDebugRuntime('Fn.terminal')
    const connected = useTerminalConnected(address.ref)
    const source = useDataSource()
    const terminal = useTerminal(address.ref)
    const visible = visibility === 'visible'
    const observed = terminal._tag === 'Observed' ? terminal.value : undefined
    const [pinned, setPinned] = React.useState<PinnedTarget | undefined>(undefined)
    if (
      observed !== undefined &&
      (pinned?.ref !== address.ref ||
        pinned.target.terminalId !== observed.terminal_id ||
        pinned.target.incarnation !== observed.runtime_incarnation)
    )
      setPinned({
        ref: address.ref,
        target: { terminalId: observed.terminal_id, incarnation: observed.runtime_incarnation },
        size: { cols: observed.columns, rows: observed.rows },
        title: observed.title,
      })
    const current = pinned?.ref === address.ref ? pinned : undefined
    const [typing, setTyping] = React.useState(false)
    const raw = useRawTerminal({
      port: source.terminalRaw,
      target: current?.target,
      active: visible,
      size: current?.size ?? { cols: 80, rows: 24 },
      title: current?.title ?? 'Terminal',
    })
    const presentation = terminalPresentation(raw.preparation)
    const rawLive = presentation._tag === 'Raw' && raw.preparation._tag === 'Session' && raw.preparation.state._tag === 'Live'
    useFeedInterest({
      interest: source.terminalInterest?.(address.ref),
      // A stale fence or a lost stream re-resolves the terminal through the projected feed.
      visible: visible && !rawLive,
    })
    const chip = terminalChip({ raw: raw.preparation, presentation, typing })
    const controls = (
      <Badge variant={chipVariant[chip.tone]} size="sm" role="status" data-terminal-mode={presentation._tag}>
        {chip.label}
      </Badge>
    )
    const rawScreen = raw.snapshot?.screen
    if (presentation._tag === 'Raw' && raw.handle !== undefined && rawScreen !== undefined && raw.preparation._tag === 'Session') {
      const state = raw.preparation.state
      const paneState: PaneState =
        state._tag === 'Live'
          ? visible
            ? 'live'
            : 'idle'
          : state._tag === 'Connecting' && state.attempt === 1
            ? 'connecting'
            : state._tag === 'Exited' || state._tag === 'Closed'
              ? 'ended'
              : 'stale'
      return (
        <div {...stylex.props(styles.root)}>
          <RenderProfiler id="terminal">
            <TerminalPane screen={rawScreen} state={paneState} controls={controls}>
              <RawTerminalSurface
                handle={raw.handle}
                screen={rawScreen}
                palette={defaultTerminalPalette}
                live={state._tag === 'Live'}
                onTypingChange={setTyping}
              />
            </TerminalPane>
          </RenderProfiler>
        </div>
      )
    }
    if (terminal._tag !== 'Observed')
      return (
        <div {...stylex.props(styles.unobserved)}>
          <p role="status" data-terminal-state={terminal._tag === 'Waiting' ? 'loading' : terminal.reason}>
            {terminalUnobservedCopy(terminal)}
          </p>
          {terminal._tag === 'Unavailable' && terminal.retryable === true && source.retryTerminal !== undefined ? (
            <button type="button" onClick={() => source.retryTerminal?.(address.ref)}>Retry</button>
          ) : null}
        </div>
      )
    const screen = terminal.value
    return (
      <div {...stylex.props(styles.root)}>
        {terminal.freshness === 'stale' && (
          <p role="status" {...stylex.props(styles.stale)}>
            Terminal snapshot is stale.
          </p>
        )}
        <RenderProfiler id="terminal">
          <TerminalPane
            screen={screen}
            state={connected === false ? 'ended' : terminal.freshness !== 'live' ? 'stale' : visible ? 'live' : 'idle'}
            controls={controls}
            {...(presentation._tag === 'ReadOnly' && raw.preparation._tag !== 'Unsupported'
              ? { banner: <Note variant="warning" size="sm">{presentation.reason}</Note> }
              : {})}
          >
            <TerminalHistory
              port={(source.terminalHistory ?? unavailableTerminalHistory)(
                screen.terminal_id,
                screen.runtime_incarnation,
              )}
              palette={defaultTerminalPalette}
              incarnation={screen.runtime_incarnation}
              alternateScreen={screen.modes.alternate_screen}
            />
            <DomTerminal screen={screen} palette={defaultTerminalPalette} />
          </TerminalPane>
        </RenderProfiler>
      </div>
    )
  },
  (left, right) =>
    left.address.ref === right.address.ref &&
    left.address.presentation === right.address.presentation &&
    left.visibility === right.visibility,
)

const styles = stylex.create({
  root: {
    padding: scale.space2,
    display: 'flex',
    flexDirection: 'column',
    flexGrow: 1,
    minHeight: 0,
    minWidth: 0,
    overflow: 'auto',
  },
  stale: { padding: scale.space2, color: tokens['--ds-gray-900'], fontSize: '0.8125rem' },
  unobserved: { margin: 'auto', padding: scale.space4, color: tokens['--ds-gray-900'], fontSize: '0.8125rem' },
})
