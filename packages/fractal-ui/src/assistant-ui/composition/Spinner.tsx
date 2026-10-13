import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { geometryVars as g, motionVars as m } from '../composition-tokens.stylex'
import { pauseOffscreen } from './pause-offscreen'

/** S1: a compositor-only turn, paused off screen; M2 is the reduced-motion frame. */
export function Spinner({ size, scale = 'normal', label }: { readonly size?: number; readonly scale?: 'normal' | 'status'; readonly label?: string }) {
  return <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={4 / 3} strokeLinecap="round" strokeLinejoin="round"
    aria-hidden={label === undefined ? true : undefined} role={label === undefined ? undefined : 'img'} aria-label={label} ref={pauseOffscreen}
    {...stylex.props(styles.icon, scale === 'status' && styles.status, size !== undefined && styles.size(size), styles.spin)}>
    <path d="M14 8a6 6 0 1 1-4.146-5.707" />
  </svg>
}

const styles = stylex.create({
  icon: { width: g.icon, height: g.icon, flexShrink: 0 },
  status: { width: g.status, height: g.status },
  size: (size: number) => ({ width: size, height: size }),
  spin: { animationName: stylex.keyframes({ to: { transform: 'rotate(360deg)' } }), animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
})
