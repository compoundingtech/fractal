import { Button } from 'react-aria-components'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { TypedToolResult } from './TypedToolResult.tsx'

/** The inline task preview and inspector share the same recorded-result interpretation. */
export const TaskResultPreview = ({ call, onOpen }: { readonly call: WorkLogCall; readonly onOpen: () => void }) => <div data-task-result-preview=""><TypedToolResult call={call} /><Button aria-label="Open inspector for task" onPress={onOpen}>Open inspector</Button></div>
