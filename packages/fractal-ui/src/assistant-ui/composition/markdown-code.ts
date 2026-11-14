/**
 * What a code fence's header shows, read from its info string: the file it names (```ts title=src/x.ts, or a path as
 * the info itself, ```src/x.ts) and its language, and for a diff the lines it adds and removes.
 */

const TITLE = /(?:^|\s)(?:title|file|path)=(?:"([^"]+)"|'([^']+)'|(\S+))/

/** The language and, when the fence names one, the file path. */
export function codeFenceLabel(language: string, meta: string | null | undefined): { readonly language: string; readonly path?: string } {
  const titled = meta === null || meta === undefined ? undefined : TITLE.exec(meta)
  const path = titled === undefined || titled === null ? undefined : titled[1] ?? titled[2] ?? titled[3]
  if (path !== undefined) return { language: language || extension(path) || 'text', path }
  // A path as the info string: it has a directory or an extension, and no language is spelled that way.
  if (/[/\\]/.test(language) || /^[\w.-]+\.\w{1,8}$/.test(language)) return { language: extension(language) || 'text', path: language }
  return { language: language || 'text' }
}

const extension = (path: string): string | undefined => /\.(\w{1,8})$/.exec(path)?.[1]?.toLowerCase()

/** Lines a unified diff adds and removes, as GitHub counts them: file headers and hunk markers excluded. */
export function diffStats(patch: string): { readonly added: number; readonly removed: number } {
  let added = 0
  let removed = 0
  let oldRemaining = 0
  let newRemaining = 0
  for (const line of patch.split('\n')) {
    const hunk = /^@@ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@/.exec(line)
    if (hunk !== null) { oldRemaining = Number(hunk[1] ?? 1); newRemaining = Number(hunk[2] ?? 1); continue }
    const inHunk = oldRemaining > 0 || newRemaining > 0
    if (!inHunk && (line.startsWith('+++') || line.startsWith('---'))) continue
    if (line.startsWith('+')) added++
    else if (line.startsWith('-')) removed++
    if (inHunk) {
      if (line.startsWith('+')) newRemaining--
      else if (line.startsWith('-')) oldRemaining--
      else if (line.startsWith(' ')) { oldRemaining--; newRemaining-- }
    }
  }
  return { added, removed }
}

export const isDiffLanguage = (language: string): boolean => language === 'diff' || language === 'patch'
