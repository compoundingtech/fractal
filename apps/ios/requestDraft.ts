/** Local, owner-scoped request drafts share byte-bounded draft persistence. */
export type RequestDraft = { choice?: { kind: 'option'; id: string } | { kind: 'custom' }; text: string; notes: Record<string, string> };
type Item = { id: string; revision: string; request?: { answers?: Array<{ id: string; label: string }> } | null };
export const requestDraftKey = (item: Item): string => `request:${item.id}:${item.revision}`;
export const emptyRequestDraft = (): RequestDraft => ({ text: '', notes: {} });
const stringRecord = (value: unknown): value is Record<string, string> => !!value && typeof value === 'object' && !Array.isArray(value) && Object.values(value).every(each => typeof each === 'string');
export const readRequestDraft = (cache: ReadonlyMap<string, string>, item: Item): RequestDraft => {
  const raw = cache.get(requestDraftKey(item));
  if (!raw) return emptyRequestDraft();
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== 'object' || !('text' in value) || typeof value.text !== 'string' || !('notes' in value) || !stringRecord(value.notes)) return emptyRequestDraft();
    let choice: RequestDraft['choice'];
    if ('choice' in value && value.choice && typeof value.choice === 'object' && 'kind' in value.choice) {
      if (value.choice.kind === 'custom') choice = { kind: 'custom' };
      else if (value.choice.kind === 'option' && 'id' in value.choice && typeof value.choice.id === 'string') choice = { kind: 'option', id: value.choice.id };
    }
    return { choice, text: value.text, notes: value.notes };
  } catch { return emptyRequestDraft(); }
};
export const writeRequestDraft = (cache: Map<string, string>, item: Item, draft: RequestDraft): void => {
  const key = requestDraftKey(item);
  cache.delete(key);
  cache.set(key, JSON.stringify(draft));
};
/** A receipt cannot erase edits made after its request snapshot was captured. */
export const clearRequestDraft = (cache: Map<string, string>, item: Item, expectedSerialized?: string): void => {
  const key = requestDraftKey(item);
  if (expectedSerialized === undefined || cache.get(key) === expectedSerialized) cache.delete(key);
};
export const requestOptionNotes = (draft: RequestDraft, selected?: string): Array<{ id: string; text: string }> => Object.entries(draft.notes).filter(([id, text]) => id !== selected && text.trim()).map(([id, text]) => ({ id, text }));
/** Canonical dismissal carries each option note once, without choosing that option. */
export const requestDismissalText = (cache: ReadonlyMap<string, string>, item: Item): string | undefined => {
  const draft = readRequestDraft(cache, item);
  const notes = requestOptionNotes(draft).map(note => `${item.request?.answers?.find(answer => answer.id === note.id)?.label ?? note.id}: ${note.text}`);
  if (draft.text.trim()) notes.push(`Draft answer: ${draft.text}`);
  return notes.length ? notes.join('\n') : undefined;
};

export const readPromptChoices = (cache: ReadonlyMap<string, string>, key: string): number[][] => {
  const raw = cache.get(key);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed) || !parsed.every((choices: unknown): choices is number[] => Array.isArray(choices) && choices.every((choice: unknown) => typeof choice === 'number' && Number.isSafeInteger(choice) && choice >= 0))) return [];
    return parsed;
  } catch { return []; }
};
export const writePromptChoices = (cache: Map<string, string>, key: string, choices: number[][]): void => {
  cache.delete(key); cache.set(key, JSON.stringify(choices));
};
