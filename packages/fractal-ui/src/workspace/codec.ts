import { Option, Schema } from 'effect'
import { Layout } from './layout.ts'
import { PaneLocal, repairWorkspace, type WorkspaceConfig, type WorkspaceState } from './model.ts'

const SpaceLocalOf = <V>(view: Schema.Codec<V>) => Schema.Struct({
  focus: Schema.String,
  zoomed: Schema.NullOr(Schema.String),
  mru: Schema.Array(Schema.String),
  closed: Schema.Array(Schema.String),
  views: Schema.Record(Schema.String, view),
  panes: Schema.Record(Schema.String, PaneLocal),
})

/** The persisted document, version 1, with the host's per-tab view schema. */
export const makeWorkspaceSchema = <V>(view: Schema.Codec<V>): Schema.Codec<WorkspaceState<V>> => {
  const local = SpaceLocalOf(view)
  return Schema.Struct({
    version: Schema.Literal(1),
    current: Schema.String,
    spaces: Schema.Array(Schema.Struct({
      id: Schema.String,
      name: Schema.String,
      layout: Layout,
      revision: Schema.optional(Schema.String),
      local,
    })),
    retired: Schema.Array(Schema.Struct({ id: Schema.String, local })),
  })
}

/** A document of an earlier shape, and how it becomes the current one. */
export interface LegacyDecoder<V> {
  readonly schema: Schema.Codec<unknown>
  readonly migrate: (legacy: never) => WorkspaceState<V>
}

export const legacyDecoder = <L, V>(schema: Schema.Codec<L>, migrate: (legacy: L) => WorkspaceState<V>): LegacyDecoder<V> =>
  ({ schema: schema as Schema.Codec<unknown>, migrate: migrate as (legacy: never) => WorkspaceState<V> })

/**
 * Decodes a stored document: the current version first, then each legacy decoder in order. The
 * result is repaired, so any document the schemas admit satisfies the model's invariants.
 */
export const decodeWorkspace = <V>(input: unknown, options: {
  readonly schema: Schema.Codec<WorkspaceState<V>>
  readonly config: WorkspaceConfig
  readonly legacy?: readonly LegacyDecoder<V>[] | undefined
}): Option.Option<WorkspaceState<V>> => {
  const current = Schema.decodeUnknownOption(options.schema)(input)
  if (Option.isSome(current)) return Option.some(repairWorkspace(current.value, options.config))
  for (const decoder of options.legacy ?? []) {
    const legacy = Schema.decodeUnknownOption(decoder.schema)(input)
    if (Option.isSome(legacy)) return Option.some(repairWorkspace(decoder.migrate(legacy.value as never), options.config))
  }
  return Option.none()
}
