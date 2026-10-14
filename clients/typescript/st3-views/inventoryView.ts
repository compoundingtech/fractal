import type { ConnectedClient, Device, Machine, Runtime, Usage, CollectionWindow } from '@smalltalk/st3-client';
import type { SessionView } from './sessionView.ts';

export type InventoryLists = { clients: ConnectedClient[]; usage: Usage[]; machines: Machine[]; devices: Device[]; sessions: SessionView[]; terminals: Runtime[] };
export type InventoryName = keyof InventoryLists;
const kinds = { clients: 'connected-client', usage: 'usage', machines: 'machine', devices: 'device', sessions: 'session', terminals: 'runtime' } as const;

/** Keep the server's window order and native discovery fields; ignore future resource kinds. */
export function inventoryRows<K extends InventoryName>(name: K, window: CollectionWindow): InventoryLists[K] {
  return window.items.filter(item => item.kind === kinds[name]) as InventoryLists[K];
}
