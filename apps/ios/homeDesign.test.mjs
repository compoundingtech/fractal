import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { alertsHeading, homeRows, homeSections, keepClosed, openAlerts, personAnswer } from '@fractal/views';

const now = Date.parse('2026-10-14T12:00:00Z');
const actor = 'person/demo/session/example-phone';
// What Fractal presents for each kind (docs/clients/ui-contract.md, Attention row): Home reads it and
// derives nothing, so these rows carry it as Fractal serves them.
const presented = (attention_kind, extra) => {
  const card = {
    'human-gate': { tier: 'stopped', kind: extra.review_mode === 'feedback' ? 'feedback' : 'review' },
    'person-step': extra.update ? { tier: 'later', kind: 'update' } : { tier: 'stopped', kind: 'request' },
    'agent-request': { tier: 'stopped', kind: 'request' },
    'launch-approval': { tier: 'today', kind: 'launch' },
    'revision-approval': { tier: 'today', kind: 'revision' },
    'harness-prompt': { tier: 'stopped', kind: 'prompt' },
    'harness-login': { tier: 'stopped', kind: 'login' },
    fault: { tier: null, kind: 'fault' },
  }[attention_kind] ?? { tier: null, kind: null };
  const step = extra.step_run_id;
  return { ...card, agent: null, raised_by: null, waiting: step ? { step, path: step.split('/').pop() } : null };
};
const attention = (id, attention_kind, extra = {}) => ({
  id: `attention/demo/${id}`, kind: 'attention', attention_kind, title: 'Review the example',
  detail: 'Invented test content.', priority: 'normal', state: 'open', person_id: 'person/demo',
  requested_at: '2026-10-14T11:00:00Z', source_id: `step-run/demo/${id}`, actions: [],
  presentation: presented(attention_kind, extra), ...extra,
});
const update = attention('update', 'person-step', {
  actions: ['work.done'], update: { version: 1, type: 'update', summary: 'Example completed.', about: 'message/demo' },
});
const items = [
  attention('launch', 'launch-approval', { actions: ['launch.approve'] }),
  attention('review', 'human-gate', { actions: ['review.approve'], step_run_id: 'step-run/demo/review' }),
  attention('request', 'person-step', { actions: ['work.done'] }),
  attention('request-without-action', 'agent-request'),
  attention('prompt', 'harness-prompt', { actions: ['prompt.respond'] }),
  attention('login', 'harness-login'),
  update,
  attention('closed', 'human-gate', { closedElsewhere: true }),
  attention('resolved', 'human-gate', { state: 'resolved' }),
  attention('someone-else', 'human-gate', { person_id: 'person/other' }),
  attention('message', 'unread-message', { actions: ['message.read'] }),
  attention('fault', 'fault'),
  attention('unknown', 'future-kind'),
];

// Exercise the actual model Home renders, not a second presentation implementation.
test('Home preserves identity and tier ordering even for duplicate titles and unsupported actions', () => {
  const before = structuredClone(items);
  const rows = homeRows(items, actor, now);
  assert.deepEqual(rows.map(row => row.item.id), [
    'attention/demo/review', 'attention/demo/request', 'attention/demo/request-without-action',
    'attention/demo/prompt', 'attention/demo/login', 'attention/demo/launch',
    'attention/demo/update', 'attention/demo/closed',
  ]);
  for (const row of rows) assert.equal(row.item, items.find(item => item.id === row.item.id));
  assert.equal(new Set(rows.map(row => row.item.id)).size, rows.length);
  assert.equal(rows[0].waiting, 'step review');
  assert.equal(rows[0].age, '1h');
  assert.deepEqual(homeSections(rows).map(section => [section.tier, section.count, !!section.closed]), [
    ['stopped', 5, false], ['today', 1, false], ['later', 1, false], ['stopped', 1, true],
  ]);
  assert.deepEqual(items, before, 'rendering rows and sections must not read or resolve anything');
});

