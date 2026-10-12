import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { iconVars as icon, motionVars as m } from '../composition-tokens.stylex'
import { glyphs, iconRegistry, type IconName, type IconSize } from './icon-registry'

export type { IconName, IconSize } from './icon-registry'

/**
 * One semantic icon: the name is the meaning, the registry picks the glyph and its token size.
 * Glyphs draw on a 24-unit grid; the stroke stays at the stroke token on screen at every size.
 */
export function Icon({ name, size, label }: { name: IconName; size?: IconSize; label?: string }) {
  const entry = iconRegistry[name]
  const resolved = size ?? entry.size
  const dots = entry.paint === 'dots'
  return <svg
    viewBox="0 0 24 24"
    fill={entry.paint === 'filled' ? 'currentColor' : 'none'}
    stroke="currentColor"
    strokeLinecap="round"
    strokeLinejoin="round"
    data-icon={name}
    data-icon-size={resolved}
    aria-hidden={label === undefined ? true : undefined}
    role={label === undefined ? undefined : 'img'}
    aria-label={label}
    {...stylex.props(styles.icon, sizeStyles[resolved], entry.paint === 'spin' && styles.spin)}
  >{glyphs[entry.glyph].map(([tag, attributes], index) => React.createElement(tag, { key: index, ...attributes, ...dots && tag === 'circle' ? stylex.props(styles.dot, dotStyles[resolved]) : undefined }))}</svg>
}

// A CSS stroke width is in grid units, so scale the on-screen token by grid / size.
const sizes = stylex.create({
  size12: { width: icon.size12, height: icon.size12, strokeWidth: `calc(${icon.stroke} * 24 / 12)` },
  size14: { width: icon.size14, height: icon.size14, strokeWidth: `calc(${icon.stroke} * 24 / 14)` },
  size16: { width: icon.size16, height: icon.size16, strokeWidth: `calc(${icon.stroke} * 24 / 16)` },
})
// Dots are filled discs whose radius in grid units keeps the dot token's diameter on screen.
const dotRadii = stylex.create({
  size12: { r: `calc(${icon.dot} * 12 / 12)` },
  size14: { r: `calc(${icon.dot} * 12 / 14)` },
  size16: { r: `calc(${icon.dot} * 12 / 16)` },
})
const sizeStyles: Record<IconSize, typeof sizes.size12> = { 12: sizes.size12, 14: sizes.size14, 16: sizes.size16 }
const dotStyles: Record<IconSize, typeof dotRadii.size12> = { 12: dotRadii.size12, 14: dotRadii.size14, 16: dotRadii.size16 }
const styles = stylex.create({
  icon: { flexShrink: 0 },
  dot: { fill: 'currentColor', stroke: 'none' },
  spin: { animationName: stylex.keyframes({ to: { transform: 'rotate(360deg)' } }), animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } },
})
