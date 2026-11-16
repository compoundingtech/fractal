import { Schema } from 'effect'
import { geometryNumbers } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { persistedAtom } from '../state/persistence.ts'

/** Sidebar choices use the shared persistence document and each host's atom registry. */
export const usageSectionOpen = persistedAtom({
  key: 'sidebar.usageSection.open',
  schema: Schema.Boolean,
  defaultValue: false,
})
export const usageSectionHeight = persistedAtom({
  key: 'sidebar.usageSection.height',
  schema: Schema.Finite.check(Schema.isGreaterThanOrEqualTo(geometryNumbers.drawerMin), Schema.isLessThanOrEqualTo(geometryNumbers.drawerMax)),
  defaultValue: geometryNumbers.drawerDefault,
})
