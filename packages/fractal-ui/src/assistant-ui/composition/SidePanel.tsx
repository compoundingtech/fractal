import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { FocusScope } from 'react-aria'
import { borderVars as border, elevationVars as elevation, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

export interface SidePanelProps {
  /** Landmark name; the panel is a labelled complementary region, never a dialog. */
  readonly 'aria-label': string
  /** Visible panel title. It takes focus when the panel opens, so reading starts at the top. */
  readonly heading: string
  /** Trailing header controls, such as the panel's close button. */
  readonly actions?: React.ReactNode
  /** Escape inside the panel calls this; the host unmounts the panel to close it. */
  readonly onClose: () => void
  readonly children?: React.ReactNode
  readonly id?: string
  readonly ref?: React.Ref<HTMLElement>
  readonly 'data-testid'?: string
  readonly style?: stylex.StyleXStyles
}

/**
 * A non-modal side panel floating over the trailing edge of the workspace. It moves focus to its
 * heading on mount, closes on Escape and returns focus to the element focused before it opened,
 * but never contains Tab, hides, inerts or scroll-locks the rest of the app. Use a modal dialog
 * only when the user must finish or dismiss the task before anything else.
 */
export function SidePanel({ 'aria-label': label, heading, actions, onClose, children, id, ref, 'data-testid': testId, style }: SidePanelProps) {
  const title = React.useRef<HTMLHeadingElement>(null)
  // The heading is a programmatic focus target only (tabIndex -1): Tab starts at the first control,
  // and neither the region nor a control paints a focus ring the user did not ask for.
  React.useLayoutEffect(() => { title.current?.focus({ preventScroll: true }) }, [])
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    // A focused child that handles Escape itself (menu, combobox) keeps the panel open.
    if (event.key !== 'Escape' || event.defaultPrevented) return
    event.preventDefault()
    event.stopPropagation()
    onClose()
  }
  return <FocusScope restoreFocus>
    <aside ref={ref} id={id} aria-label={label} data-side-panel="" data-testid={testId} onKeyDown={onKeyDown} {...stylex.props(styles.panel, style)}>
      <header {...stylex.props(styles.header)}>
        <h2 ref={title} tabIndex={-1} data-side-panel-heading="" {...stylex.props(styles.heading)}>{heading}</h2>
        {actions}
      </header>
      {children}
    </aside>
  </FocusScope>
}

const styles = stylex.create({
  panel: {
    position: 'fixed', insetBlock: 0, insetInlineEnd: 0, zIndex: 100,
    display: 'flex', flexDirection: 'column', boxSizing: 'border-box',
    width: 380, maxWidth: 'calc(100vw - 24px)', minHeight: 0,
    backgroundColor: surface.raised, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize,
    borderInlineStartWidth: 1, borderInlineStartStyle: 'solid', borderInlineStartColor: border.borderStrong,
    boxShadow: elevation.dialog,
  },
  header: {
    display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, flexShrink: 0,
    padding: s.lg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border,
  },
  // Never in the tab order and not a control: programmatic focus places the reading position silently.
  heading: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold, outlineStyle: 'none' },
})
