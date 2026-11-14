import { noObjectives } from '../factories/objectives.ts'
import type { VariantTable } from '../world.ts'
import { cleared } from './shared.ts'

export const objectivesVariants: VariantTable['objectives'] = {
  none: (_ctx, base) => cleared(base.objectives, noObjectives()),
  loading: (_ctx, base) => ({ ...base.objectives, loading: true }),
  // A served history link a client must not open: only http and https leave the app.
  'unsafe-link': (_ctx, base) => cleared(base.objectives, { ...base.objectives.state, list: { ...base.objectives.state.list, otel_ui_url: 'javascript:alert(1)' } }),
}
