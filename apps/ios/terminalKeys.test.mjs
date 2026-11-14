import assert from 'node:assert/strict';
import { InputQueue, controlBytes, fieldTyping, keyBytes, plainTyping, rawInput, wheelBytes } from './terminalKeys.ts';
import { withCursor } from './terminalStyle.ts';

// Arrows follow the program's cursor-key mode, as vim and less ask for it.
assert.equal(keyBytes('up'), '\x1b[A');
assert.equal(keyBytes('up', { application_cursor: true }), '\x1bOA');
assert.equal(keyBytes('left', { application_cursor: false }), '\x1b[D');
assert.equal(keyBytes('escape'), '\x1b');
assert.equal(keyBytes('backspace'), '\x7f');
assert.equal(keyBytes('enter'), '\r');

assert.equal(controlBytes('c'), '\x03');
assert.equal(controlBytes('W'), '\x17');
assert.equal(controlBytes('['), '\x1b');
assert.equal(controlBytes(' '), '\0');
assert.equal(controlBytes('ab'), '\x01b');

assert.equal(plainTyping('it’s “x” — ok…'), 'it\'s "x" -- ok...');

// The keyboard field keeps what was typed; each change sends only what it adds (#4233).
{
  const typing = changes => { let field = '', sent = ''; for (const text of changes) { sent += fieldTyping(field, text); field = text; } return sent; };
  // One character at a time, into a field that never empties.
  const prefixes = Array.from('everything cool?', (_, i) => 'everything cool?'.slice(0, i + 1));
  assert.equal(typing(prefixes), 'everything cool?');
  assert.equal(fieldTyping('ab', 'abb'), 'b');
  // A paste arrives whole, after what was there.
  assert.equal(fieldTyping('', 'git status'), 'git status');
  assert.equal(fieldTyping('ls ', 'ls -la /tmp'), '-la /tmp');
  // A field that empties starts again from nothing, repeated letters included.
  assert.equal(typing(['e', 'ee', '', 'e', 'ee']), 'eeee');
  // A delete (Backspace, sent as its own key) adds nothing, and typing after it sends only the new letter.
  assert.equal(fieldTyping('abc', 'ab'), '');
  assert.equal(typing(['a', 'ab', 'abc', 'ab', 'abd']), 'abcd');
  // Text inserted where the caret was moved is what is new, not the rest of the field.
  assert.equal(fieldTyping('abc', 'abXc'), 'X');
  assert.equal(fieldTyping('aa', 'aaa'), 'a');
  assert.equal(fieldTyping('abc', 'abc'), '');
}

assert.equal(wheelBytes(true, 4, 9, { mouse_tracking: 'drag', mouse_encoding: 'sgr' }), '\x1b[<64;5;10M');
assert.equal(wheelBytes(false, 0, 0, { mouse_tracking: 'click', mouse_encoding: 'default' }), '\x1b[M\x61\x21\x21');
assert.equal(wheelBytes(true, 0, 0, { mouse_tracking: 'none', mouse_encoding: 'sgr' }), '');

assert.equal(rawInput(':wq\r'), Buffer.from(':wq\r').toString('base64'));
assert.equal(rawInput('é\x1b'), Buffer.from('é\x1b').toString('base64'));
assert.equal(rawInput('ab'), 'YWI=');

// Keys typed while a request is out go together in the next one, in order.
{
  const sent = [], releases = [];
  const queue = new InputQueue(bytes => { sent.push(bytes); return new Promise(resolve => releases.push(resolve)); }, () => assert.fail());
  queue.push('i'); queue.push('h'); queue.push('i'); queue.push('\x1b');
  assert.deepEqual(sent, ['i']);
  releases.shift()(); await new Promise(resolve => setTimeout(resolve));
  assert.deepEqual(sent, ['i', 'hi\x1b']);
  releases.shift()(); await new Promise(resolve => setTimeout(resolve));
  assert.equal(queue.busy, false);
}
// A failed request drops what waited with it rather than typing it into an unknown state.
{
  const failures = [];
  let reject;
  const queue = new InputQueue(() => new Promise((_, no) => { reject = no; }), error => failures.push(error));
  queue.push('x'); queue.push('y');
  reject(new Error('stale')); await new Promise(resolve => setTimeout(resolve));
  assert.equal(failures.length, 1);
}

// The cursor cell is drawn inverse, splitting a run or padding past the line's end.
assert.deepEqual(withCursor([{ text: 'abc', fg: 2 }], 1), [{ text: 'a', fg: 2 }, { text: 'b', fg: 2, inverse: true }, { text: 'c', fg: 2 }]);
assert.deepEqual(withCursor([{ text: 'ab' }], 4), [{ text: 'ab' }, { text: '  ' }, { text: ' ', inverse: true }]);
assert.deepEqual(withCursor([], 0), [{ text: ' ', inverse: true }]);
