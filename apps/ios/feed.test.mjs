import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Feed, OPEN_WAIT_MS } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';

const settle = (ms = 20) => new Promise(resolve => setTimeout(resolve, ms));
const snapshot = index => ({ id: `snapshot/host/${index}`, host_id: 'host/one', store_index: index, projection_version: 'client-projection.v0', created_at: '2026-09-29T00:00:00Z' });
const mission = (id, title = id) => ({ kind: 'mission', id, revision: `${id}@1`, title, state: 'running', runs: [], run_generations: {} });
const agent = id => ({ kind: 'agent', id, revision: `${id}@1`, name: id, state: 'running', runtime_ids: [] });

function watch() {
  const seen = { windows: {}, connection: [], errors: [], updates: 0 };
  return {
    seen,
    handlers: {
      onWindow: (name, items, hasMore, at) => { seen.updates++; seen.windows[name] = { ids: items.map(item => item.id), hasMore, snapshot: at.id }; },
      onConnection: (state, issue) => seen.connection.push(issue ? `${state}: ${issue}` : state),
      onWindowError: (name, message) => seen.errors.push(`${name}: ${message}`),
    },
  };
}
const subscribed = socket => socket.sent.filter(command => command.kind === 'subscribe').map(command => command.id);

{
  // A pending source keeps its delivered rows and says they are stale. Ready changes
  // recover on the held subscription, canceling the pending resubscription timer.
  const { client, sockets } = fakeClient();
  const { seen, handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [100], 40);
  await settle();
  const socket = sockets[0];
  socket.frame({ kind: 'snapshot', id: 'agents', collection: 'agents', snapshot: snapshot(1), items: [agent('agent/amber'), agent('agent/blue')], order: ['agent/amber', 'agent/blue'], has_more: false });
  const before = socket.sent.length;
  const pending = { kind: 'resync', id: 'agents', collection: 'agents', code: 'internal', message: 'collection source is unavailable; held rows are stale until readiness returns', retryable: true };
  socket.frame(pending);
  socket.frame(pending);
  assert.deepEqual(seen.windows.agents.ids, ['agent/amber', 'agent/blue'], 'resync preserves the last delivered rows');
  assert.equal(seen.errors.length, 0, 'a moment of unavailability is not reported yet');
  await settle(60);
  assert.equal(seen.errors.length, 1, 'one issue per unavailable interval, once it lasted');
  assert.match(seen.errors[0], /agents: .*held rows are stale/);
  socket.frame({ kind: 'changes', id: 'agents', collection: 'agents', snapshot: snapshot(2), upserts: [agent('agent/coral')], removes: ['agent/amber'], order: ['agent/coral', 'agent/blue'], has_more: false });
  assert.deepEqual(seen.windows.agents.ids, ['agent/coral', 'agent/blue']);
  await settle(170);
  assert.equal(socket.sent.length, before, 'Ready changes cancel resubscription');
  socket.frame(pending);
  await settle(60);
  assert.equal(seen.errors.length, 2, 'a later unavailable interval is reported again');
  // A blip a write causes (unavailable, then changes at once) is never reported.
  socket.frame({ kind: 'changes', id: 'agents', collection: 'agents', snapshot: snapshot(3), upserts: [agent('agent/coral')], removes: [], order: ['agent/coral', 'agent/blue'], has_more: false });
  socket.frame(pending);
  socket.frame({ kind: 'changes', id: 'agents', collection: 'agents', snapshot: snapshot(4), upserts: [agent('agent/coral')], removes: [], order: ['agent/coral', 'agent/blue'], has_more: false });
  await settle(60);
  assert.equal(seen.errors.length, 2, 'a blip that recovered at once says nothing');
  feed.close();
}

