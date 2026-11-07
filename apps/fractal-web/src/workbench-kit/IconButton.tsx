// The base `Button` starts at 32px (`sm`); IDE chrome needs a 22–28px icon-only button with a tooltip
// that names the action and its keybinding. Upstream shape: `Button size="xs" shape="square"` plus
// a `tooltip` prop.

import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import { Button as AriaButton, type ButtonProps as AriaButtonProps } from 'react-aria-components'

import { Tooltip, TooltipTrigger } from '../ui-compat/components.tsx'

import { interactionVars, surfaceVars, textVars, radiusVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

const styles = stylex.create({
  button: {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexShrink: 0,
    borderWidth: 0,
    borderRadius: radiusVars.xs,
    padding: 0,
    color: { default: textVars.fg, ':is([data-hovered])': textVars.fg },
    backgroundColor: {
      default: 'transparent',
      ':is([data-hovered])': surfaceVars.message,
      ':is([data-pressed])': surfaceVars.message,
    },
    cursor: 'pointer',
    outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' },
    outlineWidth: '2px',
    outlineColor: interactionVars.focusRing,
    outlineOffset: '-2px',
  },
  sm: { width: '1.375rem', height: '1.375rem' },
  md: { width: '1.75rem', height: '1.75rem' },
  active: {
    color: textVars.fg,
    backgroundColor: surfaceVars.message,
  },
})

/** Icon-only action props with an accessible name and optional shortcut hint. */
export interface IconButtonProps extends Omit<AriaButtonProps, 'className' | 'style' | 'children'> {
  /** Accessible name; also the tooltip text. */
  readonly label: string
  readonly icon: ReactNode
  readonly shortcut?: ReactNode
  readonly size?: 'sm' | 'md'
  readonly isActive?: boolean
  readonly tooltipPlacement?: 'top' | 'bottom' | 'left' | 'right'
  /** Host geometry override, e.g. a touch-sized target in a narrow layout. */
  readonly stylexStyle?: stylex.StyleXStyles
}

/** Compact icon-only button whose tooltip names the action and shortcut. */
export const IconButton = ({
  label,
  icon,
  shortcut,
  size = 'sm',
  isActive = false,
  tooltipPlacement = 'bottom',
  stylexStyle,
  ...props
}: IconButtonProps) => (
  <TooltipTrigger delay={600} closeDelay={0}>
    <AriaButton
      aria-label={label}
      {...(isActive ? { 'aria-pressed': true } : {})}
      {...props}
      {...stylex.props(styles.button, styles[size], isActive && styles.active, stylexStyle)}
    >
      {icon}
    </AriaButton>
    <Tooltip placement={tooltipPlacement}>
      {label}
      {shortcut === undefined ? null : <> {shortcut}</>}
    </Tooltip>
  </TooltipTrigger>
)
