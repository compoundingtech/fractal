import { Schema } from 'effect'
import { persistedAtom } from '../state/persistence.ts'

/** One global display preference; no connection or agent identity enters its storage key. */
export const systemEventsPreference = persistedAtom({
  key: 'conversation.systemEvents',
  schema: Schema.Boolean,
  defaultValue: false,
})

/** The right panel's last tab, the same in every conversation. */
export const inspectorViewPreference = persistedAtom({
  key: 'inspector.view',
  schema: Schema.Literals(['details', 'changes', 'resources']),
  defaultValue: 'details',
})