{
  // Snapshot then changes produce ordered lists; a removal leaves the window.
  const { client, sockets } = fakeClient();
  const { seen, handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5, 40]);
  await settle();
  assert.equal(sockets.length, 1);
  assert.deepEqual(sockets[0].sent, [
    { kind: 'subscribe', id: 'attention', collection: 'attention', limit: 200 },
    { kind: 'subscribe', id: 'missions', collection: 'missions', limit: 200 },
    { kind: 'subscribe', id: 'agents', collection: 'agents', limit: 200 },
  ]);
  assert.deepEqual(seen.connection, ['connecting']);
  sockets[0].frame({ kind: 'snapshot', id: 'missions', collection: 'missions', snapshot: snapshot(1), items: [mission('mission/b'), mission('mission/a')], order: ['mission/a', 'mission/b'], has_more: true });
  assert.deepEqual(seen.windows.missions, { ids: ['mission/a', 'mission/b'], hasMore: true, snapshot: 'snapshot/host/1' });
  assert.deepEqual(seen.connection, ['connecting', 'live']);
  sockets[0].frame({ kind: 'changes', id: 'missions', collection: 'missions', snapshot: snapshot(2), upserts: [mission('mission/c')], removes: ['mission/a'], order: ['mission/c', 'mission/b'], has_more: false });
  assert.deepEqual(seen.windows.missions, { ids: ['mission/c', 'mission/b'], hasMore: false, snapshot: 'snapshot/host/2' });

  // A refused window is reported by name, and the other windows keep going.
  sockets[0].frame({ kind: 'error', id: 'attention', message: 'invalid subscription' });
  assert.deepEqual(seen.errors, ['attention: invalid subscription · trying again']);
  // Fractal stopped sending it, so it is asked for again after the backoff, said only once.
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.deepEqual(sockets[0].sent.at(-1), { kind: 'subscribe', id: 'attention', collection: 'attention', limit: sockets[0].sent.find(sent => sent.id === 'attention').limit });
  sockets[0].frame({ kind: 'error', id: 'attention', message: 'invalid subscription' });
  assert.equal(seen.errors.length, 1, 'a failure that keeps happening is said once');
  // Once it loads, nothing more is scheduled for it.
  sockets[0].frame({ kind: 'snapshot', id: 'attention', collection: 'attention', snapshot: snapshot(4), items: [], order: [], has_more: false });
  sockets[0].frame({ kind: 'snapshot', id: 'agents', collection: 'agents', snapshot: snapshot(3), items: [agent('agent/one')], order: ['agent/one'], has_more: false });
  assert.deepEqual(seen.windows.agents.ids, ['agent/one']);
  assert.deepEqual(seen.connection, ['connecting', 'live'], 'a socket goes live once, at its first snapshot');

  // A resync asks for that window again, and only that one; after a wait, so a resync that
  // keeps coming is not a loop and many clients do not ask at the same instant.
  const before = sockets[0].sent.length;
  sockets[0].frame({ kind: 'resync', id: 'missions' });
  assert.deepEqual(sockets[0].sent.slice(before), [], 'not at once');
  await settle(60);
  assert.deepEqual(sockets[0].sent.slice(before), [{ kind: 'subscribe', id: 'missions', collection: 'missions', limit: 200 }]);

  // Idle: nothing is read or sent while no frame arrives.
  const sent = sockets[0].sent.length, updates = seen.updates;
  await settle(60);
  assert.equal(sockets.length, 1);
  assert.equal(sockets[0].sent.length, sent);
  assert.equal(seen.updates, updates);
  feed.close();
  assert.equal(sockets[0].closed, true);
}

{
  // A dropped socket reconnects after the first delay and subscribes every window again; the next
  // snapshot replaces the lists. Delays grow while drops repeat and reset after a snapshot.
  const { client, sockets } = fakeClient();
  const { seen, handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5, 60]);
  await settle();
  sockets[0].frame({ kind: 'snapshot', id: 'missions', collection: 'missions', snapshot: snapshot(1), items: [mission('mission/old')], order: ['mission/old'], has_more: false });
  sockets[0].drop(new Error('daemon restarted'));
  assert.equal(seen.connection.at(-1), 'reconnecting: daemon restarted');
  await settle();
  assert.equal(sockets.length, 2, 'the first delay has passed');
  assert.deepEqual(subscribed(sockets[1]), ['attention', 'missions', 'agents']);
  sockets[1].frame({ kind: 'snapshot', id: 'missions', collection: 'missions', snapshot: snapshot(5), items: [mission('mission/new')], order: ['mission/new'], has_more: false });
  assert.deepEqual(seen.windows.missions.ids, ['mission/new']);
  // A changes frame is never applied across sockets: the new socket's snapshot is authoritative.
  sockets[0].frame({ kind: 'changes', id: 'missions', collection: 'missions', snapshot: snapshot(6), upserts: [mission('mission/ghost')], removes: [], order: ['mission/ghost'], has_more: false });
  assert.deepEqual(seen.windows.missions.ids, ['mission/new']);

  assert.equal(seen.connection.at(-1), 'live');
  sockets[1].drop(new Error('lost'));
  await settle();
  assert.equal(sockets.length, 3, 'the snapshot reset the delay');
  sockets[2].drop(new Error('lost again'));
  await settle();
  assert.equal(sockets.length, 3, 'a second drop without a snapshot waits longer');
  await settle(80);
  assert.equal(sockets.length, 4);
  // A person asking to reconnect opens a fresh socket at once.
  feed.reconnect();
  await settle(1);
  assert.equal(sockets.length, 5);
  assert.equal(sockets[3].closed, true);
  feed.close();
  sockets[4].drop(new Error('after close'));
  await settle(80);
  assert.equal(sockets.length, 5, 'a closed feed never reconnects');
}

