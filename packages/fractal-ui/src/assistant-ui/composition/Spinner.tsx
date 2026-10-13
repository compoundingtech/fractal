import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { geometryVars as g, motionVars as m } from '../composition-tokens.stylex'
import { pauseOffscreen } from './pause-offscreen'

/** S1: a compositor-only turn, paused off screen; M2 is the reduced-motion frame. */
export function Spinner({ size, scale = 'normal', label }: { readonly size?: number; readonly scale?: 'normal' | 'status'; readonly label?: string }) {
  return <span {...stylex.props(styles.box, scale === 'status' && styles.status, size !== undefined && styles.size(size))}>
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={4 / 3} strokeLinecap="round" strokeLinejoin="round"
      aria-hidden={label === undefined ? true : undefined} role={label === undefined ? undefined : 'img'} aria-label={label} ref={pauseOffscreen}
      {...stylex.props(styles.glyph, styles.spin)}>
      <path d="M14 8a6 6 0 1 1-4.146-5.707" />
    </svg>
  </span>
}

const styles = stylex.create({
  status: { width: g.status, height: g.status },
  box: { display: 'inline-flex', width: g.icon, height: g.icon, flexShrink: 0 },
  glyph: { display: 'block', width: '100%', height: '100%' },
  size: (size: number) => ({ width: size, height: size }),
  spin: { animationName: stylex.keyframes({ to: { transform: 'rotate(360deg)' } }), animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
})
