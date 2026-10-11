import * as React from 'react'
import { HarnessDeliveryGroup } from './assistant-ui/composition/HarnessDeliveryGroup'
import { AgentResultCard } from './assistant-ui/composition/AgentResultCard'
import { BackgroundJobRow } from './assistant-ui/composition/BackgroundJobRow'
import { TerminalTail } from './assistant-ui/composition/TerminalTail'
import { deliveryFixture, resultFixture, jobFixture } from './delivery-fixtures'
/** Shared production specimens for the delivery review and q106's two weight treatments. */
export const deliveryWeightExamples: readonly { readonly name: string; readonly weight: 'card' | 'row'; readonly render: () => React.ReactNode }[] = [
  { name: 'HarnessDeliveryGroup', weight: 'card', render: () => <HarnessDeliveryGroup delivery={deliveryFixture} /> },
  { name: 'AgentResultCard', weight: 'card', render: () => <AgentResultCard result={resultFixture} /> },
  { name: 'BackgroundJobRow', weight: 'row', render: () => <BackgroundJobRow job={jobFixture} /> },
  { name: 'TerminalTail', weight: 'row', render: () => <TerminalTail output={jobFixture.output} /> },
]