test('the Home open count excludes updates and retained closed rows, not requests lacking app actions', () => {
  const open = openAlerts(items, actor);
  assert.deepEqual(open.map(item => item.id), [
    'attention/demo/launch', 'attention/demo/review', 'attention/demo/request',
    'attention/demo/request-without-action', 'attention/demo/prompt', 'attention/demo/login',
  ]);
  assert.equal(alertsHeading(open.length), '6 alerts');
  assert.equal(alertsHeading(openAlerts([update, items[7]], actor).length), '');
  assert.equal(alertsHeading(openAlerts([items[1]], actor).length), '1 alert');
  assert.deepEqual(openAlerts([attention('not-an-alert', 'person-step', { alert: false })], actor), []);
});

test('updates remain listed until explicit read, while externally closed updates remain clearable', () => {
  const before = structuredClone(update);
  assert.equal(homeRows([update], actor, now)[0].kind, 'update');
  assert.deepEqual(update, before);
  const retained = keepClosed([update], [], new Set(), actor);
  assert.equal(retained[0].id, update.id);
  assert.equal(retained[0].closedElsewhere, true);
  assert.deepEqual(retained[0].actions, []);
  assert.equal(homeSections(homeRows(retained, actor, now))[0].closed, true);
  assert.equal(openAlerts(retained, actor).length, 0);
  assert.deepEqual(keepClosed([update], [], new Set([update.id]), actor), []);
  assert.deepEqual(keepClosed(retained, [], new Set([update.id]), actor), []);
  assert.deepEqual(personAnswer(undefined, 'read', 'Read'), { id: 'read' }, 'explicit read preserves the typed work.done answer');
});

