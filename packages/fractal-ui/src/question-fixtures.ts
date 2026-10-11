import type { QuestionRequestView, QuestionState, QuestionView } from './assistant-ui/QuestionForm'

export const questionKinds: readonly QuestionView[] = [
  { id: 'single', header: 'Delivery', kind: 'single', prompt: 'Where should the build report be delivered?', options: [{ id: 'thread', label: 'This thread', recommended: true, description: 'Keep the report beside the work.', preview: { _tag: 'markdown', text: '**Build report**\n\nTests passed; the deployment is ready for review.' } }, { id: 'inbox', label: 'Team inbox', preview: { _tag: 'url', url: 'https://example.org/team/inbox' } }], allowOther: true, allowNote: true },
  { id: 'multi', header: 'Checks', kind: 'multi', prompt: 'Which checks should run before publishing?', options: [{ id: 'unit', label: 'Unit tests' }, { id: 'browser', label: 'Browser checks' }], allowOther: true, allowNote: true, required: false },
  { id: 'text', header: 'Release', kind: 'text', prompt: 'What should the release note say?', options: [], allowOther: false, allowNote: true, placeholder: 'Describe the change for readers', allowAttachments: true },
  { id: 'confirm', header: 'Publish', kind: 'confirm', prompt: 'Publish the reviewed report?', options: [{ id: 'yes', label: 'Publish' }, { id: 'no', label: 'Keep as draft', tone: 'danger' }], allowOther: false, allowNote: false },
  { id: 'permission-command', header: 'Command', kind: 'permission', prompt: 'Allow this command to run?', subject: { _tag: 'command', command: 'pnpm -C packages/fractal-ui run check', cwd: '/workspace/smalltalk' }, options: [{ id: 'once', label: 'Allow once', scope: 'this command' }, { id: 'session', label: 'Allow for this session', scope: 'this session' }, { id: 'deny', label: 'Deny', tone: 'danger' }], allowOther: true, allowNote: true },
  { id: 'permission-diff', header: 'Patch', kind: 'permission', prompt: 'Apply this proposed change?', subject: { _tag: 'diff', path: 'src/report.ts', lines: ['@@ -1 +1 @@', '-const delivery = "inbox"', '+const delivery = "thread"'] }, options: [{ id: 'apply', label: 'Apply patch' }, { id: 'deny', label: 'Do not apply', tone: 'danger' }], allowOther: true, allowNote: true },
  { id: 'plan', header: 'Plan', kind: 'plan', prompt: 'Review the proposed release plan.', subject: { _tag: 'markdown', text: '## Release plan\n\n1. Run checks.\n2. Publish the report.\n3. Ask for review.' }, options: [{ id: 'approve', label: 'Approve plan' }, { id: 'revise', label: 'Revise first' }], allowOther: true, allowNote: true },
  { id: 'mcp', header: 'Service', kind: 'text', prompt: 'The release service needs a project name.', options: [], allowOther: false, allowNote: false },
]
export const questionStates: readonly QuestionState[] = [
  { _tag: 'pending' },
  { _tag: 'pending', expiresAt: '15:42 UTC' },
  { _tag: 'submitting' },
  { _tag: 'failed', error: 'The answer service disconnected' },
  { _tag: 'answered', answers: { single: { selected: ['thread'], note: 'Keep it discoverable' }, multi: { selected: [] } }, by: 'Reviewer', where: 'web', at: '15:41 UTC' },
  { _tag: 'answered-elsewhere', answers: { single: { selected: ['inbox'] } }, where: 'terminal', at: '15:41 UTC', draftDiscarded: true },
  { _tag: 'expired', at: '15:42 UTC', autoAnswer: { single: { selected: ['thread'], autoSelected: true } } },
  { _tag: 'expired', at: '15:42 UTC' },
  { _tag: 'cancelled', reason: 'Skipped · turn ended', at: '15:41 UTC' },
  { _tag: 'chat-redirected', message: 'Discuss the release before choosing.', at: '15:41 UTC' },
  { _tag: 'unrecorded', reason: 'Answer details were not recorded.' },
]
export const questionRequest: QuestionRequestView = { id: 'ask-release-1', source: 'omp · ask', agentLabel: 'orbit', askedAt: '15:40 UTC', questions: [questionKinds[0]!], state: { _tag: 'pending' }, answerable: false, allowChatRedirect: false }
