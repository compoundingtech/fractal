/**
 * The host's one door to plugins: what they contribute to every surface below the app root. Today GitHub's
 * hover card on Markdown links and its data reader; the tab kinds join `workspace/kinds.ts` from
 * `./github/index.ts` once the kind registry is on this branch.
 */
import * as React from 'react'

// The context's own module: the shell graph must not pull in the Markdown renderer (LiveAgentWorkspace.graph test).
import { MarkdownLinkPreviewContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdownLinkPreview.ts'

import { gatewayGithubReader, githubLinkPreview, GithubReaderProvider, sharedGithubReader } from './github/index.ts'

export function PluginProviders({ baseUrl, fetchImpl, children }: { readonly baseUrl: string; readonly fetchImpl: typeof fetch; readonly children: React.ReactNode }) {
  const github = React.useMemo(() => sharedGithubReader(gatewayGithubReader({ baseUrl, fetchImpl })), [baseUrl, fetchImpl])
  return <GithubReaderProvider value={github}><MarkdownLinkPreviewContext.Provider value={githubLinkPreview}>{children}</MarkdownLinkPreviewContext.Provider></GithubReaderProvider>
}
