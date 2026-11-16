import type { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Schema } from 'effect'

/**
 * TEMPORARY PRE-CODEGEN ADAPTER: exact retired-origin r2 $defs, hand-written until the
 * coordinated Phase B generation supplies these models and thread.start-context.
 * No generated files are modified. Replace this adapter with the generated read.
 */
export const ThreadStartRecipient = Schema.Struct({
  agent_id: Native.Id,
  basis: Schema.Union([
    Schema.Struct({ _tag: Schema.Literal('original') }),
    Schema.Struct({ _tag: Schema.Literal('configured') }),
    Schema.Struct({
      _tag: Schema.Literal('fallback'),
      original_agent_id: Native.Id,
      retirement: Schema.Struct({
        claim_id: Native.Id,
        kind: Schema.Literal('agent.retired'),
        state: Schema.Literal('retired'),
      }),
    }),
  ]),
}).annotate({ identifier: 'ThreadStartRecipient' })
export type ThreadStartRecipient = typeof ThreadStartRecipient.Type

export const ThreadStartContext = Schema.Struct({
  kind: Schema.Literal('thread-start-context'),
  source_id: Native.Id,
  episode: Native.Revision,
  start_recipient: ThreadStartRecipient,
}).annotate({ identifier: 'ThreadStartContext' })
export type ThreadStartContext = typeof ThreadStartContext.Type

// r2 sets additionalProperties:false at every object boundary, including union cases.
export const decodeThreadStartContext = Schema.decodeUnknownSync(ThreadStartContext, { onExcessProperty: 'error' })

export const readThreadStartContext = async (
  client: FractalClient,
  sourceId: string,
  episode: string,
  signal?: AbortSignal,
): Promise<ThreadStartContext> => {
  const query = new URLSearchParams({ source_id: sourceId, episode })
  // The generated client has no public generic GET yet. Bracket access is deliberately
  // confined here: keep its discovery, credentials, trace context, read policy and
  // envelope/error validation rather than inventing a second fetch transport.
  const { value } = await client['get']<unknown>(`/v1/client/threads/start-context?${query}`, undefined, { ...(signal === undefined ? {} : { signal }) })
  const context = decodeThreadStartContext(value)
  if (context.source_id !== sourceId || context.episode !== episode) throw new Error('Mismatched thread start context')
  signal?.throwIfAborted()
  return context
}
