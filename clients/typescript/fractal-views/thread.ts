import { utf8Bytes } from './utf8.ts';

/** Default title for a thread started from an ask. Stop on a character boundary. */
export function threadTitleFromAsk(askTitle: string): string {
  const title = askTitle.replaceAll('\0', '').trim();
  let bytes = 0;
  let end = 0;
  for (const character of title) {
    bytes += utf8Bytes(character);
    if (bytes > 512) break;
    end += character.length;
  }
  return title.slice(0, end).trimEnd() || 'Follow-up';
}
