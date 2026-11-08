import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { AgentLiveState, StepState } from './detailModel.ts'

// The mission detail's small shared parts. Colour marks only what needs the person (amber) or failed (red); every other
// state is neutral ink and differs by shape, so no state is told by colour alone.

export const STATE_LABEL: Record<StepState, string> = { done: 'Done', running: 'Running', failed: 'Failed', 'waiting-you': 'Needs you', waiting: 'Blocked', ready: 'Ready', pending: 'Not started', cancelled: 'Removed' }

/** Filled when done, an arc while running, an amber diamond when it needs you, a red cross when it failed, hollow otherwise. */
export const StepMark = ({ state, size = 10 }: { readonly state: StepState; readonly size?: number }) => {
  const c = size / 2
  const rad = size / 2 - 1
  return <svg aria-hidden="true" data-step-mark={state} width={size} height={size} viewBox={`0 0 ${size} ${size}`} {...stylex.props(styles.mark)}>
    {state === 'done' ? <circle cx={c} cy={c} r={rad} {...stylex.props(styles.fillSoft)} />
      : state === 'running' ? <><circle cx={c} cy={c} r={rad} fill="none" strokeWidth={1.5} {...stylex.props(styles.strokeFaint)} /><path d={`M${c} ${c - rad} A${rad} ${rad} 0 0 1 ${c + rad} ${c}`} fill="none" strokeWidth={1.5} {...stylex.props(styles.strokeInk)} /></>
        : state === 'failed' ? <><circle cx={c} cy={c} r={rad} {...stylex.props(styles.fillBad)} /><path d={`M${c - rad / 2} ${c - rad / 2} L${c + rad / 2} ${c + rad / 2} M${c + rad / 2} ${c - rad / 2} L${c - rad / 2} ${c + rad / 2}`} strokeWidth={1.25} {...stylex.props(styles.strokeCanvas)} /></>
          : state === 'waiting-you' ? <rect x={c - rad * 0.75} y={c - rad * 0.75} width={rad * 1.5} height={rad * 1.5} transform={`rotate(45 ${c} ${c})`} {...stylex.props(styles.fillYou)} />
            : state === 'waiting' ? <><circle cx={c} cy={c} r={rad} fill="none" strokeWidth={1.25} {...stylex.props(styles.strokeMuted)} /><path d={`M${c - rad / 2} ${c} L${c + rad / 2} ${c}`} strokeWidth={1.25} {...stylex.props(styles.strokeMuted)} /></>
              : <circle cx={c} cy={c} r={rad} fill="none" strokeWidth={1.25} strokeDasharray={state === 'cancelled' ? '2 2' : undefined} {...stylex.props(styles.strokeMuted)} />}
  </svg>
}

export const AGENT_LABEL: Record<AgentLiveState, string> = { working: 'working', idle: 'idle', 'needs-you': 'needs you', failed: 'failed', stopped: 'stopped', unknown: 'state unknown' }

/** An agent's live state as a ring: spinning arc while working, amber or red filled when it needs you or failed, dashed when stopped. */
export const AgentRing = ({ state }: { readonly state: AgentLiveState }) =>
  <svg aria-hidden="true" data-agent-ring={state} width={12} height={12} viewBox="0 0 12 12" {...stylex.props(styles.mark)}>
    {state === 'working' ? <><circle cx={6} cy={6} r={4.5} fill="none" strokeWidth={1.5} {...stylex.props(styles.strokeFaint)} /><path d="M6 1.5 A4.5 4.5 0 0 1 10.5 6" fill="none" strokeWidth={1.5} {...stylex.props(styles.strokeInk, styles.spin)} /></>
      : state === 'needs-you' ? <circle cx={6} cy={6} r={4.5} {...stylex.props(styles.fillYou)} />
        : state === 'failed' ? <circle cx={6} cy={6} r={4.5} {...stylex.props(styles.fillBad)} />
          : <circle cx={6} cy={6} r={4.5} fill="none" strokeWidth={1.5} strokeDasharray={state === 'stopped' || state === 'unknown' ? '2 2' : undefined} {...stylex.props(styles.strokeMuted)} />}
  </svg>