{
  // Background closes the socket and schedules nothing; foreground opens a fresh one.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const foreground = new ForegroundGate('background');
  const feed = new Feed(client, handlers, foreground, () => 'action/test', [5]);
  await settle();
  assert.equal(sockets.length, 0, 'a feed started in the background opens nothing');
  foreground.update('active');
  await settle();
  assert.equal(sockets.length, 1);
  foreground.update('background');
  assert.equal(sockets[0].closed, true);
  sockets[0].drop(new Error('closed'));
  await settle(40);
  assert.equal(sockets.length, 1, 'nothing reconnects in the background');
  foreground.update('active');
  await settle();
  assert.equal(sockets.length, 2);
  assert.deepEqual(subscribed(sockets[1]), ['attention', 'missions', 'agents']);
  feed.close();
  foreground.update('background');
  foreground.update('active');
  await settle();
  assert.equal(sockets.length, 2, 'a closed feed ignores the foreground');
}

{
  // One conversation rides the socket: its page, then changes, and again after a reconnect.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  await settle();
  const frames = [], issues = [];
  const follow = feed.followConversation('agent/example/worker', { onEntries: frame => frames.push(frame), onIssue: issue => issues.push(issue) });
  assert.deepEqual(sockets[0].sent.at(-1), { kind: 'subscribe', id: 'conversation', collection: 'conversation', conversation: 'agent/example/worker' });
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: true, items: [{ id: 'entry/1' }], has_more: true, older_cursor: 'before-entry-1' });
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: false, items: [{ id: 'entry/2' }] });
  assert.deepEqual(frames.map(frame => [frame.replace, frame.items.map(item => item.id), frame.hasMore, frame.olderCursor]), [[true, ['entry/1'], true, 'before-entry-1'], [false, ['entry/2'], false, undefined]]);
  sockets[0].frame({ kind: 'error', id: 'conversation', collection: 'conversation', code: 'remote-unavailable', message: 'owner host/two is temporarily unavailable' });
  assert.equal(issues.at(-1), 'two cannot be reached right now · trying again');
  // An ended subscription is asked for again after the backoff.
  const before = sockets[0].sent.length;
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.deepEqual(sockets[0].sent.slice(before), [{ kind: 'subscribe', id: 'conversation', collection: 'conversation', conversation: 'agent/example/worker' }]);
  // A page that expired under a busy store retries quietly at first.
  const quiet = issues.length;
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: true, items: [{ id: 'entry/1' }] });
  sockets[0].frame({ kind: 'error', id: 'conversation', collection: 'conversation', code: 'page-cursor-expired', message: 'the snapshot moved' });
  assert.equal(issues.length, quiet + 1, 'only the clearing of the old issue');
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(sockets[0].sent.at(-1).collection, 'conversation');
  sockets[0].drop(new Error('lost'));
  await settle();
  assert.ok(subscribed(sockets[1]).includes('conversation'), 'a reconnect follows the conversation again');
  follow.close();
  assert.deepEqual(sockets[1].sent.at(-1), { kind: 'unsubscribe', id: 'conversation' });
  sockets[1].drop(new Error('lost'));
  await settle();
  assert.ok(!subscribed(sockets[2]).includes('conversation'), 'a closed conversation is not followed again');
  feed.close();
}

{
  // Fractal keeps the subscription while the owner's host is away and says why: the copy shown is
  // stale, and the phone does not ask again itself.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  await settle();
  const issues = [];
  feed.followConversation('agent/example/worker', { onEntries: () => {}, onIssue: issue => issues.push(issue) });
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: true, items: [{ id: 'entry/1' }] });
  const before = sockets[0].sent.length;
  sockets[0].frame({ kind: 'resync', id: 'conversation', collection: 'conversation', retryable: true, code: 'remote-unavailable', message: 'owner host/two is temporarily unavailable; cached data remains usable' });
  assert.equal(issues.at(-1), 'two cannot be reached right now · trying again');
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(sockets[0].sent.length, before, 'Fractal retries; the phone does not');
  // Once Fractal reaches it again, the issue clears.
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: false, items: [{ id: 'entry/2' }] });
  assert.equal(issues.at(-1), '');
  feed.close();
}

