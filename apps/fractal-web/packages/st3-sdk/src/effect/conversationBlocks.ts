import { TimelineBlock } from '@smalltalk/st3-client/schema'
import { Schema } from 'effect'

/** Open native views evolve independently of the generated v0 fallback bodies. */
export const SourceBlock = Schema.Struct({
  ...TimelineBlock.fields,
  view: Schema.optionalKey(Schema.Record(Schema.String, Schema.Unknown)),
  header: Schema.optionalKey(Schema.Record(Schema.String, Schema.Unknown)),
  assistant_metadata: Schema.optionalKey(Schema.Record(Schema.String, Schema.Unknown)),
})
export type SourceBlock = typeof SourceBlock.Type
const decodeBlocks = Schema.decodeUnknownSync(Schema.Array(SourceBlock))

/** Feature-detect the optional negotiated blocks; old daemons keep the fallback body. */
export const sourceBlocks = (body: unknown): readonly SourceBlock[] | undefined => {
  if (body === null || typeof body !== 'object' || !('blocks' in body) || body.blocks === undefined) return undefined
  return decodeBlocks(body.blocks)
}
