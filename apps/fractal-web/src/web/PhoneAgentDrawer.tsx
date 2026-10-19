import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { FocusScope, mergeProps, useDialog, useModalOverlay } from 'react-aria'
import * as Aria from 'react-aria-components'
import { colorVars as c, spaceVars as s, surfaceVars as surface, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'

export interface PhoneAgentDrawerHandle { close: () => void }

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
  const [isOpen, setOpen] = React.useState(false)
  const id = React.useId()
  const close = React.useCallback(() => setOpen(false), [])
  const open = () => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    setOpen(true)
  }
  const state = { isOpen, setOpen, open, close, toggle: () => setOpen(value => !value) }
  const { modalProps, underlayProps } = useModalOverlay({ isDismissable: true }, state, dialog)
  const { dialogProps } = useDialog({ 'aria-label': 'Agents' }, dialog)
  React.useLayoutEffect(() => { if (isOpen) closeButton.current?.focus(); else opener.current?.focus() }, [isOpen])
  React.useImperativeHandle(ref, () => ({ close }), [close])
  return <>
    {header(open, { open: isOpen, id })}
    <div {...underlayProps} hidden={!isOpen} {...stylex.props(styles.overlay, !isOpen && styles.hidden)}>
      <FocusScope contain={isOpen}>
        <div {...mergeProps(modalProps, dialogProps)} ref={dialog} id={id} {...stylex.props(styles.drawer)}>
          <header {...stylex.props(styles.header)}><strong>Agents</strong><Aria.Button ref={closeButton} aria-label="Close agents" onPress={close} {...stylex.props(styles.close)}>×</Aria.Button></header>
          <Aria.UNSAFE_PortalProvider getContainer={getContainer}>{children}<div ref={portalHost} /></Aria.UNSAFE_PortalProvider>
        </div>
      </FocusScope>
    </div>
  </>
}

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 90, display: 'flex', backgroundColor: surface.scrim },
  hidden: { display: 'none' },
  drawer: { width: 'min(320px, calc(100vw - 40px))', height: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', paddingBlockStart: 'env(safe-area-inset-top)', paddingBlockEnd: 'env(safe-area-inset-bottom)', paddingInlineStart: 'env(safe-area-inset-left)', backgroundColor: c.sidebar, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, outline: 'none' },
  header: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', paddingInline: s.lg, minHeight: 44, flexShrink: 0 },
  close: { minWidth: 44, minHeight: 44, borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, fontSize: 24, cursor: 'pointer', ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } },
})
