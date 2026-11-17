import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const React = { createElement: (type, props, ...children) => ({ type, props, children: children.flat() }) };
const source = readFileSync(new URL('./conversationComposer.tsx', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React } });
const exports = {};
new Function('require', 'exports', 'React', outputText)(() => ({ View: 'View', Field: 'Field', Button: 'Button' }), exports, React);
const find = (node, type) => !node || typeof node !== 'object' ? undefined : node.type === type ? node : node.children?.map(child => find(child, type)).find(Boolean);
const base = { text: '', label: 'Message thread', enabled: true, canSend: false, onChange() {}, onSend() {} };

test('one controlled field preserves focus, reveal, disabled sending and capability slots', () => {
  const events = [];
  const inputRef = { current: undefined }, rowRef = { current: undefined };
  const tree = exports.ConversationComposer({ ...base, inputRef, rowRef, onReveal: () => events.push('reveal'),
    before: React.createElement('Images'), leading: React.createElement('Attach'), inputAdornment: React.createElement('Dictation'),
    status: React.createElement('Status'), onChange: text => events.push(text), onSend: () => events.push('send') });
  for (const slot of ['Images', 'Attach', 'Dictation', 'Status']) assert.ok(find(tree, slot));
  const input = find(tree, 'Field');
  assert.equal(input.props.ref, inputRef);
  input.props.onChangeText('new draft'); input.props.onFocus();
  assert.deepEqual(events, ['new draft', 'reveal']);
  assert.equal(find(tree, 'Button').props.disabled, true);
  assert.equal(input.props.accessibilityLabel, undefined);
  assert.equal(input.props.placeholder, 'Message thread');
});

test('image-only send is supported by capability policy; text-only thread still refuses empty text', () => {
  const ordinary = exports.ConversationComposer({ ...base, canSend: true, sendLabel: 'send' });
  assert.equal(find(ordinary, 'Button').props.disabled, false);
  const thread = exports.ConversationComposer(base);
  assert.equal(find(thread, 'Button').props.disabled, true);
  assert.equal(find(exports.ConversationComposer({ ...base, text: 'Reply', canSend: true }), 'Field').props.accessibilityLabel, 'Message thread');
  assert.equal(find(exports.ConversationComposer({ ...base, enabled: false, canSend: true }), 'Button').props.disabled, true);
});

test('ordinary capabilities and thread quote routing remain production adapters, not a common protocol', () => {
  const ordinary = readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8');
  const threads = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  assert.match(ordinary, /<ConversationComposer/);
  assert.match(ordinary, /accessibilityLabel="Attach an image"/);
  assert.match(ordinary, /accessibilityLabel="Dictate a message"/);
  assert.match(ordinary, /ImagePicker\.launchImageLibraryAsync/);
  assert.match(ordinary, /Clipboard\.getImageAsync/);
  assert.match(ordinary, /startDictation/);
  assert.match(ordinary, /actions\.send\(agent\.id, text, agent\.current_session_id \?\? undefined, tags, sending\)/);
  assert.match(ordinary, /<EntryView/);
  assert.match(ordinary, /<MailImageView/);
  assert.match(ordinary, /subagentSession/);
  assert.match(threads, /return ConversationComposer\(/);
  assert.match(threads, /label="Remove quote"/);
  assert.match(threads, /actions\.replyToThread\(thread, content, draft\.quote\?\.id\)/);
  assert.match(threads, /!enabled \|\| !content \|\| issue/);
});

test('static layout contract: width flex stays on the wrapper, never on its column input (not native height proof)', () => {
  for (const inputStyle of [undefined, { minHeight: 36, maxHeight: 140 }]) {
    const tree = exports.ConversationComposer({ ...base, inputStyle });
    const input = find(tree, 'Field');
    const parent = node => !node || typeof node !== 'object' ? undefined : node.children?.includes(input) ? node
      : node.children?.map(parent).find(Boolean);
    assert.equal(parent(tree).props.style.flex, 1);
    assert.equal(input.props.style?.flex, undefined);
    assert.equal(input.props.style, inputStyle, 'context intrinsic min/max heights pass through unchanged');
    assert.equal(input.props.multiline, true);
  }
});

test('thread adapter routes quote removal and validation through the common composer slots', () => {
  const threadSource = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  const start = threadSource.indexOf('function ThreadComposeField(');
  const { outputText } = ts.transpileModule(threadSource.slice(start) + '\nexports.field = ThreadComposeField;', {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  });
  const adapter = {};
  new Function('exports', 'ConversationComposer', outputText)(adapter, exports.ConversationComposer);
  let removed = 0;
  const quote = React.createElement('Quote', { onRemove: () => removed++ });
  const validation = React.createElement('Validation');
  const tree = adapter.field({ ...base, before: quote, after: validation });
  assert.equal(find(tree, 'Quote'), quote);
  assert.equal(find(tree, 'Validation'), validation);
  find(tree, 'Quote').props.onRemove();
  assert.equal(removed, 1);
  assert.match(threadSource, /before=\{draft.quote/);
  assert.match(threadSource, /after=\{issue/);
});
