import * as React from 'react'

import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import type { AsidePanelProps } from '../web/AgentAside.tsx'

const MissionPanel = React.lazy(() => import('./detail/MissionPanel.tsx').then(module => ({ default: module.MissionPanel })))

/** A mission tab's side panel: Details (the inspector), Plan and Agents for the mission this pane shows. */
export const MissionAside = ({ subject, onOpen, onClose }: AsidePanelProps) => {
  if (subject._tag !== 'Mission') return null
  return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading mission details" />}>
    <MissionPanel key={subject.ref} missionRef={subject.ref} onClose={onClose}
      onOpen={(ref, gesture) => onOpen(ref, gesture?.background === true || gesture?.split !== undefined ? { split: 'right' } : undefined)} />
  </React.Suspense>
}
