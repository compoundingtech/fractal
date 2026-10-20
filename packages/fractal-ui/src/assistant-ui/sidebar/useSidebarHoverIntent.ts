import * as React from 'react'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

/** Phone rows expose details through their touch menu, never a hover-only overlay. */
export function useSidebarHoverIntent() {
  const narrow = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  const timer = React.useRef<number | undefined>(undefined)
  React.useEffect(() => {
    setOpen(false)
    return () => { clearTimeout(timer.current) }
  }, [narrow])
  const setHoverOpen = (value: boolean) => {
    clearTimeout(timer.current)
    setOpen(!narrow && value)
  }
  const hoverIntent = (value: boolean) => {
    clearTimeout(timer.current)
    if (narrow) return
    timer.current = window.setTimeout(() => setOpen(value), value ? 150 : 300)
  }
  return { hoverEnabled: !narrow, hoverOpen: !narrow && open, setHoverOpen, hoverIntent }
}
