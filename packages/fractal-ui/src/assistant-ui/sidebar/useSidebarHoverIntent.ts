import * as React from 'react'
import { useTouchLayout } from './useTouchLayout'

/** Touch rows (coarse pointer or a touch host) expose details through their touch menu, never a hover-only overlay. */
export function useSidebarHoverIntent() {
  const touch = useTouchLayout()
  // Open state remembers the layout it was set under; a layout switch closes it during render. A reset effect would
  // also run after mount, and a hover that lands before mount effects flush would be closed by it.
  const [hover, setHover] = React.useState({ open: false, touch })
  if (hover.touch !== touch) setHover({ open: false, touch })
  const timer = React.useRef<number | undefined>(undefined)
  React.useEffect(() => () => { clearTimeout(timer.current) }, [touch])
  const setHoverOpen = (value: boolean) => {
    clearTimeout(timer.current)
    setHover({ open: !touch && value, touch })
  }
  const hoverIntent = (value: boolean) => {
    clearTimeout(timer.current)
    if (touch) return
    timer.current = window.setTimeout(() => setHover({ open: value, touch }), value ? 150 : 300)
  }
  return { hoverEnabled: !touch, hoverOpen: !touch && hover.touch === touch && hover.open, setHoverOpen, hoverIntent }
}
