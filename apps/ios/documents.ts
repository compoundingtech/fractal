import { ClientError, type Capabilities, type DocumentReference, type St3Client } from '../../clients/typescript/st3-client';
import {
  documentUnavailable, documentView, failDocumentList, openOnTrail, pushDocumentPage, unavailableDocument,
  type DocumentList, type DocumentUnavailable, type DocumentView,
} from '@smalltalk/st3-views';

// Documents on the phone read through the client's one path to a document (document.get,
// documents.list and document-references.get) and the shared view in @smalltalk/st3-views, as
// stui and web do. Read-only: nothing here edits or publishes, and the text is shown as written.

/** The client reads a document view needs; null while the phone is offline. */
export type DocumentReader = Pick<St3Client, 'documentGet' | 'documentsList' | 'documentReferencesGet'>;

/** Whether this st serves documents to the phone: true, false (an older st, or not granted), or null before it has said. */
export function documentsServed(caps: Pick<Capabilities, 'capabilities'> | null | undefined): boolean | null {
  if (!caps) return null;
  return caps.capabilities.some(capability => capability.id === 'documents' && capability.version >= 1 && capability.state === 'granted');
}

export const DOCUMENTS_NOT_SERVED = 'The st this phone is paired with does not serve documents yet; it needs a newer st.';

/** Why a read failed: a 404 is not found or not synced yet; anything else, or no answer, is the remote. */
export function whyUnreadable(error: unknown): DocumentUnavailable {
  return error instanceof ClientError ? documentUnavailable(error.status, error.response.code) : 'remote-unavailable';
}

/** Opens `reference` (`doc/NAME` for the latest version, or `doc/NAME@HASH`). It always says why it could not. */
export async function openDocument(reader: DocumentReader | null, reference: string, signal?: AbortSignal): Promise<DocumentView> {
  if (!reader) return unavailableDocument(reference, 'remote-unavailable');
  try {
    return documentView(reference, (await reader.documentGet(reference, { signal })).value);
  } catch (error) {
    return unavailableDocument(reference, whyUnreadable(error));
  }
}

export const DOCUMENT_PAGE = 50;

/** The list with its next page added, or marked failed with the reason. */
export async function nextDocumentPage(reader: DocumentReader | null, list: DocumentList, signal?: AbortSignal): Promise<DocumentList> {
  if (!reader) return failDocumentList(list, 'remote-unavailable');
  try {
    const page = await reader.documentsList(
      { ...(list.prefix ? { prefix: list.prefix } : {}), ...(list.nextCursor ? { cursor: list.nextCursor } : {}), limit: DOCUMENT_PAGE },
      { signal },
    );
    return pushDocumentPage(list, page.value);
  } catch (error) {
    return failDocumentList(list, whyUnreadable(error));
  }
}

/** The documents a message, mission, run or step names, each once in the order st gives them. */
export async function referencedDocuments(reader: DocumentReader | null, subjects: readonly string[], signal?: AbortSignal): Promise<{ references: DocumentReference[]; failed: DocumentUnavailable | null }> {
  if (!reader) return { references: [], failed: 'remote-unavailable' };
  const references: DocumentReference[] = [];
  let failed: DocumentUnavailable | null = null;
  for (const subject of subjects) {
    try {
      for (const reference of (await reader.documentReferencesGet(subject, { signal })).value.references) {
        if (!references.some(known => known.reference === reference.reference)) references.push(reference);
      }
    } catch (error) {
      failed = whyUnreadable(error);
    }
  }
  return { references, failed };
}

/** Following a link from the open document: back to it when it is already on the trail, else one more. */
export type Follow = { kind: 'stay' } | { kind: 'back'; count: number } | { kind: 'push'; trail: string[] };

export function followDocument(trail: readonly string[], reference: string): Follow {
  const next = openOnTrail(trail, reference);
  if (next.length > trail.length) return { kind: 'push', trail: next };
  const count = trail.length - next.length;
  return count ? { kind: 'back', count } : { kind: 'stay' };
}

/** When a version was bound to its name, as a date and time on this phone, or null when st does not say. */
export function changedAt(unixMs: number | null): string | null {
  if (unixMs === null) return null;
  const at = new Date(unixMs);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** The line under a document's name: its short version, whether it is the latest, and when it changed. */
export function versionLine(view: { shortVersion: string; latest: boolean | null; changedAtUnixMs: number | null }): string {
  const parts = [`version ${view.shortVersion}`];
  if (view.latest === true) parts.push('latest');
  if (view.latest === false) parts.push('an older version');
  const changed = changedAt(view.changedAtUnixMs);
  if (changed) parts.push(`changed ${changed}`);
  return parts.join(' · ');
}
