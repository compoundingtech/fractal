import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { FocusScope, mergeProps, UNSAFE_PortalProvider, useDialog, useModalOverlay } from 'react-aria'
import { useOverlayTriggerState } from 'react-stately'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl'
import { colorVars as c, spaceVars as s, surfaceVars as surface, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'

export interface PhoneAgentDrawerHandle { close: () => void; toggle: () => void }

/** React Aria's modal hooks keep one roster mounted. Drawer state never reaches the thread. */
export function PhoneAgentDrawer({ ref, children, header }: {
  readonly ref: React.Ref<PhoneAgentDrawerHandle>
  readonly children: React.ReactNode
  readonly header: (open: () => void, sheet: { readonly open: boolean; readonly id: string }) => React.ReactNode
}) {
  const dialog = React.useRef<HTMLDivElement>(null)
  const closeButton = React.useRef<HTMLButtonElement>(null)
  const opener = React.useRef<HTMLElement | null>(null)
  const portalHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => portalHost.current, [])
  const state = useOverlayTriggerState({})
  const { isOpen, open: openOverlay, close } = state
  const id = React.useId()
  const open = () => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    openOverlay()
  }
  const { modalProps, underlayProps } = useModalOverlay({ isDismissable: true }, state, dialog)
  const { dialogProps } = useDialog({ 'aria-label': 'Agents' }, dialog)
  React.useLayoutEffect(() => {
    if (isOpen) { closeButton.current?.focus(); return }
    // Restore after React Aria has released containment and its pending blur frame.
    const frame = window.requestAnimationFrame(() => opener.current?.focus())
    return () => window.cancelAnimationFrame(frame)
  }, [isOpen])
  React.useImperativeHandle(ref, () => ({ close, toggle: () => { if (isOpen) close(); else open() } }), [close, isOpen, open])
  return <>
    {header(open, { open: isOpen, id })}
    <div {...underlayProps} hidden={!isOpen} {...stylex.props(styles.overlay, !isOpen && styles.hidden)}>
      <FocusScope contain={isOpen}>
        <div {...mergeProps(modalProps, dialogProps)} ref={dialog} id={id} {...stylex.props(styles.drawer)}>
          <aside aria-label="Agents" data-sidebar-presentation="drawer" {...stylex.props(styles.sidebar)}>
          <header {...stylex.props(styles.header)}><strong>Agents</strong><ChromeControl ref={closeButton} icon="chevrons-left" label="Close agents" onPress={close} /></header>
          <UNSAFE_PortalProvider getContainer={getContainer}>{children}<div ref={portalHost} /></UNSAFE_PortalProvider>
          </aside>
        </div>
      </FocusScope>
    </div>
  </>
}

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 90, display: 'flex', backgroundColor: surface.scrim },
  hidden: { display: 'none' },
  drawer: { width: 'min(320px, calc(100vw - 40px))', height: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', paddingBlockStart: 'env(safe-area-inset-top)', paddingBlockEnd: 'env(safe-area-inset-bottom)', paddingInlineStart: 'env(safe-area-inset-left)', backgroundColor: c.sidebar, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, outline: 'none' },
  sidebar: { display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, width: '100%' },
  header: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', paddingInline: s.lg, minHeight: 44, flexShrink: 0 },
})
