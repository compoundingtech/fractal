import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Focusable, Tooltip, TooltipTrigger, VisuallyHidden } from 'react-aria-components'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { elevationVars as elevation, geometryNumbers, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'

type DataAttributes = Readonly<Record<`data-${string}`, string | number | boolean | undefined>>

/** Hover/focus delay shared by every quota and usage-meter disclosure, matching the kit tooltip. */
const delay = 350

/** The one tooltip surface for quota and usage-meter disclosures; pair it with any React Aria trigger. */
export const ValueTooltip = ({ children }: { readonly children: React.ReactNode }): React.ReactElement =>
  <Tooltip placement="top" offset={geometryNumbers.tooltipOffset} {...stylex.props(styles.tooltip)}>{children}</Tooltip>

/** Wraps an existing React Aria trigger (Button, Focusable) with the shared value tooltip. */
export const ValueTooltipTrigger = ({ label, children }: { readonly label: string; readonly children: React.ReactElement }): React.ReactElement =>
  <TooltipTrigger delay={delay} closeDelay={0}>{children}<ValueTooltip>{label}</ValueTooltip></TooltipTrigger>

/**
 * A visually clipped or abbreviated value with one focusable disclosure: hover or keyboard focus opens a single tooltip
 * with the full text. The trigger is a named static element (`role="img"`, as React Aria requires of a non-interactive
 * tooltip trigger) labelled by a mounted VisuallyHidden copy of the full text; React Aria also points
 * `aria-describedby` at the open tooltip. The visible rendering is hidden from assistive technology.
 */
export const FullValueTooltip = ({ full, children, style, visibleStyle, data, valueData }: {
  readonly full: string
  readonly children: React.ReactNode
  /** Trigger box: clipping, display and typography of the visible value. */
  readonly style?: stylex.StyleXStyles
  /** Wrapper of the visible rendering, for values laid out as flex rows or stacked lines. */
  readonly visibleStyle?: stylex.StyleXStyles
  readonly data?: DataAttributes
  /** Probe hooks on the mounted full value. */
  readonly valueData?: DataAttributes
}): React.ReactElement => {
  const fullId = React.useId()
  return <TooltipTrigger delay={delay} closeDelay={0}>
    <Focusable>
      <span role="img" tabIndex={0} aria-labelledby={fullId} data-full-value-trigger {...data} {...stylex.props(styles.trigger, style)}>
        <span aria-hidden="true" {...stylex.props(visibleStyle)}>{children}</span>
        <VisuallyHidden elementType="span"><span id={fullId} {...valueData}>{full}</span></VisuallyHidden>
      </span>
    </Focusable>
    <ValueTooltip>{full}</ValueTooltip>
  </TooltipTrigger>
}

const styles = stylex.create({
  // Inset ring: cells clip their overflow, so an outset ring would be cut off.
  trigger: { outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.inlineFocusOffset },
  // max-content: a portal near the viewport edge would otherwise shrink-to-fit the anywhere-wrapping text to one glyph per line.
  tooltip: { zIndex: 50, boxSizing: 'border-box', width: 'max-content', maxWidth: `min(${geometry.tooltipMax}, calc(100vw - ${space.lg}))`, paddingInline: space.sm, paddingBlock: space.xs, borderRadius: radius.keyboard, backgroundColor: ink.fg, color: surface.raised, boxShadow: elevation.popover, fontFamily: type.fontSans, fontSize: type.denseSize, lineHeight: type.metaLeading, overflowWrap: 'anywhere', whiteSpace: 'normal' },
})