{
  // Fractal keeps no start for this transcript: the phone says Fractal's reason once and stops asking.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  await settle();
  const issues = [], stops = [];
  feed.followConversation('agent/example/worker', { onEntries: () => {}, onIssue: (issue, stopped) => { issues.push(issue); stops.push(stopped); } });
  const before = sockets[0].sent.length;
  const reason = 'the retained transcript starts after its first entries; st cannot show it from the start';
  sockets[0].frame({ kind: 'error', id: 'conversation', collection: 'conversation', code: 'timeline-history-incomplete', message: reason });
  assert.equal(issues.at(-1), 'the retained transcript starts after its first entries; Fractal cannot show it from the start', 'the server reason is rendered without changing the raw message or adding "trying again"');
  assert.equal(stops.at(-1), true, 'the screen offers a retry instead of loading forever');
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(sockets[0].sent.length, before, 'not asked for again');
  feed.close();
}

{
  // The person's glasses ride the socket while followed: a snapshot, changes, and again after a
  // reconnect; nothing once closed.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  await settle();
  const seen = [], issues = [];
  const follow = feed.followGlasses({ onGlasses: glasses => seen.push(glasses.map(glass => glass.body.name)), onIssue: issue => issues.push(issue) });
  assert.deepEqual(sockets[0].sent.at(-1), { kind: 'subscribe', id: 'glasses', collection: 'glasses', limit: 100 });
  const glass = (id, name) => ({ id: `glass/person/avery/${id}`, kind: 'glass', revision: 'r1', updated_at: '', deleted: false, body: { name, tabs: [] } });
  const snapshot = { id: 'snapshot/1', host_id: 'host/one', store_index: 1, projection_version: '1', created_at: '' };
  sockets[0].frame({ kind: 'snapshot', id: 'glasses', collection: 'glasses', snapshot, items: [glass('a', 'main')], order: ['glass/person/avery/a'], has_more: false });
  sockets[0].frame({ kind: 'changes', id: 'glasses', collection: 'glasses', snapshot, upserts: [glass('b', 'review')], removes: [], order: ['glass/person/avery/a', 'glass/person/avery/b'], has_more: false });
  assert.deepEqual(seen, [['main'], ['main', 'review']]);
  sockets[0].frame({ kind: 'error', id: 'glasses', collection: 'glasses', code: 'forbidden', message: 'this device lacks read.glasses' });
  assert.equal(issues.at(-1), 'not allowed: this device lacks read.glasses');
  sockets[0].drop(new Error('lost'));
  await settle();
  assert.ok(subscribed(sockets[1]).includes('glasses'), 'a reconnect follows the glasses again');
  follow.close();
  assert.deepEqual(sockets[1].sent.at(-1), { kind: 'unsubscribe', id: 'glasses' });
  feed.close();
}



{
  // The missions window is followed only when asked: not subscribed at the start, subscribed when a
  // missions screen shows, and left (its rows dropped) when none does.
  const { client, sockets } = fakeClient();
  const { seen, handlers } = watch();
  const stopped = [];
  const feed = new Feed(client, { ...handlers, onWindowStopped: name => stopped.push(name) }, new ForegroundGate('active'), () => 'action/test', [100], 1000, false);
  await settle();
  const socket = sockets[0];
  assert.ok(!subscribed(socket).includes('missions'), 'not followed at the start');
  assert.ok(subscribed(socket).includes('agents') && subscribed(socket).includes('attention'));
  feed.setMissions(true);
  assert.ok(subscribed(socket).includes('missions'), 'followed once a missions screen shows');
  feed.setMissions(false);
  assert.ok(socket.sent.some(command => command.kind === 'unsubscribe' && command.id === 'missions'), 'left when none shows');
  assert.deepEqual(stopped, ['missions']);
  feed.setMissions(false);
  assert.deepEqual(stopped, ['missions'], 'leaving twice says nothing more');
  // Frames already on their way when it was left change nothing: no rows, no retry, no error.
  const sent = socket.sent.length;
  socket.frame({ kind: 'snapshot', id: 'missions', collection: 'missions', snapshot: snapshot(9), items: [mission('mission/late')], order: ['mission/late'], has_more: false });
  socket.frame({ kind: 'changes', id: 'missions', collection: 'missions', snapshot: snapshot(10), upserts: [mission('mission/later')], removes: [], order: ['mission/later'], has_more: false });
  socket.frame({ kind: 'resync', id: 'missions', collection: 'missions', code: 'internal', message: 'collection source is unavailable', retryable: true });
  socket.frame({ kind: 'error', id: 'missions', code: 'internal', message: 'the window failed', retryable: true });
  await settle(1100);
  assert.equal(seen.windows.missions, undefined, 'a late missions frame brings no rows back');
  assert.equal(seen.errors.length, 0, 'and no error');
  assert.equal(socket.sent.length, sent, 'and is not subscribed again');
  feed.close();
}

