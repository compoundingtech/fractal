import type { NeedsYouRequest } from './assistant-ui/NeedsYou'

export const needsYouRequests: readonly [NeedsYouRequest, NeedsYouRequest] = [
  { id: 'ask-release-1', agentLabel: 'orbit', prompt: 'Where should the build report be delivered?' },
  { id: 'ask-release-2', agentLabel: 'relay', prompt: 'Which checks should run before publishing the reviewed release report to the team inbox and the release channel?' },
]
export const needsYouRequest = needsYouRequests[0]
