// Below the host's drawer breakpoint the thread is the only thing on screen; the current agent opens from the end edge.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, Heading, Modal, ModalOverlay, Toolbar } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from './Icons'

/** A modal sheet from the end edge; the thread behind it keeps its layout. Escape closes it and focus returns to its opener. */
export function NarrowSheet({ label, isOpen, onOpenChange, width, portalContainer, children }: {
  readonly label: string
  readonly isOpen: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly width: number
  /** A node inside the themed workspace, so the sheet resolves the workspace's theme. */
  readonly portalContainer?: Element
  readonly children: React.ReactNode
}) {
  return <ModalOverlay isOpen={isOpen} onOpenChange={onOpenChange} isDismissable UNSTABLE_portalContainer={portalContainer} {...stylex.props(styles.overlay)}>
    <Modal style={{ width }} {...stylex.props(styles.sheet)}>
      <Dialog aria-label={label} data-narrow-sheet="" {...stylex.props(styles.dialog)}>
        <header {...stylex.props(styles.sheetHeader)}>
          <Heading slot="title" {...stylex.props(styles.sheetTitle)}>{label}</Heading>
          <Button aria-label={`Close ${label.toLowerCase()}`} onPress={() => onOpenChange(false)} {...stylex.props(styles.iconButton)}><Icon name="x" /></Button>
        </header>
        {children}
      </Dialog>
    </Modal>
  </ModalOverlay>
}

/** The agent's actions as one wrapping row at the top of a sheet. */
export function NarrowActionRow({ label, children }: { readonly label: string; readonly children: React.ReactNode }) {
  return <Toolbar aria-label={label} data-narrow-actions="" {...stylex.props(styles.actionRow)}>{children}</Toolbar>
}

const styles = stylex.create({
  overlay: { position: 'fixed', insetInline: 0, insetBlockStart: 0, insetBlockEnd: 'var(--wf-devbar-space, 0px)', zIndex: 90, display: 'flex', justifyContent: 'flex-end', backgroundColor: surface.scrim },
  sheet: { height: '100%', maxWidth: '100%', display: 'flex', backgroundColor: surface.canvas, color: text.fg, borderInlineStartWidth: g.hairline, borderInlineStartStyle: 'solid', borderInlineStartColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  dialog: { flexGrow: 1, minWidth: 0, display: 'flex', flexDirection: 'column', outline: 'none' },
  sheetHeader: { height: g.band, flexShrink: 0, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  sheetTitle: { margin: 0, flexGrow: 1, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  iconButton: { width: g.controlMd, height: g.controlMd, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', borderWidth: 0, borderRadius: r.control, backgroundColor: surface.transparent, color: text.fgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: text.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  actionRow: { flexShrink: 0, display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.xs, paddingBlock: s.md, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
})