{
  // Screens ask and give back; the window stays for a grace period after the last one leaves.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const stopped = [];
  const feed = new Feed(client, { ...handlers, onWindowStopped: name => stopped.push(name) }, new ForegroundGate('active'), () => 'action/test', [100], 1000, false, 40);
  await settle();
  const socket = sockets[0];
  const first = feed.watchMissions();
  const second = feed.watchMissions();
  assert.equal(subscribed(socket).filter(id => id === 'missions').length, 1, 'subscribed once for two screens');
  first();
  first();
  await settle(80);
  assert.deepEqual(stopped, [], 'one screen still shows missions');
  second();
  await settle(10);
  assert.deepEqual(stopped, [], 'kept through the grace period');
  const again = feed.watchMissions();
  await settle(80);
  assert.deepEqual(stopped, [], 'asked again within the grace: still followed');
  again();
  await settle(80);
  assert.deepEqual(stopped, ['missions'], 'left once the grace ended');
  feed.close();
}

{
  // A conversation frame carries the header when Fractal sends one (`conversation-blocks.v1`);
  // without it there is none, and nothing assumes the field.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  await settle();
  const frames = [];
  feed.followConversation('agent/example/worker', { onEntries: frame => frames.push(frame), onIssue: () => {} });
  const header = { model: { value: 'synthetic/model', source: 'transcript', as_of: '2026-10-06T12:00:00Z' }, working: { value: true, source: 'register', as_of: '2026-10-06T12:00:00Z' } };
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: true, items: [{ id: 'entry/1' }], has_more: false, header });
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: false, items: [] });
  sockets[0].frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/one', replace: true, items: [], header: { model: { value: 'x', source: 'register', as_of: '2026-10-06T12:00:00Z' } } });
  assert.deepEqual(frames.map(frame => frame.header), [header, undefined, { model: { value: 'x', source: 'register', as_of: '2026-10-06T12:00:00Z' } }]);
  feed.close();
}

{
  // Sidebar follows an explicit owner, applies stream changes and resubscribes on reconnect.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  const seen = [], issues = [];
  const follow = feed.followArrangements('person/avery', { onArrangements: rows => seen.push(rows), onIssue: issue => issues.push(issue) });
  await settle();
  assert.deepEqual(sockets[0].sent.at(-1), { kind: 'subscribe', id: 'arrangements', collection: 'arrangements', person: 'person/avery', limit: 100 });
  const arrangement = { kind: 'arrangement', id: 'arrangement/person/avery/demo', owner: 'person/avery', revision: 'claim/1', deleted: false, body: { version: 1, name: { value: 'Sidebar', revision: 'claim/1' }, folders: {}, placements: {} } };
  sockets[0].frame({ kind: 'snapshot', id: 'arrangements', collection: 'arrangements', snapshot: snapshot(1), items: [arrangement], order: [arrangement.id], has_more: false });
  assert.equal(seen[0][0].body.name.value, 'Sidebar');
  sockets[0].frame({ kind: 'changes', id: 'arrangements', collection: 'arrangements', snapshot: snapshot(2), upserts: [{ ...arrangement, body: { ...arrangement.body, name: { value: 'Renamed', revision: 'claim/2' } } }], removes: [], order: [arrangement.id], has_more: false });
  assert.equal(seen.at(-1)[0].body.name.value, 'Renamed');
  sockets[0].frame({ kind: 'resync', id: 'arrangements', retryable: true });
  assert.equal(sockets[0].sent.filter(command => command.id === 'arrangements').length, 1, 'daemon retries temporary resync itself');
  sockets[0].drop(new Error('test drop'));
  await settle(40);
  assert.equal(sockets[1].sent.at(-1).person, 'person/avery');
  sockets[1].frame({ kind: 'snapshot', id: 'arrangements', collection: 'arrangements', snapshot: snapshot(3), items: [], order: [], has_more: false });
  assert.deepEqual(seen.at(-1), []);
  sockets[1].frame({ kind: 'error', id: 'arrangements', code: 'forbidden', message: 'scope missing' });
  assert.ok(issues.at(-1).includes('scope missing'));
  follow.close();
  assert.deepEqual(sockets[1].sent.at(-1), { kind: 'unsubscribe', id: 'arrangements' });
  feed.close();
}

