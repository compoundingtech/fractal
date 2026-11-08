import * as stylex from '@stylexjs/stylex'
import { textVars as ink, borderVars as border, geometryVars as g, radiusVars as r, motionVars as m } from '../composition-tokens.stylex'

const spin = stylex.keyframes({ from: { transform: 'rotate(0deg)' }, to: { transform: 'rotate(360deg)' } })

/** The in-progress ring shared by live rows; `spinning={false}` holds it still, and reduced motion always does. */
export function LiveRing({ spinning = true }: { readonly spinning?: boolean }) {
  return <span aria-hidden="true" data-live-ring {...stylex.props(styles.ring, !spinning && styles.still)} />
}

const styles = stylex.create({
  ring: { display: 'block', width: g.status, height: g.status, boxSizing: 'border-box', flexShrink: 0, borderRadius: r.full, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderTopColor: ink.fgMuted, animationName: spin, animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
  still: { animationName: 'none' },
})
