import * as React from 'react'
import { useEntityDrag } from '@fractal/ui/workspace/react'
import { paneKey, subjectOfRef } from './subject.ts'

/**
 * An entity named in prose (a document, a pull request link) that drags onto the workspace as its tab
 * would: a strip slot, a pane's centre or an edge. The link inside keeps its click; the platform's own
 * link drag gives way to the workspace's. Outside a workspace it is only its link.
 */
export const DraggableMention = ({ subject, children }: { readonly subject: string; readonly children?: React.ReactNode }) => {
  const drag = useEntityDrag({ ref: paneKey(subjectOfRef(subject)) })
  // Capture: a React Aria link's press stops the pointer's propagation.
  return <span data-entity-mention={subject} onPointerDownCapture={drag.onPointerDown} onDragStart={event => event.preventDefault()}>{children}</span>
}
