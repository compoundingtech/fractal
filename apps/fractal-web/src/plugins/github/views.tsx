/**
 * Everything the GitHub plugin renders, behind one lazy boundary: the host's registry and Markdown links
 * load this same-origin chunk (CSP `script-src 'self'`) the first time a GitHub thing is shown.
 */
export { GithubCard, GithubHoverLink } from './card/GithubCard.tsx'
export { IssueDetail, PullRequestDetail, RunDetail } from './detail/index.ts'
export { GithubSurface } from './surface.tsx'
