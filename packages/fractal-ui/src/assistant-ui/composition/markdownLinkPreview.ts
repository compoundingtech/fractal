import * as React from 'react'

/**
 * A host's hover preview for a Markdown link (a plugin card, say). Returning a node replaces the link with it;
 * the kit never fetches or knows what the href names. The link it receives carries no native `title`, which
 * the preview would otherwise fight. Its own module, so a host can provide it without loading the renderer.
 */
export type MarkdownLinkPreview = (href: string, link: React.ReactElement) => React.ReactNode | undefined
export const MarkdownLinkPreviewContext = React.createContext<MarkdownLinkPreview | undefined>(undefined)
