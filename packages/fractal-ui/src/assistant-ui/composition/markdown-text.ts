// Structurally the same types as clients/typescript/fractal-client/markdown-text.ts; the kit does
// not depend on the client package. markdownMisuse.tsx proves the two unify.
export type MarkdownBlock = { readonly '~fractal/markdown': 'block' }
export type MarkdownInline = { readonly '~fractal/markdown': 'inline' }
export const markdownSource = (markdown: MarkdownBlock | MarkdownInline): string => markdown as unknown as string
export const markdownBlock = (source: string): MarkdownBlock => source as unknown as MarkdownBlock
export const markdownInline = (source: string): MarkdownInline => source as unknown as MarkdownInline
export const markdownCodeFence = (code: string, language = ''): MarkdownBlock => {
  const longest = (code.match(/`+/g) ?? []).reduce((most, run) => Math.max(most, run.length), 2)
  const fence = '`'.repeat(longest + 1)
  // markdown-template-reviewed: the fence is longer than any backtick run in the code
  return markdownBlock(`${fence}${language}\n${code}\n${fence}`)
}
