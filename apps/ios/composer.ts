// What the message box does around a send: how long the text may be, what to say when a send is
// refused, and how the text comes back. Plain functions, so they are tested without the app.
import { ClientError, plainError } from '../../clients/typescript/fractal-client';
import { INLINE_MAX_BYTES, signatureRefusal, utf8Bytes } from '@fractal/views';

/** How a send that did not go ended: `unconfirmed` means Fractal may have taken it. */
export type SendFailure = { reason: string; unconfirmed: boolean };

/** The longest message in UTF-8 bytes this Fractal version advertises, or undefined when it says nothing (an older Fractal). */
export function messageLimit(caps: { limits?: { max_message_bytes?: number } } | null | undefined): number | undefined {
  const limit = caps?.limits?.max_message_bytes;
  return typeof limit === 'number' && limit > 0 ? limit : undefined;
}

export type DraftSize = {
  bytes: number;
  /** Past the advertised limit: Send is refused until the text is shorter. */
  over: boolean;
  /** What to show beside the box, or null while the message is short enough not to matter. */
  note: string | null;
};

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  const kib = bytes / 1024;
  return `${kib >= 100 ? Math.round(kib) : Math.round(kib * 10) / 10} KiB`;
}

/**
 * The draft's size against the limit Fractal advertises, counted in UTF-8 bytes as Fractal counts them. With no
 * advertised limit nothing is blocked here (Fractal answers for itself and the text is kept). The count
 * appears once the message is long, so the limit is known before Send is pressed.
 */
export function draftSize(text: string, limit: number | undefined): DraftSize {
  if (limit === undefined) return { bytes: text.length > INLINE_MAX_BYTES ? utf8Bytes(text) : 0, over: false, note: null };
  // Characters are at most four bytes, so a short string cannot be long, nor one of 1/4 the limit past it.
  if (text.length <= INLINE_MAX_BYTES / 4) return { bytes: 0, over: false, note: null };
  const bytes = utf8Bytes(text);
  if (bytes > limit) return { bytes, over: true, note: `Too long to send: ${size(bytes)}, and the limit is ${size(limit)}. Shorten it or split it into two messages. Your text is kept.` };
  if (bytes > INLINE_MAX_BYTES) return { bytes, over: false, note: `${size(bytes)} of ${size(limit)}` };
  return { bytes, over: false, note: null };
}

/**
 * The text back in the box after a send that did not go. Words typed while it was in flight stay:
 * they follow the text that came back. Nothing is ever dropped.
 */
export function restoreDraft(current: string, failed: string): string {
  if (!failed.trim()) return current;
  if (!current.trim()) return failed;
  if (current.includes(failed)) return current;
  return `${failed}\n\n${current}`;
}

// Each refusal Fractal can answer a message with, by its code, in words for the person. Fractal's own
// sentences are written for an agent at a command line, so none is shown as it is.
const REFUSALS: Record<string, string> = {
  'message-too-large': 'the message is too long for Fractal; shorten it or split it into several messages',
  'message-store-full': 'this machine is holding as many of your long messages as it can; send a shorter message',
  'long-message-signature-unsupported': 'this app signs messages in a form that cannot cover one over 8 KiB; update the app, or send a shorter message',
  'empty-message': 'there is nothing to send',
  'person-has-no-inbox': 'people do not have inboxes; a message can only go to an agent',
  'recipient-retired': 'this agent was retired, so it takes no more messages',
  'lifecycle-unavailable': 'Fractal cannot tell right now whether this agent takes messages; try again shortly',
  'not-found': 'Fractal does not know this agent',
  'assistant-send-refused': 'this phone is not allowed to send messages; pair it again with permission to send',
  'rate-limited': 'Fractal is busy with other messages; send it again in a moment',
  'database-busy': 'Fractal is busy; send it again in a moment',
  'database-locked': 'Fractal is busy; send it again in a moment',
  'read-deadline': 'Fractal is busy; send it again in a moment',
  'stale-fence': 'the conversation changed while the message was on its way; send it again',
  'idempotency-conflict': 'this was already sent once in another form, so it was not sent again',
  'idempotency-mismatch': 'this was already sent once in another form, so it was not sent again',
  'legacy-action-receipt-untrusted': 'Fractal could not tell whether this was sent before; send it as a new message',
  'too-many-attachments': 'there are too many images on this message; send some in another message',
  'blob-not-found': 'an image did not reach Fractal; attach it again',
  'invalid-blob-reference': 'an image did not reach Fractal; attach it again',
  'blob-content-mismatch': 'an image is not the kind of file it says it is; attach it again',
  'unpinned-document-reference': 'a document named in the message is not a pinned version (doc/NAME@HASH)',
  'invalid-document-reference': 'a document named in the message is not a pinned version (doc/NAME@HASH)',
  'missing-document': 'a document named in the message is not stored on this machine',
  'reserved-message-tag': 'the message carries a tag only Fractal itself may use',
  // It may come after the message was recorded, so the person looks before sending it again.
  'internal': 'Fractal ran into a problem on its side; look in the conversation before sending it again',
};
// A Fractal version from before long messages: sendReason says what to do about it.
const LEGACY_LIMIT = /cannot exceed 4 ?KiB|post a document first/i;
// The fault an internal refusal names first, before its detail (`signature-state-corrupt: …`).
const FAULT = /^\s*([a-z]+(?:-[a-z]+)+):/;

/**
 * Why Fractal did not take a message, in words for the person, by the refusal's code. A refusal
 * whose cause only Fractal's sentence names (a request a newer Fractal reads differently, a
 * permission) keeps that sentence after the phone's words.
 */
export function sendRefusal(error: unknown): string {
  if (!(error instanceof ClientError)) return plainError(error);
  const { code, message } = error.response;
  if (LEGACY_LIMIT.test(message)) return message;
  // An internal refusal names its fault, so the person can tell someone why (#1951).
  if (code === 'internal') return `${REFUSALS.internal} (${message.match(FAULT)?.[1] ?? code})`;
  const refused = signatureRefusal(code, 'phone') ?? REFUSALS[code];
  if (refused) return refused;
  if (code === 'validation-failed') return `Fractal did not accept the message as this app sent it; the app or Fractal may need an update (${message})`;
  // A missing grant or a recipient this phone may not write to: Fractal's reason says which.
  if (code === 'forbidden') return `this phone is not allowed to send this message (${message})`;
  // A code the phone does not word itself keeps the SDK's words and names the code.
  return `${plainError(error)} (${code})`;
}

/** Why a send was refused, in words for the person; Fractal's own sentence where it already says it. */
export function sendReason(reason: string, unconfirmed: boolean): string {
  if (unconfirmed) return `${reason}. It may have been sent: look in the conversation first. Sending again will not send it twice.`;
  // A Fractal version that predates long messages refuses with advice the phone cannot follow.
  if (/cannot exceed 4 ?KiB|post a document first/i.test(reason)) return 'This Fractal version only takes messages up to 4 KiB. Shorten the message, or update Fractal. Your text is kept.';
  return `${reason}. Your text is kept.`;
}
