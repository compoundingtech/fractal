import * as React from 'react'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

/** A coarse primary pointer with no fine one: the composer's touch query. */
export const touchPointerQuery = '(pointer: coarse) and (not (any-pointer: fine))'
const subscribeTouchPointer = (onChange: () => void) => {
  const media = window.matchMedia(touchPointerQuery)
  media.addEventListener('change', onChange)
  return () => media.removeEventListener('change', onChange)
}
const touchPointer = () => window.matchMedia(touchPointerQuery).matches
/** Touch layout: a narrow host or a coarse primary pointer. */
export function useTouchLayout() {
  const narrow = React.useContext(NarrowLayoutContext)
  const coarse = React.useSyncExternalStore(subscribeTouchPointer, touchPointer, () => false)
  return narrow || coarse
}
