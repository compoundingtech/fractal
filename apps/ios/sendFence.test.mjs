import assert from 'node:assert/strict';
import { sendFences } from './sendFence.ts';

// The first try uses the snapshot already held and reads nothing; a try after a refusal reads fresh.
{
  let reads = 0;
  const next = sendFences('snapshot/harbor/12/abc', async () => { reads++; return { snapshot_id: `snapshot/harbor/${20 + reads}/def`, subject_revisions: {} }; });
  assert.deepEqual(await next(), { snapshot_id: 'snapshot/harbor/12/abc', subject_revisions: {} });
  assert.equal(reads, 0, 'no capabilities read for the first try');
  assert.equal((await next()).snapshot_id, 'snapshot/harbor/21/def');
  assert.equal((await next()).snapshot_id, 'snapshot/harbor/22/def');
  assert.equal(reads, 2);
}

// Nothing held yet (no window has loaded): a fresh read from the start.
{
  let reads = 0;
  const next = sendFences('', async () => { reads++; return { snapshot_id: 'snapshot/harbor/5/x', subject_revisions: {} }; });
  assert.equal((await next()).snapshot_id, 'snapshot/harbor/5/x');
  assert.equal(reads, 1);
}

// Inbox actions keep exactly the revision the person saw, even after a fresh snapshot read.
{
  let reads = 0;
  const revisions = { 'attention/ask': 'revision/viewed' };
  const next = sendFences('snapshot/harbor/12/abc', async () => {
    reads++;
    return { snapshot_id: 'snapshot/harbor/20/def', subject_revisions: {} };
  }, revisions);
  revisions['attention/ask'] = 'revision/changed';
  const first = await next();
  assert.deepEqual(first.subject_revisions, { 'attention/ask': 'revision/viewed' });
  assert.equal(reads, 0);
  first.subject_revisions['attention/ask'] = 'revision/mutated-request';
  assert.deepEqual(await next(), { snapshot_id: 'snapshot/harbor/20/def', subject_revisions: { 'attention/ask': 'revision/viewed' } });
  assert.equal(reads, 1);
}

// Before any window arrives, a fresh snapshot still carries the viewed revision.
{
  const next = sendFences(undefined, async () => ({ snapshot_id: 'snapshot/harbor/20/def', subject_revisions: {} }), { 'attention/ask': 'revision/viewed' });
  assert.deepEqual((await next()).subject_revisions, { 'attention/ask': 'revision/viewed' });
}
