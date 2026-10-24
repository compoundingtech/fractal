import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { FocusScope, mergeProps, UNSAFE_PortalProvider, useDialog, useModalOverlay } from 'react-aria'
import { useOverlayTriggerState } from 'react-stately'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout'
import { borderVars as border, colorVars as c, elevationVars as elevation, geometryVars as g, surfaceVars as surface, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'
import { SidebarHeader, useCoarsePointer } from './ResponsiveAgentSidebar.tsx'

export interface PhoneAgentDrawerHandle { close: () => void; toggle: () => void }

/**
 * React Aria's modal hooks keep one roster mounted. Drawer state never reaches the thread.
 * The drawer is the dock in a modal: the same header (collapse, wordmark, alerts) and the dock's density unless the
 * primary pointer is coarse. Closing returns focus to `triggerRef`, the header control that opens it, with a focus
 * ring only when the drawer was dismissed from the keyboard.
 */
export function PhoneAgentDrawer({ ref, triggerRef, headerActions, children, header }: {
  readonly ref: React.Ref<PhoneAgentDrawerHandle>
  /** The control that opens the drawer; it gets focus back on every close. */
  readonly triggerRef: React.RefObject<HTMLElement | null>
  /** The dock header's trailing actions, the alerts. */
  readonly headerActions?: React.ReactNode
  readonly children: React.ReactNode
  readonly header: (open: () => void, sheet: { readonly open: boolean; readonly id: string }) => React.ReactNode
}) {
  const dialog = React.useRef<HTMLDivElement>(null)
  const closeButton = React.useRef<HTMLButtonElement>(null)
  const portalHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => portalHost.current, [])
  // The input that last acted inside the open drawer decides whether the restored trigger shows its focus ring.
  const dismissedBy = React.useRef<'keyboard' | 'pointer'>('keyboard')
  const wasOpen = React.useRef(false)
  const coarse = useCoarsePointer()
  const state = useOverlayTriggerState({})
  const { isOpen, open, close } = state
  const id = React.useId()
  const { modalProps, underlayProps } = useModalOverlay({ isDismissable: true }, state, dialog)
  const { dialogProps } = useDialog({ 'aria-label': 'Agents' }, dialog)
  React.useLayoutEffect(() => {
    if (isOpen) { wasOpen.current = true; dismissedBy.current = 'keyboard'; closeButton.current?.focus(); return }
    if (!wasOpen.current) return
    wasOpen.current = false
    // Restore after React Aria has released containment and its pending blur frame.
    const focusVisible = dismissedBy.current === 'keyboard'
    const frame = window.requestAnimationFrame(() => triggerRef.current?.focus({ preventScroll: true, focusVisible }))
    return () => window.cancelAnimationFrame(frame)
  }, [isOpen, triggerRef])
  React.useImperativeHandle(ref, () => ({ close, toggle: () => { if (isOpen) close(); else open() } }), [close, isOpen, open])
  return <>
    {header(open, { open: isOpen, id })}
    <div {...underlayProps} onPointerDownCapture={() => { dismissedBy.current = 'pointer' }} onKeyDownCapture={() => { dismissedBy.current = 'keyboard' }} hidden={!isOpen} {...stylex.props(styles.overlay, !isOpen && styles.hidden)}>
      <FocusScope contain={isOpen}>
        <div {...mergeProps(modalProps, dialogProps)} ref={dialog} id={id} aria-modal="true" {...stylex.props(styles.drawer)}>
          {/* Touch density follows the pointer, not the window: a fine pointer at 899px keeps the dock's rows and controls. */}
          <NarrowLayoutContext.Provider value={coarse}>
            <aside aria-label="Agents" data-sidebar-presentation="drawer" data-sidebar-density={coarse ? 'touch' : 'dock'} {...stylex.props(styles.sidebar)}>
              <UNSAFE_PortalProvider getContainer={getContainer}>
                <SidebarHeader ref={closeButton} label="Close agents" onCollapse={close} actions={headerActions} />
                {children}
                <div ref={portalHost} />
              </UNSAFE_PortalProvider>
            </aside>
          </NarrowLayoutContext.Provider>
        </div>
      </FocusScope>
    </div>
  </>
}

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 90, display: 'flex', backgroundColor: surface.scrim },
  hidden: { display: 'none' },
  // A floating layer: the raised surface, a hairline on its trailing edge and the dialog shadow, so the edge reads over the dark scrim.
  drawer: { width: 'min(320px, calc(100vw - 40px))', height: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', paddingBlockStart: 'env(safe-area-inset-top)', paddingBlockEnd: 'env(safe-area-inset-bottom)', paddingInlineStart: 'env(safe-area-inset-left)', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, borderInlineEndWidth: g.hairline, borderInlineEndStyle: 'solid', borderInlineEndColor: border.borderStrong, boxShadow: elevation.dialog, outline: 'none' },
  sidebar: { display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, width: '100%' },
})
