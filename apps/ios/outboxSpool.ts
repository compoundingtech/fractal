import { createOutboxRecord } from '../../clients/typescript/fractal-client/outbox.ts';
import type { OutboxIntent, OutboxPreparingRecord, OutboxRepository } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { decodeBase64, MAX_BYTES, MAX_IMAGES } from './images.ts';
import type { Picked } from './images.ts';
import { messageIntent } from './outboxMessage.ts';
import type { OutboxMessageMetadata } from './outboxMessage.ts';

export type OutboxStorageProtection = {
  databaseDirectory: string;
  spoolDirectory: string;
  databaseExcludedFromBackup: boolean;
  spoolExcludedFromBackup: boolean;
};
export interface OutboxFilesBridge {
  prepareStorage(): Promise<OutboxStorageProtection>;
  writeAttachment(base64: string, contentHash: string): Promise<{ reference: string; contentHash: string; bytes: number }>;
  readAttachment(reference: string): Promise<string>;
  deleteAttachment(reference: string): Promise<void>;
}
export type OutboxHash = (bytes: Uint8Array) => Promise<Uint8Array>;

/** Refuse protected acceptance; never route to legacy sending or clear the composer. */
export class OutboxStorageProtectionError extends Error {
  readonly mode = 'unprotected-storage';
  readonly keepDraft = true;
  constructor(cause?: unknown) {
    super('Crash-safe storage is unavailable. Your draft is kept; no message was accepted.', { cause });
    this.name = 'OutboxStorageProtectionError';
  }
}

/** Transient access/open failure is not evidence that a stored row is damaged. */
export class OutboxStorageUnavailableError extends Error {
  readonly mode = 'storage-unavailable';
  readonly keepDraft = true;
  readonly retryLater = true;
  constructor(cause?: unknown) {
    super('Storage unavailable, retry later. Your draft and recovery records are kept.', { cause });
    this.name = 'OutboxStorageUnavailableError';
  }
}

export const outboxStorageOperation = async <TResult>(operation: () => Promise<TResult>): Promise<TResult> => {
  try { return await operation(); }
  catch (error) {
    if (error instanceof OutboxStorageUnavailableError || error instanceof OutboxStorageProtectionError) throw error;
    throw new OutboxStorageUnavailableError(error);
  }
};

export const requireProtectedOutboxStorage = async (
  bridge: OutboxFilesBridge | null | undefined, databaseDirectory?: string,
): Promise<OutboxStorageProtection> => {
  if (!bridge) throw new OutboxStorageProtectionError();
  try {
    const proof = await outboxStorageOperation(() => bridge.prepareStorage());
    if (proof.databaseExcludedFromBackup !== true || proof.spoolExcludedFromBackup !== true
      || !proof.databaseDirectory || (databaseDirectory !== undefined && proof.databaseDirectory !== databaseDirectory)
      || proof.spoolDirectory !== `${proof.databaseDirectory}/spool`) {
      throw new OutboxStorageProtectionError();
    }
    return Object.freeze({ ...proof });
  } catch (error) {
    if (error instanceof OutboxStorageProtectionError || error instanceof OutboxStorageUnavailableError) throw error;
    throw new OutboxStorageProtectionError(error);
  }
};


/** Run the native/SQLite opener only after a successful exclusion read-back. */
export const openProtectedOutboxStorage = async <TResult>(
  bridge: OutboxFilesBridge | null | undefined,
  open: (protection: OutboxStorageProtection) => Promise<TResult>,
): Promise<TResult> => {
  try { return await open(await requireProtectedOutboxStorage(bridge)); }
  catch (error) {
    if (error instanceof OutboxStorageProtectionError || error instanceof OutboxStorageUnavailableError) throw error;
    throw new OutboxStorageUnavailableError(error);
  }
};
const contentHash = async (bytes: Uint8Array, sha256: OutboxHash): Promise<string> => {
  const digest = await sha256(bytes);
  if (digest.length !== 32) throw new OutboxStorageProtectionError();
  return `sha256:${Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('')}`;
};
const referenceFor = (hash: string): string => `spool/${hash.slice(7)}`;

/** No uploads/network. Every immutable file is acknowledged before the sole intent commit. */
export const acceptProtectedOutboxIntent = async (
  repository: OutboxRepository<PreparedPacket>, bridge: OutboxFilesBridge | null | undefined,
  databaseDirectory: string, sha256: OutboxHash,
  id: string, text: string, images: readonly Picked[] = [], metadata?: OutboxMessageMetadata,
): Promise<OutboxPreparingRecord> => {
  // Capture before await: later composer edits cannot change this acceptance snapshot.
  const captured = images.map(image => ({ ...image }));
  const capturedMetadata: OutboxMessageMetadata | undefined = metadata ? JSON.parse(JSON.stringify(metadata)) : undefined;
  try {
    await requireProtectedOutboxStorage(bridge, databaseDirectory);
    if (!bridge || captured.length > MAX_IMAGES) throw new OutboxStorageProtectionError();
    const attachments: OutboxIntent['attachments'][number][] = [];
    for (const image of captured) {
      const bytes = decodeBase64(image.base64);
      if (bytes.length > MAX_BYTES || bytes.length !== image.bytes) throw new OutboxStorageProtectionError();
      const hash = await contentHash(bytes, sha256);
      const stored = await outboxStorageOperation(() => bridge.writeAttachment(image.base64, hash));
      if (stored.contentHash !== hash || stored.reference !== referenceFor(hash) || stored.bytes !== bytes.length) {
        throw new OutboxStorageProtectionError();
      }
      attachments.push(Object.freeze({ contentHash: hash, reference: stored.reference, mediaType: image.mediaType,
        ...(image.name ? { name: image.name } : {}) }));
    }
    const intent: OutboxIntent = Object.freeze({ text, attachments: Object.freeze(attachments), ...capturedMetadata });
    if (capturedMetadata && !messageIntent(intent)) throw new OutboxStorageProtectionError();
    const record = createOutboxRecord(id, intent);
    if (!await outboxStorageOperation(() => repository.commit(record))) throw new OutboxStorageProtectionError();
    return record;
  } catch (error) {
    if (error instanceof OutboxStorageProtectionError || error instanceof OutboxStorageUnavailableError) throw error;
    throw new OutboxStorageProtectionError(error);
  }
};

/** Revalidate content on read; an immutable reference never becomes an arbitrary file path. */
export const readOutboxAttachment = async (
  bridge: OutboxFilesBridge, databaseDirectory: string, sha256: OutboxHash, reference: string,
): Promise<Uint8Array> => {
  if (!/^spool\/[a-f0-9]{64}$/.test(reference)) throw new OutboxStorageProtectionError();
  await requireProtectedOutboxStorage(bridge, databaseDirectory);
  const bytes = decodeBase64(await outboxStorageOperation(() => bridge.readAttachment(reference)));
  if (bytes.length > MAX_BYTES || reference !== referenceFor(await contentHash(bytes, sha256))) {
    throw new OutboxStorageProtectionError();
  }
  return bytes;
};
