import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Link } from 'react-aria-components'
import { inlineNodesText, parseInlineMarkdown, type InlineNode } from './inline-markdown'
import { phrasingStyles } from './phrasing.stylex'

export { toPlainText } from './inline-markdown'

/** Inline code with the shared token set; the block `Markdown` renders its code spans through this too. */
export function InlineCode({ children }: { readonly children: React.ReactNode }) {
  return <code {...stylex.props(phrasingStyles.code)}>{children}</code>
}

export interface InlineMarkdownProps {
  readonly text: string
  /**
   * `label` (default) draws a link as its label text: legal inside buttons, rows, tabs, radio labels and clamped text.
   * `anchor` draws safe links (relative, http(s), mailto, irc(s), xmpp) as anchors; use it only where no ancestor is interactive.
   */
  readonly links?: 'label' | 'anchor'
}

type LinkMode = NonNullable<InlineMarkdownProps['links']>

function renderNodes(nodes: readonly InlineNode[], links: LinkMode, depth = 0): React.ReactNode {
  // Both element construction and React's own traversal stay bounded. Deeper
  // content remains complete, escaped text rather than disappearing or throwing.
  if (depth >= 32) return inlineNodesText(nodes)
  return nodes.map((node, key) => {
    switch (node._tag) {
      case 'Text': return <React.Fragment key={key}>{node.text}</React.Fragment>
      case 'Code': return <InlineCode key={key}>{node.code}</InlineCode>
      case 'Strong': return <strong key={key} {...stylex.props(phrasingStyles.strong)}>{renderNodes(node.children, links, depth + 1)}</strong>
      case 'Emphasis': return <em key={key} {...stylex.props(phrasingStyles.emphasis)}>{renderNodes(node.children, links, depth + 1)}</em>
      case 'Link': return links === 'anchor' && node.href !== undefined
        ? <Link key={key} href={node.href} target="_blank" rel="noopener noreferrer" {...stylex.props(phrasingStyles.link)}>{renderNodes(node.children, links, depth + 1)}</Link>
        : <React.Fragment key={key}>{renderNodes(node.children, links, depth + 1)}</React.Fragment>
    }
  })
}

/**
 * Single-line agent or user text as phrasing content only (text, `code`, `strong`, `em`, optional `a`), without a wrapper
 * element, so it fits spans, buttons, tabs, `<dt>` and line clamps. HTML is never parsed or injected.
 * Multi-paragraph prose belongs to the block `Markdown`.
 */
export const InlineMarkdown = React.memo(function InlineMarkdown({ text, links = 'label' }: InlineMarkdownProps) {
  const nodes = React.useMemo(() => parseInlineMarkdown(text), [text])
  return <>{renderNodes(nodes, links)}</>
})
