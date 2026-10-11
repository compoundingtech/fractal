import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { DeliveryAction } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
/** Unavailable actions remain in the keyboard order, with a visible accessible reason. */
export function DeliveryActionButton({ label, action }: { readonly label: string; readonly action: DeliveryAction }) {
  const reasonId = React.useId()
  return <><Button aria-disabled={action._tag === 'Unavailable'} aria-describedby={action._tag === 'Unavailable' ? reasonId : undefined} onPress={action._tag === 'Available' ? action.onPress : undefined} {...stylex.props(styles.button)}>{label}</Button>{action._tag === 'Unavailable' && <span id={reasonId} {...stylex.props(styles.muted)}>{action.reason}</span>}</>
}
