import * as React from 'react'
import { createPortal } from 'react-dom'

/** Optional instrumentation of React's delegated portal event containers. */
export const PortalContainerRegistrationContext = React.createContext<((container: Element) => () => void) | undefined>(undefined)

export function usePortalContainerRegistration(getContainer: () => Element | null) {
  const register = React.useContext(PortalContainerRegistrationContext)
  React.useLayoutEffect(() => {
    const container = getContainer()
    if (container !== null) return register?.(container)
  }, [getContainer, register])
}

/** No wrapper DOM: portal ownership and lifecycle are unchanged. */
export function RegisteredPortal({ container, children }: { container: Element; children: React.ReactNode }) {
  const getContainer = React.useCallback(() => container, [container])
  usePortalContainerRegistration(getContainer)
  return createPortal(children, container)
}
