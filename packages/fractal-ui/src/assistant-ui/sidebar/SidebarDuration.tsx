// Explicit-clock presentation only.
import * as React from 'react'
import { VisuallyHidden } from 'react-aria-components'
import { compactTime, type SidebarDuration as Duration } from './model'

/** Display the reported duration scope; a one-day activity span is never labelled lifetime.
 *  Only a reported duration renders: callers omit the field when the source reported none. */
export const SidebarDuration = React.memo(function SidebarDuration({ duration }: { readonly duration: Extract<Duration, { readonly _tag: 'Known' }> }) {
  const value = compactTime({ at: 0, now: duration.ms })
  const label = duration.scope === '24h-activity-span' ? '24h activity span' : 'Lifetime duration'
  const descriptionId = React.useId()
  // Exact milliseconds stay available as the description; hosts own the only hover disclosure.
  return <><time dateTime={`PT${duration.ms / 1000}S`} aria-label={`${label}: ${value}`} aria-describedby={descriptionId}>{value}{duration.scope === '24h-activity-span' && <span data-line1-drop="scope">/24h</span>}</time><VisuallyHidden elementType="span"><span id={descriptionId}>{`${label}: ${duration.ms} milliseconds`}</span></VisuallyHidden></>
})
