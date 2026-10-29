import assert from 'node:assert/strict';
import { inventoryRows } from '../inventoryView.ts';

const native = { id: 'session/native', kind: 'session', managed: false, native_session_id: 'native', process: { pid: 42, exact_session: true } };
const unresolved = { id: 'session/process', kind: 'session', managed: false, native_session_id: null, process: { pid: 43, exact_session: false } };
const rows = { items: [unresolved, { id: 'future/one', kind: 'future-kind' }, native], hasMore: true };
assert.deepEqual(inventoryRows('sessions', rows), [unresolved, native], 'window order and exact native discovery fields survive');
for (const [name, kind] of Object.entries({ clients: 'connected-client', usage: 'usage', machines: 'machine', devices: 'device', terminals: 'runtime' })) {
  const value = { kind, id: `${kind}/one`, value: { since_ms: 1, until_ms: 2, rows: [], limits: [] } };
  assert.deepEqual(inventoryRows(name, { items: [native, value] }), [value]);
}