{
  // Read positions: each frame of the person's collection says re-list; a reconnect subscribes again.
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  let changed = 0;
  const follow = feed.followReadPositions('person/avery', { onChanged: () => changed++ });
  await settle();
  assert.deepEqual(sockets[0].sent.at(-1), { kind: 'subscribe', id: 'read-positions', collection: 'read-positions', person: 'person/avery', limit: 100 });
  sockets[0].frame({ kind: 'snapshot', id: 'read-positions', collection: 'read-positions', snapshot: snapshot(1), items: [], order: [], has_more: false });
  sockets[0].frame({ kind: 'changes', id: 'read-positions', collection: 'read-positions', snapshot: snapshot(2), upserts: [], removes: [], order: [], has_more: false });
  assert.equal(changed, 2);
  sockets[0].frame({ kind: 'resync', id: 'read-positions', retryable: true });
  assert.equal(sockets[0].sent.filter(command => command.id === 'read-positions').length, 1, 'daemon retries temporary resync itself');
  sockets[0].frame({ kind: 'resync', id: 'read-positions', retryable: false });
  assert.equal(sockets[0].sent.filter(command => command.id === 'read-positions').length, 2, 'a final resync subscribes again');
  sockets[0].frame({ kind: 'error', id: 'read-positions', code: 'remote-unavailable', message: 'busy' });
  await settle(40);
  assert.equal(sockets[0].sent.filter(command => command.id === 'read-positions').length, 3, 'a transient error subscribes again later');
  sockets[0].drop(new Error('test drop'));
  await settle(40);
  assert.deepEqual(sockets[1].sent.filter(command => command.id === 'read-positions').at(-1), { kind: 'subscribe', id: 'read-positions', collection: 'read-positions', person: 'person/avery', limit: 100 });
  sockets[1].frame({ kind: 'snapshot', id: 'read-positions', collection: 'read-positions', snapshot: snapshot(3), items: [], order: [], has_more: false });
  assert.equal(changed, 3);
  follow.close();
  assert.deepEqual(sockets[1].sent.at(-1), { kind: 'unsubscribe', id: 'read-positions' });
  sockets[1].frame({ kind: 'snapshot', id: 'read-positions', collection: 'read-positions', snapshot: snapshot(4), items: [], order: [], has_more: false });
  assert.equal(changed, 3, 'nothing is heard after closing');
  feed.close();
}

// ---- A socket that never opens is a failed try, not a wait without end: the feed says reconnecting and tries again.
test('a collections socket that never opens is given up on and tried again', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  let opens = 0;
  const late = [];
  const client = { collectionStream: () => { opens++; return new Promise(resolve => late.push(resolve)); } };
  const states = [];
  const feed = new Feed(client, { onWindow() {}, onConnection: (state, issue) => states.push([state, issue]) }, new ForegroundGate('active'), () => 'action/test', [1000]);
  await Promise.resolve();
  assert.equal(opens, 1);
  assert.deepEqual(states.at(-1), ['connecting', undefined]);
  t.mock.timers.tick(OPEN_WAIT_MS);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(states.at(-1), ['reconnecting', 'Fractal did not answer'], 'a hung open ends in a retry cycle');
  t.mock.timers.tick(1000);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(opens, 2, 'and it tries again by itself');
  // The first socket opening after it was given up on is closed, not adopted.
  let closed = false;
  late[0]({ close() { closed = true; }, subscribe() {}, unsubscribe() {} });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(closed, true);
  feed.close();
});

