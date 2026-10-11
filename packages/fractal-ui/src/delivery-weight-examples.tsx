import * as React from 'react'
import { HarnessDeliveryGroup } from './assistant-ui/composition/HarnessDeliveryGroup'
import { AgentResultCard } from './assistant-ui/composition/AgentResultCard'
import { BackgroundJobRow } from './assistant-ui/composition/BackgroundJobRow'
import { TerminalTail } from './assistant-ui/composition/TerminalTail'
import { deliveryFixture, resultFixture, jobFixture } from './delivery-fixtures'
import type { HarnessWeight } from './assistant-ui/embrace-data/model'
/** Shared production specimens for the delivery review and q106's two weight treatments. */
export const deliveryWeightExamples: readonly { readonly name: string; readonly weight: 'card' | 'row'; readonly render: (weight?: HarnessWeight) => React.ReactNode }[] = [
  { name: 'HarnessDeliveryGroup', weight: 'card', render: weight => <HarnessDeliveryGroup delivery={deliveryFixture} weight={weight} /> },
  { name: 'AgentResultCard', weight: 'card', render: weight => <AgentResultCard result={resultFixture} weight={weight} /> },
  { name: 'BackgroundJobRow', weight: 'row', render: weight => <BackgroundJobRow job={jobFixture} weight={weight} /> },
  { name: 'TerminalTail', weight: 'row', render: () => <TerminalTail output={jobFixture.output} /> },
]
