// Chrome controls: thread header, sidebar header, toolbar and footer. Explicit-fact presentation only.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { accentVars as accent, geometryVars as g, motionVars as m, radiusVars as r, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { Icon, type IconName } from './Icons'
import { Tooltip } from './Controls'

export interface ChromeControlProps {
  readonly icon: IconName
  /** Accessible name and tooltip text: "Toggle right panel", "Actions for fractal-web". */
  readonly label: string
  /** Toggle state; omitted for plain actions. A pressed toggle keeps the hover fill. */
  readonly pressed?: boolean
  /** The fill without toggle semantics: a menu trigger whose filter or sort is in effect. The label says what is set. */
  readonly active?: boolean
  /** Why the control cannot run; it renders disabled, described by the reason, with the reason as the hover title. */
  readonly disabledReason?: string
  /** Briefly unavailable, e.g. while its own action is in flight. */
  readonly isDisabled?: boolean
  readonly onPress?: () => void
  readonly tooltipPlacement?: 'top' | 'bottom'
}

/**
 * One quiet 28px icon button for the chrome: borderless, filled on hover, named by the kit tooltip.
 * Inside a React Aria trigger (MenuTrigger, DialogTrigger) it is the trigger's button.
 */
export function ChromeControl({ icon, label, pressed, active = false, disabledReason, isDisabled = false, onPress, tooltipPlacement = 'bottom' }: ChromeControlProps) {
  const reasonId = React.useId()
  const button = <Button
    aria-label={label}
    {...(pressed === undefined ? {} : { 'aria-pressed': pressed })}
    {...(disabledReason === undefined ? { isDisabled } : { isDisabled: true, 'aria-describedby': reasonId })}
    {...(onPress === undefined ? {} : { onPress })}
    {...stylex.props(styles.control, (pressed === true || active) && styles.pressed)}
  >
    <Icon name={icon} />
  </Button>
  // A disabled button receives no pointer events, so its reason sits on the wrapper's title.
  if (disabledReason !== undefined) return <span title={disabledReason} {...stylex.props(styles.disabledWrap)}>{button}<span id={reasonId} hidden>{disabledReason}</span></span>
  return <Tooltip label={label} placement={tooltipPlacement}>{button}</Tooltip>
}

const styles = stylex.create({
  control: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, boxSizing: 'border-box',
    width: g.controlMd, height: g.controlMd, padding: 0,
    borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fgMuted,
    cursor: 'pointer',
    transitionProperty: 'background-color, color', transitionDuration: m.fast, transitionTimingFunction: m.ease,
    ':hover': { backgroundColor: surface.rowHover, color: ink.fg },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
    ':disabled': { opacity: 0.64, cursor: 'default', backgroundColor: 'transparent', color: ink.fgMuted },
  },
  pressed: { backgroundColor: surface.rowActive, color: ink.fg, ':hover': { backgroundColor: surface.rowActive, color: ink.fg } },
  disabledWrap: { display: 'inline-flex' },
})
