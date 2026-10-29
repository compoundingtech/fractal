import type { WorkKind } from '@fractal/ui/assistant-ui/work-log'
import type { ToolCallItem } from './model.ts'
import { recordValue } from './blocks.ts'
import { recordedStSendFor } from './recordedStSend.ts'

export type ToolHarness = 'omp' | 'claude' | 'codex'
type ToolTable = Readonly<Record<string, WorkKind | undefined>>
/** Delete harness tables when ST-20 gives every operation a typed view. Exact names only. */
export const harnessToolKinds: Readonly<Record<ToolHarness, ToolTable>> = {
  omp: { read: 'read', grep: 'search', glob: 'search', bash: 'run', run: 'run', eval: 'run', edit: 'edit', write: 'edit', task: 'subagent', wait: 'wait', web_search: 'web', ask: 'decision', ask_question: 'decision', todo: 'todo', hub: 'subagent' },
  claude: { Read: 'read', Grep: 'search', Glob: 'search', Bash: 'run', Edit: 'edit', MultiEdit: 'edit', Write: 'edit', Agent: 'subagent', Task: 'subagent', TaskCreate: 'todo', TaskUpdate: 'todo', TaskList: 'todo', TaskGet: 'todo', SendMessage: 'message', TaskOutput: 'wait', TaskStop: 'subagent', AskUserQuestion: 'decision', WebSearch: 'web', WebFetch: 'web', Monitor: 'schedule', ScheduleWakeup: 'schedule', CronCreate: 'schedule', CronDelete: 'schedule', CronList: 'schedule', Skill: 'skill', TodoWrite: 'todo' },
  codex: { exec_command: 'run', shell: 'run', shell_command: 'run', exec: 'run', apply_patch: 'edit', update_plan: 'todo', spawn_agent: 'subagent', send_input: 'message', resume_agent: 'subagent', close_agent: 'subagent', wait_agent: 'wait', wait: 'wait', request_user_input: 'decision', write_stdin: 'terminal-input', web_search: 'web', web: 'web' },
}
const viewKinds: ToolTable = {
  bash: 'run', command: 'run', edit: 'edit', file_change: 'edit', write: 'edit', read: 'read', search: 'search', web: 'web', todo: 'todo', plan: 'todo', ask: 'decision', question: 'decision', approval: 'decision', task: 'subagent', agent_spawn: 'subagent', hub: 'subagent', agent_message: 'message', agent_message_sent: 'message', wait: 'wait', schedule: 'schedule', terminal_input: 'terminal-input', skill: 'skill', skill_invocation: 'skill', eval: 'run',
}
const toolName = (name: string): string => {
  for (const prefix of ['functions.', 'tools.', 'functions/']) if (name.startsWith(prefix)) return name.slice(prefix.length)
  return name
}
const harnessOf = (item: ToolCallItem): ToolHarness | undefined => {
  const source = item.blocks?.find(block => block.kind === 'tool_call')?.source_type
  return source?.startsWith('claude') ? 'claude' : source?.startsWith('codex') ? 'codex' : source?.startsWith('omp') || source?.startsWith('pi') ? 'omp' : undefined
}
const lookup = (table: ToolTable, key: string): WorkKind | undefined => Object.hasOwn(table, key) ? table[key] : undefined
const fromName = (name: string, harness?: ToolHarness): WorkKind => {
  const key = toolName(name)
  if (harness !== undefined) return lookup(harnessToolKinds[harness], key) ?? 'unknown'
  return lookup(harnessToolKinds.omp, key) ?? lookup(harnessToolKinds.claude, key) ?? lookup(harnessToolKinds.codex, key) ?? 'unknown'
}

export const skippedTool = (item: ToolCallItem): boolean => {
  const metadata = item.result?.metadata
  return metadata?.__synthetic === true && metadata.source === 'interrupt_skipped'
}

/** Native view first; only absent/generic projections need the temporary harness name table. */
export const toolKindFor = (tool: string | ToolCallItem): WorkKind => {
  if (typeof tool === 'string') return fromName(tool)
  const view = tool.view ?? tool.result?.view
  const type = typeof view?.type === 'string' ? view.type : undefined
  const input = recordValue(tool.input)
  const path = typeof view?.path === 'string' ? view.path : typeof input?.path === 'string' ? input.path : undefined
  const operation = type === undefined || type === 'generic' ? fromName(tool.name, harnessOf(tool)) : lookup(viewKinds, type) ?? 'unknown'
  if (operation === 'edit' && (type === 'write' || toolName(tool.name) === 'write' || toolName(tool.name) === 'Write') && path?.startsWith('agent://')) return 'message'
  if (type === 'search' && view?.engine === 'web') return 'web'
  if (operation === 'run' && recordedStSendFor(tool) !== undefined) return 'message'
  return operation
}
