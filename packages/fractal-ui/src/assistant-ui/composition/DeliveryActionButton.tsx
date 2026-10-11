import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import type { DeliveryAction } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
/** Unavailable actions retain their keyboard slot; the reason appears on focus or hover. */
export function DeliveryActionButton({ label, action }: { readonly label: string; readonly action: DeliveryAction }) {
  const reasonId = React.useId()
  const [reasonOpen, setReasonOpen] = React.useState(false)
  const unavailable = action._tag === 'Unavailable'
  const reasonVisible = unavailable && reasonOpen
  return <TooltipTrigger isDisabled={!unavailable} isOpen={reasonVisible} onOpenChange={setReasonOpen} delay={150}><Button aria-disabled={unavailable} aria-describedby={unavailable && !reasonVisible ? reasonId : undefined} onPress={action._tag === 'Available' ? action.onPress : undefined} {...stylex.props(styles.button, unavailable && styles.unavailable)}>{label}</Button><Tooltip id={reasonId} {...stylex.props(styles.tooltip)}>{action._tag === 'Unavailable' ? action.reason : ''}</Tooltip>{unavailable && !reasonVisible && <span id={reasonId} {...stylex.props(styles.hiddenReason)}>{action.reason}</span>}</TooltipTrigger>
}