/** A press that opens a ref, in a new pane beside this one with Cmd or Ctrl. */
export type OpenRef = (ref: string, gesture?: { readonly background?: boolean; readonly split?: 'right' | 'below' }) => void
export const openGesture = (event: { readonly metaKey: boolean; readonly ctrlKey: boolean }) => event.metaKey || event.ctrlKey ? { background: true } : undefined

export const SmallButton = ({ label, onPress, children, pressed, isDisabled }: { readonly label: string; readonly onPress: () => void; readonly children: React.ReactNode; readonly pressed?: boolean; readonly isDisabled?: boolean }) =>
  <Aria.Button aria-label={label} aria-pressed={pressed} onPress={onPress} isDisabled={isDisabled ?? false} {...stylex.props(styles.small, pressed === true && styles.smallPressed)}>{children}</Aria.Button>

export const SrOnly = ({ children }: { readonly children: React.ReactNode }) => <span {...stylex.props(styles.srOnly)}>{children}</span>

/** A tab row: Aria tabs over the kit's quiet underline. */
export const TabRow = <T extends string>({ label, tabs, selected, onChange, children }: {
  readonly label: string
  readonly tabs: ReadonlyArray<{ readonly id: T; readonly label: string; readonly count?: number | undefined }>
  readonly selected: T
  readonly onChange: (id: T) => void
  readonly children: (id: T) => React.ReactNode
}) => <Aria.Tabs selectedKey={selected} onSelectionChange={key => onChange(String(key) as T)} {...stylex.props(styles.tabs)}>
  <Aria.TabList aria-label={label} {...stylex.props(styles.tabList)}>
    {tabs.map(tab => <Aria.Tab key={tab.id} id={tab.id} {...stylex.props(styles.tab)}>{tab.label}{tab.count !== undefined && tab.count > 0 ? <span {...stylex.props(styles.count)}>{tab.count}</span> : null}</Aria.Tab>)}
  </Aria.TabList>
  {tabs.map(tab => <Aria.TabPanel key={tab.id} id={tab.id} {...stylex.props(styles.tabPanel)}>{children(tab.id)}</Aria.TabPanel>)}
</Aria.Tabs>

const spin = stylex.keyframes({ from: { transform: 'rotate(0deg)' }, to: { transform: 'rotate(360deg)' } })

export const styles = stylex.create({
  mark: { flexShrink: 0, display: 'block', overflow: 'visible' },
  fillSoft: { fill: ink.fgSoft },
  fillYou: { fill: tone.attention },
  fillBad: { fill: tone.dangerFg },
  strokeInk: { stroke: ink.fg },
  strokeFaint: { stroke: border.rail },
  strokeMuted: { stroke: ink.fgMuted },
  strokeCanvas: { stroke: surface.canvas },
  spin: { transformOrigin: '6px 6px', animationName: spin, animationDuration: '1.2s', animationIterationCount: 'infinite', animationTimingFunction: 'linear', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
  small: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: s.xs, minWidth: g.controlSm, height: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.denseSize, cursor: { default: 'pointer', ':is([data-disabled])': 'default' }, opacity: { default: 1, ':is([data-disabled])': 0.5 }, ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  smallPressed: { backgroundColor: surface.rowActive },
  srOnly: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  tabs: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  tabList: { display: 'flex', gap: s.lg, paddingInline: s.xl, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  tab: { display: 'inline-flex', alignItems: 'center', gap: s.xs, paddingBlock: s.sm, marginBlockEnd: '-1px', borderBottomWidth: '2px', borderBottomStyle: 'solid', borderBottomColor: { default: 'transparent', ':is([data-selected])': ink.fg }, color: { default: ink.fgMuted, ':is([data-selected])': ink.fg, ':hover': ink.fg }, fontSize: t.metaSize, fontWeight: 500, cursor: 'pointer', outline: 'none', ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  count: { minWidth: '16px', paddingInline: s.xs, borderRadius: r.full, backgroundColor: surface.rowActive, color: ink.fgSoft, fontSize: t.denseSize, textAlign: 'center' },
  tabPanel: { minWidth: 0, outline: 'none' },
})
