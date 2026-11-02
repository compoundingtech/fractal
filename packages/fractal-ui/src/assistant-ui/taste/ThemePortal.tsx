import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { UNSAFE_PortalProvider, useUNSAFE_PortalContext } from 'react-aria'
import { usePortalContainerRegistration } from './PortalContainer'
import { spaceVars as s } from '../composition-tokens.stylex'
/** Portal inside the themed surface: arbitrary token themes reach hover cards and menus without component overrides. */
export function ThemePortal({ children }: { children: React.ReactNode }) {
  const inherited = useUNSAFE_PortalContext()
  const host = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => host.current, [])
  usePortalContainerRegistration(getContainer)
  // Reuse the surface's overlay layer instead of creating a host inside a clipped or transformed row.
  if (inherited.getContainer !== undefined) return children
  return <UNSAFE_PortalProvider getContainer={getContainer}>{children}<div ref={host} {...stylex.props(styles.host)} /></UNSAFE_PortalProvider>
}
const styles = stylex.create({ host: { position: 'fixed', top: s.zero, left: s.zero, width: s.zero, height: s.zero, zIndex: 20 } })
