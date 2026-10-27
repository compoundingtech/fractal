import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { Markdown, ResourceChip, type InlineResource } from './assistant-ui/composition/Markdown'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { interactionVars as interaction, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
const richText = [
  '# [**Selection** and *ordering*](https://example.com/docs/selection_(stable) "Selection guide")',
  '',
  '**Bold with *nested italic***, __alternate bold__, *italic*, _alternate italic_, and escaped \\*literal stars\\*. Inline code keeps `a ** b` literal.',
  '',
  '3. Preserve the selected row.',
  '   - Keep **identity** stable.',
  '     1. Read `src/model.ts`.',
  '     2. Inspect src/selection.ts.',
  '   - Restore focus after sorting.',
  '4. Confirm the rendered order.',
  '',
  '| Resource | State | Count |',
  '| :--- | :---: | ---: |',
  '| `src/model.ts` | **Ready** | 3 |',
  '| [Selection *guide*](https://example.com/guide "Detailed guide") | *Streaming* | 12 |',
  '| Escaped \\| pipe | `a ** b` | 0 |',
].join('\n')
export const languages = [
  ['ts', 'export const selected: string = "row-3"'],
  ['tsx', 'export const Row = ({ title }: { title: string }) => <strong>{title}</strong>'],
  ['js', '// Preserve identity\nconst rows = items.filter(item => item.ready)'],
  ['json', '{ "ready": true, "count": 3 }'],
  ['shell', 'printf "%s\\n" "$HOME"\nfor file in src/*.ts; do echo "$file"; done'],
  ['diff', '--- a/src/model.ts\n+++ b/src/model.ts\n@@ -1 +1 @@\n-old\n+new'],
  ['rust', 'fn main() { let selected: Option<usize> = Some(3); println!("{:?}", selected); }'],
  ['nix', '{ pkgs, ... }: { packages = [ pkgs.nodejs ]; enabled = true; }'],
  ['python', 'def selected(rows):\n    return [row for row in rows if row["ready"]]'],
  ['yaml', 'selected: row-3\nready: true\nresources:\n  - src/model.ts'],
  ['markdown', '# Selection\n**Stable** identity and [a guide](https://example.com).'],
  ['css', '.selected { color: var(--foreground); display: grid; }'],
  ['unknown-language', '<script>alert("This stays literal code")</script>'],
  ['', 'A fence without a language stays plain text.'],
] as const
const languageText = languages.map(([language, code]) => `\`\`\`${language}\n${code}\n\`\`\``).join('\n\n')
export const streamChunks = [
  '# Streaming result\n\n```typescript\nconst selected = "row-3";',
  '\nconst description = "A long row title that wraps only after the reader enables wrapping and stays wrapped as new tokens arrive in the same code fence.";',
  '\nexport const state = { selected, description };',
  '\n```\n\n**Complete.**\n\n```typescript\nexport const complete = true;\n```',
] as const
export const inlineStreamStages = [
  '**Stable',
  '**Stable** and *ordered',
  '**Stable** and *ordered* in `src/model.ts',
  '**Stable** and *ordered* in `src/model.ts` — [**Selection** guide](https://example.com/gui',
  '**Stable** and *ordered* in `src/model.ts` — [**Selection** guide](https://example.com/guide "Selection guide")',
] as const
const resources: readonly InlineResource[] = [{ path: 'src/model.ts', added: 3, removed: 1, preview: 'export const selected = "row-3"' }]
const customReference: InlineResource = { path: 'src/selection.ts', added: 2, removed: 0 }
const renderInlineReference = (path: string) => path === customReference.path ? <span data-testid="custom-inline-reference"><ResourceChip resource={customReference} /></span> : undefined

type Scenario = 'rich-text' | 'languages' | 'streaming' | 'inline-streaming' | 'all'
export function MarkdownCanvas({ scenario = 'rich-text', scheme = 'dark' }: { scenario?: Scenario; scheme?: 'dark' | 'light' }): React.ReactNode {
  const [stage, setStage] = React.useState(0)
  const [opened, setOpened] = React.useState<string | undefined>()
  const openResource = React.useCallback((path?: string) => setOpened(path), [])
  const streaming = scenario === 'streaming' || scenario === 'inline-streaming'
  const stages = scenario === 'inline-streaming' ? inlineStreamStages : streamChunks
  const text = scenario === 'languages' ? languageText : scenario === 'streaming' ? streamChunks.slice(0, stage + 1).join('') : scenario === 'inline-streaming' ? inlineStreamStages[stage]! : richText
  return <main data-testid="markdown-story" data-stream-stage={stage} {...stylex.props(styles.canvas, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <div {...stylex.props(styles.lane)}>
      {streaming ? <div {...stylex.props(styles.toolbar)}><Button data-testid="append-stream" onPress={() => setStage(value => value + 1)} isDisabled={stage === stages.length - 1} {...stylex.props(styles.button)}>Append stream chunk</Button><span>{stage === stages.length - 1 ? 'Complete' : 'Streaming'}</span></div> : null}
      <Markdown text={text} streaming={streaming && stage < stages.length - 1} resources={resources} onOpenResource={openResource} renderInlineReference={renderInlineReference} />
      {opened ? <output data-testid="opened-resource">Opened {opened}</output> : null}
      {scenario === 'all' ? <><h2 {...stylex.props(styles.heading)}>Code languages</h2><Markdown text={languageText} /><h2 {...stylex.props(styles.heading)}>Open streaming fence</h2><Markdown text={streamChunks[0]} streaming /><h2 {...stylex.props(styles.heading)}>Partial inline streams</h2>{inlineStreamStages.map(value => <Markdown key={value} text={value} streaming resources={resources} />)}</> : null}
    </div>
  </ThemePortal></main>
}
export const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  lane: { width: '100%', maxWidth: g.lane, minWidth: 0, marginInline: 'auto' },
  inputNarrow: { maxWidth: g.polishNarrow },
  toolbar: { display: 'flex', alignItems: 'center', gap: s.md, marginBlockEnd: s.lg },
  button: { minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing }, ':disabled': { color: ink.fgMuted, cursor: 'default' } },
  heading: { marginBlock: s.lg, fontSize: t.headingSize, lineHeight: t.headingLeading },
})