// There is no React Native renderer harness in this app. Parse TSX only for the wiring and
// accessibility contracts that cannot be exercised through the shared pure model above.
const source = path => ts.createSourceFile(path, readFileSync(new URL(path, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nowSource = source('./screens/Now.tsx');
const uiSource = source('./ui.tsx');
const nodes = (root, predicate) => {
  const result = [];
  const visit = node => { if (predicate(node)) result.push(node); ts.forEachChild(node, visit); };
  visit(root);
  return result;
};
const namedFunction = (root, name) => {
  const found = nodes(root, node => ts.isFunctionDeclaration(node) && node.name?.text === name)[0];
  assert.ok(found, `${name} exists`);
  return found;
};
const ancestor = (node, predicate) => {
  for (let parent = node.parent; parent; parent = parent.parent) if (predicate(parent)) return parent;
};
const elements = (root, name) => nodes(root, node => (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) && node.tagName.getText() === name);
const prop = (element, name) => element.attributes.properties.find(attribute => ts.isJsxAttribute(attribute) && attribute.name.getText() === name)?.initializer;
const calls = (root, expression) => nodes(root, node => ts.isCallExpression(node) && node.expression.getText() === expression);
const home = namedFunction(nowSource, 'NowScreen');
const detail = namedFunction(nowSource, 'AttentionScreen');

test('Home keys and both Open destinations use the attention ID, with no action on opening', () => {
  const list = elements(home, 'SectionList')[0];
  assert.ok(list);
  assert.equal(prop(list, 'keyExtractor').expression.getText(), 'row => row.item.id');
  const destinations = calls(home, 'navigation.navigate').filter(call => call.arguments[0]?.text === 'Attention');
  assert.equal(destinations.length, 2, 'row tap and context-menu Open both reach Attention');
  for (const call of destinations) {
    assert.equal(call.arguments[1].getText(), '{ id: row.item.id }');
    const handler = ancestor(call, ts.isArrowFunction);
    assert.ok(handler);
    assert.equal(handler.body, call, 'opening only navigates: no markRead, done, or clearClosed side effect');
  }
  assert.equal(calls(detail, 'data.attention.find')[0].arguments[0].getText(), 'candidate => candidate.id === route.params.id');
  assert.equal(calls(home, 'openAlerts')[0].getText(), 'openAlerts(data.attention, caps?.session_actor)');
});

test('detail reads are explicit, action-capability gated, and disabled while busy or offline', () => {
  const writes = [...calls(detail, 'actions.done'), ...calls(detail, 'actions.markRead')];
  assert.equal(writes.length, 3, 'update read, step completion, and message read remain distinct');
  for (const call of writes) {
    const onPress = ancestor(call, node => ts.isJsxAttribute(node) && node.name.getText() === 'onPress');
    assert.ok(onPress, 'no read or completion during rendering, opening, or an effect');
    const button = ancestor(call, node => ts.isJsxSelfClosingElement(node) && node.tagName.getText() === 'Button');
    assert.ok(button);
    assert.equal(prop(button, 'disabled').expression.getText(), "busy || status !== 'online'");
    const gate = ancestor(button, ts.isConditionalExpression);
    assert.ok(gate);
    const action = call.expression.getText() === 'actions.markRead' ? 'message.read' : 'work.done';
    assert.equal(gate.condition.getText(), `item.actions.includes('${action}')`);
  }
  const read = calls(detail, 'actions.done').find(call => call.arguments[1]?.text === 'Read');
  assert.ok(read, 'updates are read through work.done, never message.read');
  assert.equal(read.arguments[2].text, 'read');
  const button = ancestor(read, node => ts.isJsxSelfClosingElement(node) && node.tagName.getText() === 'Button');
  assert.equal(prop(button, 'label').text.toLowerCase(), 'mark read');
  assert.equal(calls(detail, 'actions.markRead')[0].arguments[0].getText(), 'item');
});

test('Home and detail opt into grouped system presentation without a glyph legend', () => {
  const uiImport = nodes(nowSource, node => ts.isImportDeclaration(node) && node.moduleSpecifier.text === '../ui')[0];
  assert.ok(uiImport);
  const screenImport = nodes(uiImport, node => ts.isImportSpecifier(node) && (node.propertyName ?? node.name).text === 'SystemScreen')[0];
  assert.ok(screenImport, 'the wrapper is the system presentation export, regardless of its local alias');
  const screenName = screenImport.name.text;
  assert.equal(elements(home, screenName).length, 1);
  assert.ok(elements(detail, screenName).length > 0);
  assert.equal(nodes(uiImport, node => ts.isImportSpecifier(node) && (node.propertyName ?? node.name).text === 'Screen').length, 0, 'the legacy wrapper is not imported');
  assert.equal(elements(home, 'GroupedRow').length, 1);
  assert.equal(elements(home, 'GroupedHeader').length, 1);
  assert.equal(elements(home, 'Legend').length, 0);
  assert.equal(nodes(home, node => ts.isIdentifier(node) && node.text === 'HOME_LEGEND').length, 0);
});

test('grouped rows expose readable wrapping button labels and shared buttons expose disabled state', () => {
  const row = namedFunction(uiSource, 'GroupedRow');
  const pressable = elements(row, 'Pressable')[0];
  assert.equal(prop(pressable, 'accessibilityRole').expression.getText(), "onPress ? 'button' : 'text'", 'a row without a destination only reads');
  assert.equal(prop(pressable, 'accessibilityLabel').expression.getText(), '`${title}. ${context}`');
  assert.equal(prop(pressable, 'accessibilityHint').expression.getText(), 'onPress ? hint : undefined');
  assert.match(row.getText(), /hint = 'Opens alert details'/, 'an alert row still says where it goes');
  assert.equal(prop(pressable, 'onPress').expression.getText(), 'onPress', 'taps retain the detail destination');
  assert.equal(prop(pressable, 'onLongPress').expression.getText(), 'onLongPress', 'ContextMenu can inject the existing action sheet');
  assert.ok(elements(row, 'T').every(element => !prop(element, 'numberOfLines')), 'titles and context wrap rather than truncate');
  assert.equal(prop(pressable, 'disabled'), undefined, 'a read-only row reads as text, not as a dimmed control');
  assert.equal(prop(pressable, 'accessibilityState'), undefined);
  const header = elements(namedFunction(uiSource, 'GroupedHeader'), 'View')[0];
  assert.equal(prop(header, 'accessibilityRole').text, 'header');
  const button = elements(namedFunction(uiSource, 'Button'), 'Pressable')[0];
  assert.equal(prop(button, 'accessibilityRole').text, 'button');
  assert.equal(prop(button, 'accessibilityLabel').expression.getText(), 'label');
  assert.equal(prop(button, 'accessibilityState').expression.getText(), '{ disabled }');
  assert.equal(prop(button, 'disabled').expression.getText(), 'disabled');
});

test('the native app permits the device light and dark appearance', () => {
  const config = JSON.parse(readFileSync(new URL('./app.json', import.meta.url), 'utf8'));
  assert.equal(config.expo.userInterfaceStyle, 'automatic');
  assert.equal(config.expo.ios.infoPlist.UIViewControllerBasedStatusBarAppearance, true, 'native stacks own status-bar appearance without an RN screens assertion');
});

test('Now draws its background, text and native chrome from the live system appearance', () => {
  const systemScreen = namedFunction(uiSource, 'SystemScreen');
  assert.equal(calls(systemScreen, 'useSystemColors').length, 1, 'the background re-renders with the appearance');
  assert.equal(calls(namedFunction(uiSource, 'useSystemColors'), 'useScheme').length, 1);
  assert.equal(calls(namedFunction(uiSource, 'useTextAppearance'), 'useScheme').length, 1, 'every T reads the appearance itself');
  assert.equal(calls(namedFunction(uiSource, 'useScheme'), 'useColorScheme').length, 1);
  const appSource = source('./App.tsx');
  const stack = namedFunction(appSource, 'TabStack');
  assert.equal(calls(stack, 'useScheme').length, 1, 'native header options follow the appearance');
  for (const name of ['NowRoot', 'Attention']) {
    const screen = elements(stack, 'Stack.Screen').find(element => prop(element, 'name')?.text === name);
    assert.match(prop(screen, 'options').expression.getText(), /^\{ \.\.\.systemOptions,/, name);
  }
  assert.equal(calls(namedFunction(appSource, 'Main'), 'useScheme').length, 1, 'the navigation theme and tab tint follow the appearance');
});

test('paired availability renders the server person and clears unsupported reconnects', () => {
  const section = namedFunction(source('./screens/Availability.tsx'), 'AvailabilitySection');
  assert.equal(nodes(section, node => ts.isIdentifier(node) && node.text === 'actor').length, 0, 'a device session actor is not a concrete person ID');
  const store = source('./store.tsx');
  const read = nodes(store, node => ts.isVariableDeclaration(node) && node.name.getText() === 'readAvailability')[0];
  assert.ok(read);
  const guard = nodes(read, ts.isIfStatement).find(node => node.expression.getText().includes('!servesAvailability(caps)'));
  assert.ok(guard);
  assert.equal(calls(guard.thenStatement, 'setAvailability')[0].arguments[0].getText(), 'NO_AVAILABILITY', 'a daemon without availability must not retain the previous daemon person');
  assert.equal(calls(read, 'ownAvailability')[0].arguments.length, 2, 'the server resolves the paired person; do not filter by session actor');
  const publish = nodes(read, ts.isIfStatement).filter(node => calls(node.thenStatement, 'setAvailability').length);
  assert.ok(publish.filter(node => node !== guard).every(node => node.expression.getText().includes('availabilityRead.current === read')), 'late reads cannot restore availability after reconnect invalidated it');
});

test('availability writes invalidate old reads, serialize edits and discard old connection replies', async () => {
  const write = namedFunction(source('./store.tsx'), 'runAvailabilityWrite');
  const javascript = ts.transpileModule(write.getText(), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const caps = { session_actor: actor };
  const capsRef = { current: caps }, availabilityRead = { current: 1 }, availabilityWriting = { current: null };
  let value = { person: { person: 'person/demo', status: 'available' }, note: null, issue: '' };
  const run = new Function('caps', 'capsRef', 'availabilityRead', 'availabilityWriting', 'setAvailability', 'runAction', 'notApplied', 'writeRefusal',
    `${javascript}; return runAvailabilityWrite;`)(caps, capsRef, availabilityRead, availabilityWriting, update => { value = update(value); }, async action => { await action(); return true; }, () => false, String);
  let finish;
  const pending = run(() => new Promise(resolve => { finish = resolve; }));
  assert.equal(availabilityRead.current, 2, 'starting a write invalidates a previously issued read');
  assert.equal(await run(async () => { assert.fail('a second edit must not race'); }), false);
  finish({ person: { person: 'person/demo', status: 'away' } });
  assert.equal(await pending, true);
  assert.equal(value.person.status, 'away');
  assert.equal(availabilityWriting.current, null);
  const oldConnection = run(() => new Promise(resolve => { finish = resolve; }));
  capsRef.current = { session_actor: 'person/other/session/phone' };
  value = { person: { person: 'person/other', status: 'available' }, note: null, issue: '' };
  finish({ person: { person: 'person/demo', status: 'away' }, note: { text: 'old connection' } });
  await oldConnection;
  assert.equal(value.person.person, 'person/other');
  assert.equal(value.note, null);
  capsRef.current = caps;
  const disconnected = run(() => new Promise(resolve => { finish = resolve; }));
  ++availabilityRead.current;
  value = { person: null, note: null, issue: '' };
  finish({ note: { text: 'old connection' } });
  await disconnected;
  assert.equal(value.note, null, 'disconnect invalidation fences a late write even before capabilities change');
});

test('presence answers apply only between writes at the same read generation on the same connection', async () => {
  const store = source('./store.tsx');
  const construction = nodes(store, ts.isNewExpression).find(node => node.expression.getText().includes('PresenceReporter'));
  const reportProperty = nodes(construction, ts.isPropertyAssignment).find(node => node.name.getText() === 'report');
  assert.ok(reportProperty, 'the PresenceReporter report callback is found');
  const javascript = ts.transpileModule(`const report = ${reportProperty.initializer.getText()};`,
    { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const caps = { session_actor: actor };
  const capsRef = { current: caps }, availabilityRead = { current: 1 }, availabilityWriting = { current: null };
  let value = { person: { person: 'person/demo', status: 'available' }, note: null, issue: '' };
  let finish;
  const report = new Function('caps', 'capsRef', 'availabilityRead', 'availabilityWriting', 'setAvailability', 'client',
    `${javascript}; return report;`)(caps, capsRef, availabilityRead, availabilityWriting, update => { value = update(value); },
    { presenceReport: () => new Promise(resolve => { finish = () => resolve({ value: { availability: { person: 'person/demo', status: 'away' }, report_again_at: null } }); }) });
  const beforeWrite = report({ state: 'active' });
  ++availabilityRead.current;
  finish();
  await beforeWrite;
  assert.equal(value.person.status, 'available', 'a write that started while the phone was reporting wins over its answer');
  availabilityWriting.current = caps;
  const duringWrite = report({ state: 'active' });
  finish();
  await duringWrite;
  assert.equal(value.person.status, 'available', 'an edit in flight keeps its own answer the last word');
  availabilityWriting.current = null;
  const betweenWrites = report({ state: 'active' });
  finish();
  await betweenWrites;
  assert.equal(value.person.status, 'away', 'between writes the answer still refreshes the person');
  const oldConnection = report({ state: 'active' });
  capsRef.current = { session_actor: 'person/other/session/phone' };
  value = { person: { person: 'person/other', status: 'available' }, note: null, issue: '' };
  finish();
  await oldConnection;
  assert.equal(value.person.person, 'person/other', 'a replaced connection cannot repopulate availability');
});
