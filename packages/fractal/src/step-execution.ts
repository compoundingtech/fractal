import { Schema } from 'effect'

/** Response-attributed spend; null usage means no accounting evidence, not zero spend. */
export const StepExecutionUsage = Schema.Struct({
  input_tokens: Schema.Number,
  output_tokens: Schema.Number,
  cache_write_tokens: Schema.Number,
  cache_write_1h_tokens: Schema.Number,
  cached_tokens: Schema.Number,
  total_tokens: Schema.Number,
  cost_microusd: Schema.Number,
  reported_cost_microusd: Schema.Number,
  unpriced_tokens: Schema.Number,
})

/** Generic execution evidence, joined by the durable work claim rather than by a seat's live model. */
export const StepExecution = Schema.Struct({
  id: Schema.String,
  step_subject: Schema.String,
  attempt: Schema.Number,
  agent: Schema.String,
  incarnation_id: Schema.NullOr(Schema.String),
  declared_model: Schema.NullOr(Schema.String),
  observed_models: Schema.Array(Schema.String),
  fallback: Schema.NullOr(Schema.Boolean),
  started_at_unix_ms: Schema.Number,
  ended_at_unix_ms: Schema.NullOr(Schema.Number),
  duration_ms: Schema.NullOr(Schema.Number),
  status: Schema.NullOr(Schema.String),
  trace_id: Schema.NullOr(Schema.String),
  exit_evidence: Schema.NullOr(Schema.String),
  usage: Schema.NullOr(StepExecutionUsage),
})
export type StepExecution = typeof StepExecution.Type