test('the open deadline covers a returned carrier until its WebSocket handshake', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient({ autoOpen: false });
  const { seen, handlers } = watch();
  let carrierClosed = 0;
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [1000, 2000], 1000, true, 30_000,
    () => ({ close() { carrierClosed++; }, onping: null }));
  const flush = () => new Promise(resolve => setImmediate(resolve));
  await flush();
  assert.equal(sockets.length, 1);
  assert.deepEqual(subscribed(sockets[0]), [], 'no subscriptions before onOpen');
  t.mock.timers.tick(OPEN_WAIT_MS - 1);
  await flush();
  assert.equal(sockets[0].closed, false);
  t.mock.timers.tick(1);
  await flush();
  assert.equal(sockets[0].closed, true, 'the actual pending carrier is closed at 15 seconds');
  assert.ok(carrierClosed > 0);
  assert.equal(seen.connection.at(-1), 'reconnecting: Fractal did not answer');
  sockets[0].open();
  sockets[0].frame({ kind: 'snapshot', id: 'agents', collection: 'agents', snapshot: snapshot(1), items: [], order: [], has_more: false });
  assert.equal(seen.updates, 0, 'late events cannot revive the expired attempt');
  t.mock.timers.tick(1000);
  await flush();
  sockets[1].open();
  await flush();
  assert.deepEqual(subscribed(sockets[1]), ['attention', 'missions', 'agents']);
  assert.notEqual(seen.connection.at(-1), 'live', 'handshake alone does not mark the feed live');
  sockets[1].drop(new Error('before a snapshot'));
  t.mock.timers.tick(1000);
  await flush();
  assert.equal(sockets.length, 2, 'handshake alone does not reset retry backoff');
  t.mock.timers.tick(1000);
  await flush();
  sockets[2].open();
  await flush();
  sockets[2].frame({ kind: 'snapshot', id: 'agents', collection: 'agents', snapshot: snapshot(2), items: [], order: [], has_more: false });
  assert.equal(seen.connection.at(-1), 'live');
  t.mock.timers.tick(OPEN_WAIT_MS);
  await flush();
  assert.equal(sockets[2].closed, false, 'successful handshake cancels the opening deadline');
  feed.close();
});

test('Share sheet inactivity keeps the live feed and retries; background suspends both', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient();
  const foreground = new ForegroundGate('active');
  const { seen, handlers } = watch();
  const feed = new Feed(client, handlers, foreground, () => 'action/test', [1000]);
  const flush = () => new Promise(resolve => setImmediate(resolve));
  await flush();
  foreground.update('inactive');
  assert.equal(sockets[0].closed, false, 'a Share sheet does not tear down the socket');
  sockets[0].frame({ kind: 'snapshot', id: 'agents', collection: 'agents', snapshot: snapshot(1), items: [], order: [], has_more: false });
  assert.equal(seen.connection.at(-1), 'live', 'updates still arrive while inactive');
  sockets[0].drop(new Error('network loss during Share sheet'));
  t.mock.timers.tick(1000);
  await flush();
  assert.equal(sockets.length, 2, 'network failures still retry while inactive');
  foreground.update('active');
  await flush();
  assert.equal(sockets.length, 2, 'dismissing the Share sheet does not open another socket');
  foreground.update('background');
  assert.equal(sockets[1].closed, true);
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(sockets.length, 2, 'background schedules no retries');
  foreground.update('inactive');
  await flush();
  assert.equal(sockets.length, 2, 'inactive on the way back does not resume background work');
  foreground.update('active');
  await flush();
  assert.equal(sockets.length, 3);
  feed.close();
});

test('backgrounding or reconnecting during handshake closes the pending carrier', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient({ autoOpen: false });
  const foreground = new ForegroundGate('active');
  const { seen, handlers } = watch();
  const feed = new Feed(client, handlers, foreground, () => 'action/test', [1000]);
  const flush = () => new Promise(resolve => setImmediate(resolve));
  await flush();
  foreground.update('background');
  await flush();
  assert.equal(sockets[0].closed, true);
  t.mock.timers.tick(OPEN_WAIT_MS + 1000);
  await flush();
  assert.equal(sockets.length, 1);
  assert.deepEqual(seen.connection, ['connecting'], 'cancelling an attempt is not a transport failure');
  foreground.update('active');
  await flush();
  feed.reconnect();
  await flush();
  assert.equal(sockets[1].closed, true);
  sockets[1].open();
  await flush();
  assert.deepEqual(subscribed(sockets[1]), []);
  sockets[2].open();
  await flush();
  assert.deepEqual(subscribed(sockets[2]), ['attention', 'missions', 'agents']);
  feed.close();
});

