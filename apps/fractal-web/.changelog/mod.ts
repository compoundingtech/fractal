import type { ChangelogDeployment, ChangelogSourceRegistration } from '../src/changelog/contract.ts'
import agentControls from './2026-10-14-agent-controls.ts'
import scopedChangelog from './2026-10-13-scoped-changelog.ts'
import changelogPlacement from './2026-10-12-changelog-placement.ts'
import changelogGlass from './2026-10-12-changelog-glass.ts'
import changelogDismissFocus from './2026-10-11-changelog-dismiss-focus.ts'
import missionsScreen from './2026-10-14-missions-screen.ts'
import semanticTools from './2026-10-11-semantic-tools.ts'
import toolDetails from './2026-10-11-tool-details.ts'
import recordedToolDetails from './2026-10-11-recorded-tool-details.ts'
import waitInspection from './2026-10-11-wait-inspection.ts'
import routineToolErrors from './2026-10-12-routine-tool-errors.ts'
import toolRowVocabulary from './2026-10-12-tool-row-vocabulary.ts'
import taskRequestDetails from './2026-10-12-task-request-details.ts'
import recordedMessageSends from './2026-10-12-recorded-message-sends.ts'
import serviceReadiness from './2026-10-12-service-readiness.ts'
import toolRowIntent from './2026-10-12-tool-row-intent.ts'
import diagnosticsRecovery from './2026-10-12-diagnostics-recovery.ts'
import toolInspectorEscape from './2026-10-12-tool-inspector-escape.ts'
import commandPalette from './2026-10-13-command-palette.ts'
import toolInspectorEvidence from './2026-10-14-tool-inspector-evidence.ts'

/** Public registry: every other file in this directory must be listed here exactly once. */
export const changelogSources: readonly ChangelogSourceRegistration[] = Object.freeze([
  { file: '2026-10-14-agent-controls.ts', source: agentControls, visibility: 'public' },
  { file: '2026-10-13-scoped-changelog.ts', source: scopedChangelog, visibility: 'public' },
  { file: '2026-10-12-changelog-placement.ts', source: changelogPlacement, visibility: 'public' },
  { file: '2026-10-12-changelog-glass.ts', source: changelogGlass, visibility: 'public' },
  { file: '2026-10-11-changelog-dismiss-focus.ts', source: changelogDismissFocus, visibility: 'public' },
  { file: '2026-10-14-missions-screen.ts', source: missionsScreen, visibility: 'public' },
  { file: '2026-10-11-semantic-tools.ts', source: semanticTools, visibility: 'public' },
  { file: '2026-10-11-tool-details.ts', source: toolDetails, visibility: 'public' },
  { file: '2026-10-11-recorded-tool-details.ts', source: recordedToolDetails, visibility: 'public' },
  { file: '2026-10-11-wait-inspection.ts', source: waitInspection, visibility: 'public' },
  { file: '2026-10-12-routine-tool-errors.ts', source: routineToolErrors, visibility: 'public' },
  { file: '2026-10-12-tool-row-vocabulary.ts', source: toolRowVocabulary, visibility: 'public' },
  { file: '2026-10-12-task-request-details.ts', source: taskRequestDetails, visibility: 'public' },
  { file: '2026-10-12-recorded-message-sends.ts', source: recordedMessageSends, visibility: 'public' },
  { file: '2026-10-12-service-readiness.ts', source: serviceReadiness, visibility: 'public' },
  { file: '2026-10-12-tool-row-intent.ts', source: toolRowIntent, visibility: 'public' },
  { file: '2026-10-12-diagnostics-recovery.ts', source: diagnosticsRecovery, visibility: 'public' },
  { file: '2026-10-12-tool-inspector-escape.ts', source: toolInspectorEscape, visibility: 'public' },
  { file: '2026-10-13-command-palette.ts', source: commandPalette, visibility: 'public' },
  { file: '2026-10-14-tool-inspector-evidence.ts', source: toolInspectorEvidence, visibility: 'public' },
])
export const changelogDeployment: ChangelogDeployment = 'main'
