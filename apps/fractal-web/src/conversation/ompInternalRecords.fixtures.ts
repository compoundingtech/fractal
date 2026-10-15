/**
 * Native omp records as st3 serves them on the conversation timeline.
 *
 * `toolExecutionStartRecord` is the record captured from the main preview build c21a31af4cb1
 * (thread "Agent attention", 2026-10-15T21:41Z), where it rendered as transcript text. The entry
 * envelope is what st3's `external_sessions.rs` builds for an omp `custom` record with a typed view:
 * fallback text `[omp tool start <tool>]\n{"raw":…}`, a `status` block with view `tool_start`
 * and visibility `internal`, and the `source_record` block (fixtures/clients/transcripts/omp-parity.json).
 */
export const toolExecutionStartRecord = {
  type: 'custom',
  customType: 'tool_execution_start',
  data: {
    toolCallId: 'toolu_01BPSh4b7Vcq35u9GoEhGRhx',
    toolName: 'eval',
    startedAt: '2026-10-15T21:41:34.878Z',
    intent: 'running python',
  },
  id: 'aa72510f',
  parentId: 'c7f073e4',
  timestamp: '2026-10-15T21:41:34.878Z',
} as const

/** The assistant's call that `toolExecutionStartRecord` announces. */
export const toolExecutionStartCall = {
  id: 'timeline-entry/native-call-aa72510f',
  sequence: 40,
  revision: 1,
  final: true,
  timestamp: '2026-10-15T21:41:34.870Z',
  type: 'tool_call',
  role: 'assistant',
  body: { call_id: 'toolu_01BPSh4b7Vcq35u9GoEhGRhx', name: 'eval', arguments: { i: 'running python' } },
} as const

export const toolExecutionStartEntry = {
  id: 'timeline-entry/native-aa72510f',
  sequence: 41,
  revision: 1,
  final: true,
  timestamp: '2026-10-15T21:41:34.878Z',
  type: 'content',
  role: 'system',
  body: {
    media_type: 'text/plain',
    text: `[omp tool start eval]\n${JSON.stringify({ raw: toolExecutionStartRecord })}`,
    blocks: [
      {
        id: 'native-41/0',
        kind: 'status',
        source_type: 'custom',
        visibility: 'internal',
        payload: { raw: toolExecutionStartRecord },
        view: {
          type: 'tool_start',
          call_id: 'toolu_01BPSh4b7Vcq35u9GoEhGRhx',
          tool: 'eval',
          started_at: '2026-10-15T21:41:34.878Z',
        },
      },
      {
        id: 'native-41/source',
        kind: 'source_record',
        source_type: 'omp',
        visibility: 'internal',
        payload: { raw: toolExecutionStartRecord },
      },
    ],
  },
} as const

/** A custom record st3 has no view for: `push_unrecognized` marks it, with no visibility hint. */
export const unknownCustomRecord = {
  type: 'custom',
  customType: 'plan_mode_toggled',
  data: { enabled: true },
  id: 'b81c09d2',
  parentId: 'aa72510f',
  timestamp: '2026-10-15T21:41:35.002Z',
} as const

export const unknownCustomEntry = {
  id: 'timeline-entry/native-b81c09d2',
  sequence: 42,
  revision: 1,
  final: true,
  timestamp: '2026-10-15T21:41:35.002Z',
  type: 'content',
  role: 'system',
  body: {
    media_type: 'text/plain',
    text: `[unrecognized omp entry \`custom\`]\n${JSON.stringify({ raw: unknownCustomRecord })}`,
    blocks: [
      { id: 'native-42/0', kind: 'unknown', source_type: 'custom', payload: { raw: unknownCustomRecord } },
    ],
  },
} as const

/** A custom record with a typed view but no visibility hint keeps rendering as it did. */
export const visibleCustomEntry = {
  ...toolExecutionStartEntry,
  id: 'timeline-entry/native-visible',
  sequence: 43,
  body: {
    ...toolExecutionStartEntry.body,
    blocks: toolExecutionStartEntry.body.blocks.map(({ visibility: _visibility, ...block }) => block),
  },
} as const
