import type { WorldDefinition } from '../kit/world.ts'
import { syntheticFleetMidRefactor } from './fleetMidRefactor.ts'

/** The refactor fleet with the server's ungranted arrangements feature. */
export const failedSyncArrangementsAbsent: WorldDefinition = {
  ...syntheticFleetMidRefactor,
  id: 'failed-sync-arrangements-absent',
  scope: syntheticFleetMidRefactor.id,
  title: 'Failed sync: arrangements-absent',
  narrative: 'The refactor fleet stays readable while sidebar discovery refuses the ungranted arrangements feature.',
  selected: { sync: 'arrangements-absent' },
}
