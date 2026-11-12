// Text whose format is part of its type. At runtime each is the plain wire string, so decoding
// costs nothing and a value reached through `any` still prints as readable text. At compile time
// neither is a `string`: passing one to a JSX child, a title, or a string helper is an error, and a
// `string` cannot reach a Markdown renderer without naming how it became Markdown.
//
// The property key is a string, not a unique symbol, so packages/fractal-ui can declare the same
// type without depending on this package.

/** Intentionally opaque, not a string: all consumers are private in-repo packages checked by the gate.
 * Use markdownSource to cross the source-text boundary; nullable fields keep this same contract. */
export type MarkdownBlock = { readonly '~fractal/markdown': 'block' };
/** Intentionally opaque like MarkdownBlock; use markdownSource when a string is required. */
export type MarkdownInline = { readonly '~fractal/markdown': 'inline' };
export type MarkdownText = MarkdownBlock | MarkdownInline;

/** The Markdown source, for code that genuinely edits or copies source text. */
export const markdownSource = (markdown: MarkdownText): string => markdown as unknown as string;

/** The source of an optional field, `undefined` kept, for code that reads the words as a string. */
export const optionalSource = (markdown: MarkdownText | undefined): string | undefined =>
  markdown === undefined ? undefined : markdownSource(markdown);

/** One line of inline Markdown is also a valid block. The reverse needs `inlineFromBlock`. */
export const blockFromInline = (inline: MarkdownInline): MarkdownBlock => inline as unknown as MarkdownBlock;

/** Text this client wrote as Markdown, or that a documented contract says is Markdown. */
export const markdownBlock = (source: string): MarkdownBlock => source as unknown as MarkdownBlock;
export const markdownInline = (source: string): MarkdownInline => source as unknown as MarkdownInline;

const ESCAPED = /[\\`*_{}[\]()#+\-.!|<>~&]/g;

/** Literal text as Markdown that renders to exactly those characters, line breaks kept. */
export const markdownFromPlain = (plain: string): MarkdownBlock =>
  markdownBlock(plain.replace(ESCAPED, '\\$&').replace(/\r?\n/g, '  \n'));

/** Literal text as one line of inline Markdown; line breaks become spaces. */
export const inlineFromPlain = (plain: string): MarkdownInline =>
  markdownInline(plain.replace(ESCAPED, '\\$&').replace(/\s*\r?\n\s*/g, ' '));

/** Verbatim text in a code fence longer than any backtick run inside it. */
export const markdownCodeFence = (code: string, language = ''): MarkdownBlock => {
  const longest = (code.match(/`+/g) ?? []).reduce((most, run) => Math.max(most, run.length), 2);
  const fence = '`'.repeat(longest + 1);
  // markdown-template-reviewed: the fence is longer than any backtick run in the code
  return markdownBlock(`${fence}${language}\n${code}\n${fence}`);
};

/** Apply a source-to-source rewrite, such as `spaced`, keeping the format. */
export const mapMarkdown = <M extends MarkdownText>(markdown: M, rewrite: (source: string) => string): M =>
  rewrite(markdown as unknown as string) as unknown as M;

/** Cut to at most `max` code points without leaving a code fence open. */
export const excerptMarkdown = (markdown: MarkdownBlock, max: number): MarkdownBlock => {
  const points = Array.from(markdownSource(markdown));
  if (points.length <= max) return markdown;
  const cut = points.slice(0, max).join('');
  const fences = cut.match(/^\s*(`{3,}|~{3,})/gm) ?? [];
  const open = fences.length % 2 === 1;
  const fence = fences[fences.length - 1]?.trim();
  return markdownBlock(open && fence !== undefined ? `${cut}…\n${fence}` : `${cut}…`);
};