test('attention hints are requested only after a grant on each paired socket', async () => {
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const feed = new Feed(client, handlers, new ForegroundGate('active'), () => 'action/test', [5]);
  try {
    await settle();
    const first = sockets[0];
    const base = first.sent.find(command => command.collection === 'attention');
    assert.equal(base.attention_thread_hints, undefined);
    first.frame({ kind: 'snapshot', collection: 'attention', id: base.id, snapshot: snapshot(1), items: [], order: [], has_more: false });
    feed.setAttentionThreadHints(true);
    const hinted = first.sent.filter(command => command.collection === 'attention').at(-1);
    assert.equal(hinted.id, base.id);
    assert.equal(hinted.attention_thread_hints, true);
    feed.setAttentionThreadHints(true);
    assert.equal(first.sent.filter(command => command.collection === 'attention').length, 2);
    feed.reconnect();
    await settle();
    const next = sockets[1].sent.find(command => command.collection === 'attention');
    assert.equal(next.attention_thread_hints, undefined);
  } finally {
    feed.close();
  }
});

test('thread windows share the paired socket, apply exact order, and follow again after reconnect', async () => {
  const { client, sockets } = fakeClient();
  const { handlers } = watch();
  const foreground = new ForegroundGate('active');
  const feed = new Feed(client, handlers, foreground, () => 'action/test', [5]);
  const rows = [], details = [], events = [], issues = [];
  const context = { origin_source: 'notice/one', origin_episode: 'episode/one' };
  const summary = feed.followThreadSummaries(context, { onWindow: window => rows.push(window.items), onIssue: (...issue) => issues.push(issue) });
  const detail = feed.followThreadSummary('thread/one', { onWindow: window => details.push(window.items), onIssue: (...issue) => issues.push(issue) });
  const timeline = feed.followThreadTimeline('thread/one', { onWindow: window => events.push(window.items), onIssue: (...issue) => issues.push(issue) });
  try {
    await settle();
    const socket = sockets[0];
    const list = socket.sent.find(command => command.collection === 'threads' && command.thread_context);
    const exact = socket.sent.find(command => command.collection === 'threads' && command.subject);
    const history = socket.sent.find(command => command.collection === 'thread-timeline');
    assert.deepEqual(list.thread_context, context);
    assert.equal(exact.subject, 'thread/one');
    assert.equal(exact.limit, 1);
    assert.equal(exact.thread_context, undefined);
    assert.equal(history.subject, 'thread/one');
    assert.equal(sockets.length, 1);
    const thread = { id: 'thread/one', title: 'Review', unread: true, message_count: 1, state: 'open' };
    const event = { id: 'message/first', kind: 'message', content: 'One', thread_quote_id: null };
    socket.frame({ kind: 'snapshot', collection: 'threads', id: list.id, snapshot: snapshot(1), items: [thread], order: [thread.id], has_more: false });
    socket.frame({ kind: 'snapshot', collection: 'threads', id: exact.id, snapshot: snapshot(1), items: [thread], order: [thread.id], has_more: false });
    socket.frame({ kind: 'snapshot', collection: 'thread-timeline', id: history.id, snapshot: snapshot(1), items: [event], order: [event.id], has_more: false });
    const changed = { ...thread, unread: false, message_count: 2 };
    socket.frame({ kind: 'changes', collection: 'threads', id: list.id, snapshot: snapshot(2), upserts: [changed], removes: [], order: [thread.id], has_more: false });
    assert.deepEqual(rows, [[thread], [changed]]);
    const read = { ...thread, unread: false };
    socket.frame({ kind: 'changes', collection: 'threads', id: exact.id, snapshot: snapshot(2), upserts: [read], removes: [], order: [thread.id], has_more: false });
    assert.deepEqual(details, [[thread], [read]], 'remote read updates exact summary without a timeline change');
    assert.deepEqual(events, [[event]]);

    feed.reconnect();
    await settle();
    assert.equal(sockets.length, 2);
    assert.equal(socket.closed, true);
    assert.ok(sockets[1].sent.some(command => command.id === list.id));
    assert.ok(sockets[1].sent.some(command => command.id === exact.id));
    assert.ok(sockets[1].sent.some(command => command.id === history.id));
    socket.frame({ kind: 'snapshot', collection: 'threads', id: list.id, snapshot: snapshot(3), items: [thread], order: [thread.id], has_more: false });
    assert.equal(rows.length, 2, 'a late frame from the old socket cannot restore its state');
    sockets[1].frame({ kind: 'error', id: list.id, code: 'forbidden', message: 'grant expired', retryable: false });
    assert.deepEqual(issues, [['not allowed: grant expired', true]]);
    summary.close();
    detail.close();
    timeline.close();
    assert.deepEqual(sockets[1].sent.slice(-3), [{ kind: 'unsubscribe', id: list.id }, { kind: 'unsubscribe', id: exact.id }, { kind: 'unsubscribe', id: history.id }]);
  } finally {
    feed.close();
  }
});
