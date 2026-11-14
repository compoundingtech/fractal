/**
 * What a code fence's header shows, read from its info string: the file it names (```ts title=src/x.ts, or a path as
 * the info itself, ```src/x.ts) and its language. A diff's counts come from `diffStats` in diff-patch.ts.
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


export const isDiffLanguage = (language: string): boolean => language === 'diff' || language === 'patch'
