import * as stylex from '@stylexjs/stylex'
import { textVars as ink, borderVars as border, geometryVars as g, radiusVars as r, motionVars as m } from '../composition-tokens.stylex'

const spin = stylex.keyframes({ from: { transform: 'rotate(0deg)' }, to: { transform: 'rotate(360deg)' } })

/**
 * The one in-progress ring. Decorative: the label sits on adjacent text or the enclosing slot. `sm` suits dense lines and
 * keeps the 1px stroke. `spinning={false}` holds it still for a last-observed snapshot; reduced motion always does.
 * Unknown-duration loads wait `loadingGlyphDelayMs` before showing it; live agent activity shows it at once.
 */
export function LiveRing({ size = 'md', spinning = true }: { readonly size?: 'md' | 'sm'; readonly spinning?: boolean }) {
  return <span aria-hidden="true" data-live-ring={size} {...stylex.props(styles.ring, size === 'sm' && styles.sm, !spinning && styles.still)} />
}

const styles = stylex.create({
  ring: { display: 'block', width: g.status, height: g.status, boxSizing: 'border-box', flexShrink: 0, borderRadius: r.full, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderTopColor: ink.fgMuted, animationName: spin, animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
  sm: { width: g.statusSm, height: g.statusSm },
  still: { animationName: 'none' },
})
