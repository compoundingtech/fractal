import assert from 'node:assert/strict';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { draftSize, messageLimit, restoreDraft, sendReason, sendRefusal } from './composer.ts';

// The limit is whatever Fractal advertises; an older Fractal advertises none and nothing is blocked here.
assert.equal(messageLimit({ limits: { max_message_bytes: 262144 } }), 262144);
assert.equal(messageLimit({ limits: {} }), undefined);
assert.equal(messageLimit(null), undefined);
assert.equal(draftSize('x'.repeat(500_000), undefined).over, false);

// Short messages say nothing; a long one says how long it is against the limit.
assert.equal(draftSize('hello', 262144).note, null);
assert.equal(draftSize('a'.repeat(8192), 262144).note, null);
assert.equal(draftSize('a'.repeat(9000), 262144).note, '8.8 KiB of 256 KiB');
assert.equal(draftSize('a'.repeat(9000), 262144).over, false);

// Bytes, not characters: 3-byte characters reach the limit at a third of its length.
const euros = '€'.repeat(Math.floor(262144 / 3));
assert.equal(draftSize(euros, 262144).over, false);
assert.equal(draftSize(`${euros}€`, 262144).over, true);
assert.equal(draftSize('a'.repeat(262144), 262144).over, false);
const over = draftSize('a'.repeat(262145), 262144);
assert.equal(over.over, true);
assert.match(over.note, /Too long to send: 256 KiB, and the limit is 256 KiB/);
assert.match(over.note, /text is kept/);
assert.equal(draftSize('🎉'.repeat(70000), 262144).over, true);

// A refused send puts the text back; what was typed meanwhile stays after it, and nothing is lost.
assert.equal(restoreDraft('', 'the long notes'), 'the long notes');
assert.equal(restoreDraft('  ', 'the long notes'), 'the long notes');
assert.equal(restoreDraft('and one more thing', 'the long notes'), 'the long notes\n\nand one more thing');
assert.equal(restoreDraft('the long notes', 'the long notes'), 'the long notes');
assert.equal(restoreDraft('typed', ''), 'typed');

// What the person reads.
assert.match(sendReason('st took too long to answer; it may or may not have done it', true), /Sending again will not send it twice/);
assert.equal(sendReason('an inline message cannot exceed 4 KiB; post a document first', false), 'This Fractal version only takes messages up to 4 KiB. Shorten the message, or update Fractal. Your text is kept.');
assert.equal(sendReason('the full text is too large', false), 'the full text is too large. Your text is kept.');

// A refused send says why in the phone's words, by Fractal's code, never Fractal's sentence for an agent (#1951).
{
  const refused = (code, message) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message, retryable: false, details: {} }, 409);
  const said = (code, message) => `Not sent: ${sendReason(sendRefusal(refused(code, message)), false)}`;
  assert.equal(said('message-too-large', 'a message cannot be longer than 256 KiB; this one is 300 KiB. Shorten it or send it as several messages.'),
    'Not sent: the message is too long for Fractal; shorten it or split it into several messages. Your text is kept.');
  assert.equal(said('recipient-retired', 'Retired: recipient `agent/example` refuses this first send'), 'Not sent: this agent was retired, so it takes no more messages. Your text is kept.');
  assert.equal(said('forbidden', 'the authenticated client session is not granted `message.send`'), 'Not sent: this phone is not allowed to send this message (the authenticated client session is not granted `message.send`). Your text is kept.');
  assert.equal(sendRefusal(refused('remote-unavailable', 'host/example-box is not reachable')), 'example-box cannot be reached right now (remote-unavailable)');
  assert.equal(said('person-has-no-inbox', 'people do not have inboxes: print your answer in the chat.'), 'Not sent: people do not have inboxes; a message can only go to an agent. Your text is kept.');
  assert.equal(said('message-store-full', 'person/example already has 64 MiB of long messages on this machine'), 'Not sent: this machine is holding as many of your long messages as it can; send a shorter message. Your text is kept.');
  // Not 'Fractal hit a problem': the person looks before sending again, since it may have been recorded.
  assert.equal(sendRefusal(refused('internal', 'the store failed')), 'Fractal ran into a problem on its side; look in the conversation before sending it again (internal)');
  // An internal refusal names the fault it reports, so the person can say which.
  assert.equal(said('internal', 'signature-state-corrupt: bootstrap job stopped: Err(database is locked)'),
    'Not sent: Fractal ran into a problem on its side; look in the conversation before sending it again (signature-state-corrupt). Your text is kept.');
  // Only a leading fault counts: a name later in the sentence is not one.
  assert.equal(sendRefusal(refused('internal', 'failed to reach host/example-box: connection reset')), 'Fractal ran into a problem on its side; look in the conversation before sending it again (internal)');
  // The signature refusals the pairing screens already word.
  assert.match(sendRefusal(refused('device-signature-stale', 'the device signed this message more than 15 minutes from this daemon\'s clock')), /this phone's clock is more than 15 minutes off/);
  assert.match(sendRefusal(refused('device-key-not-enrolled', 'the signing key is not enrolled')), /pair the phone again/);
  // A request this Fractal reads differently asks for an update, and keeps Fractal's words for whoever looks into it.
  assert.equal(sendRefusal(refused('validation-failed', 'action parameters require `to`')), 'Fractal did not accept the message as this app sent it; the app or Fractal may need an update (action parameters require `to`)');
  // An older Fractal's 4 KiB refusal keeps its own advice.
  assert.equal(said('validation-failed', 'an inline message cannot exceed 4 KiB; post a document first'), 'Not sent: This Fractal version only takes messages up to 4 KiB. Shorten the message, or update Fractal. Your text is kept.');
  // A code this app does not know yet falls back to the SDK's plain words and names the code.
  assert.equal(sendRefusal(refused('some-new-refusal', 'a reason')), 'a reason (some-new-refusal)');
  assert.match(sendRefusal(new Error('Network request failed')), /the connection to Fractal broke/);
}
