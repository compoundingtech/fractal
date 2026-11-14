import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';

const source = readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8');
const start = source.indexOf('  function sendAgain(item: Pending) {');
const end = source.indexOf('  // A sent message waits', start);
assert.ok(start >= 0 && end > start);
// Exercise the actual native-alert callback without loading React Native. Its
// dependencies are injected; the callback itself comes unchanged from the screen.
const confirmation = stripTypeScriptTypes(source.slice(start, end));

test('Send as new resolves the current member when the person confirms, not when the alert opens', async () => {
  assert.match(source, /const actionsRef = useRef\(actions\);\s*actionsRef\.current = actions;/);
  const sent = [];
  const oldActions = { send: async () => { sent.push('old'); return { how: 'sent' }; } };
  const currentActions = { send: async (...parameters) => { sent.push(['current', parameters]); return { how: 'sent' }; } };
  const actionsRef = { current: oldActions };
  let buttons;
  const settled = [];
  const sendAgain = new Function('actionsRef', 'actions', 'Alert', 'setPending', 'settle', 'setSendIssue', `return (${confirmation});`)(
    actionsRef, oldActions,
    { alert: (_title, _body, options) => { buttons = options; } },
    () => {}, (...args) => settled.push(args), () => {},
  );
  const item = {
    id: 'pending/1', pickedImages: [{ base64: 'image' }],
    unconfirmed: { member: 'original', request: { parameters: { to: 'agent/example', content: 'hello', session_id: 'session/1', tags: ['dictated'] } } },
  };
  sendAgain(item);
  assert.deepEqual(sent, [], 'opening the alert sends nothing');
  // A store update selects another member while the native alert stays open.
  actionsRef.current = currentActions;
  await buttons.find(button => button.text === 'Send as new').onPress();
  assert.deepEqual(sent, [['current', ['agent/example', 'hello', 'session/1', ['dictated'], item.pickedImages]]]);
  assert.deepEqual(settled, [['pending/1', { how: 'sent' }]]);
});
