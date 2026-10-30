import { Button } from 'react-aria-components'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { TypedToolResult } from './TypedToolResult.tsx'

/** Inline recorded result for a typed call; the host's tool detail stays one press away. */
export const TypedToolPreview = ({ call, onOpen }: { readonly call: WorkLogCall; readonly onOpen: () => void }) => <div data-typed-tool-preview="">
  <TypedToolResult call={call} />
  <Button aria-label={`Open inspector for ${call.kind === 'subagent' ? 'task' : call.kind}`} onPress={onOpen}>Open inspector</Button>
</div>
