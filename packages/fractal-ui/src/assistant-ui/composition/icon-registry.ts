import { lucideGlyphs, type GlyphNode, type LucideGlyphName } from './icon-glyphs.generated'

/** Token sizes: 12 for chevrons and meta labels, 14 for the leading glyph in its 16 slot, 16 for toolbar actions in a 20 slot. */
export type IconSize = 12 | 14 | 16
export type IconGroup = 'Navigation' | 'Actions' | 'Composer' | 'Work kinds' | 'Status' | 'Objects' | 'Meta'
/** Paint beyond the outline default: a filled shape, dots kept at the dot token, or the spinning running ring. */
export type IconPaint = 'outline' | 'filled' | 'dots' | 'spin'
export type GlyphName = LucideGlyphName | 'kit-ring'
export interface IconEntry {
  readonly group: IconGroup
  readonly meaning: string
  readonly size: IconSize
  readonly glyph: GlyphName
  readonly paint: IconPaint
}

/** Kit-drawn glyphs on the same 24-unit grid. The running ring is a status shape, not a vendored glyph. */
const kitGlyphs = {
  'kit-ring': [['path', { d: 'M12 2.25A9.75 9.75 0 1 1 2.25 12' }], ['path', { d: 'M12 6.75A5.25 5.25 0 1 0 17.25 12' }]],
} as const satisfies Record<string, readonly GlyphNode[]>
export const glyphs: Readonly<Record<GlyphName, readonly GlyphNode[]>> = { ...lucideGlyphs, ...kitGlyphs }

const outline = 'outline'
const entries = {
  'chevron-right': { group: 'Navigation', meaning: 'Collapsed disclosure; next branch', size: 12, glyph: 'chevron-right', paint: outline },
  'chevron-down': { group: 'Navigation', meaning: 'Expanded disclosure; open a picker', size: 12, glyph: 'chevron-down', paint: outline },
  'chevron-up': { group: 'Navigation', meaning: 'Collapse a bottom drawer', size: 12, glyph: 'chevron-up', paint: outline },
  'chevron-left': { group: 'Navigation', meaning: 'Previous branch', size: 12, glyph: 'chevron-left', paint: outline },
  back: { group: 'Navigation', meaning: 'Return to the previous page', size: 14, glyph: 'arrow-left', paint: outline },
  'sidebar-collapse': { group: 'Navigation', meaning: 'Collapse the agent sidebar', size: 16, glyph: 'panel-left-close', paint: outline },
  'sidebar-expand': { group: 'Navigation', meaning: 'Expand the agent sidebar', size: 16, glyph: 'panel-left-open', paint: outline },
  'panel-left': { group: 'Navigation', meaning: 'Toggle the left dock', size: 16, glyph: 'panel-left', paint: outline },
  'panel-right': { group: 'Navigation', meaning: 'Toggle the details pane', size: 16, glyph: 'panel-right', paint: outline },
  'panel-bottom': { group: 'Navigation', meaning: 'Toggle the bottom drawer', size: 16, glyph: 'panel-bottom', paint: outline },
  'split-right': { group: 'Navigation', meaning: 'Split the pane to the right; open in a split', size: 16, glyph: 'columns-2', paint: outline },
  'split-down': { group: 'Navigation', meaning: 'Split the pane downward', size: 16, glyph: 'rows-2', paint: outline },
  open: { group: 'Navigation', meaning: 'Open the conversation from its row', size: 12, glyph: 'arrow-up-right', paint: outline },
  'scroll-end': { group: 'Navigation', meaning: 'Jump to the newest entry', size: 14, glyph: 'arrow-down', paint: outline },
  more: { group: 'Actions', meaning: 'Context or overflow menu', size: 16, glyph: 'ellipsis', paint: 'dots' },
  'view-options': { group: 'Actions', meaning: 'Choose visible sections or columns', size: 16, glyph: 'sliders-horizontal', paint: outline },
  settings: { group: 'Actions', meaning: 'Application settings, nothing else', size: 16, glyph: 'settings', paint: outline },
  'theme-light': { group: 'Actions', meaning: 'Switch to the light theme', size: 16, glyph: 'sun', paint: outline },
  'theme-dark': { group: 'Actions', meaning: 'Switch to the dark theme', size: 16, glyph: 'moon', paint: outline },
  close: { group: 'Actions', meaning: 'Close a tab, pane or dialog', size: 14, glyph: 'x', paint: outline },
  search: { group: 'Actions', meaning: 'Search or filter by text', size: 14, glyph: 'search', paint: outline },
  filter: { group: 'Actions', meaning: 'Filter the list', size: 16, glyph: 'list-filter', paint: outline },
  sort: { group: 'Actions', meaning: 'Sort the list', size: 16, glyph: 'arrow-down-wide-narrow', paint: outline },
  'sort-asc': { group: 'Actions', meaning: 'Column sorted ascending', size: 12, glyph: 'arrow-up', paint: outline },
  'sort-desc': { group: 'Actions', meaning: 'Column sorted descending', size: 12, glyph: 'arrow-down', paint: outline },
  sortable: { group: 'Actions', meaning: 'Column can be sorted', size: 12, glyph: 'chevrons-up-down', paint: outline },
  'new-thread': { group: 'Actions', meaning: 'Start a new thread', size: 16, glyph: 'square-pen', paint: outline },
  add: { group: 'Actions', meaning: 'Add a pane or item', size: 14, glyph: 'plus', paint: outline },
  edit: { group: 'Actions', meaning: 'Rename or edit', size: 14, glyph: 'pencil', paint: outline },
  copy: { group: 'Actions', meaning: 'Copy to the clipboard', size: 14, glyph: 'copy', paint: outline },
  check: { group: 'Actions', meaning: 'Selected, done or copied', size: 12, glyph: 'check', paint: outline },
  'drag-handle': { group: 'Actions', meaning: 'Drag to reorder or move', size: 14, glyph: 'grip-vertical', paint: 'dots' },
  retry: { group: 'Actions', meaning: 'Retry; when limits reset', size: 12, glyph: 'rotate-ccw', paint: outline },
  'wrap-lines': { group: 'Actions', meaning: 'Wrap long diff lines', size: 14, glyph: 'text-wrap', paint: outline },
  whitespace: { group: 'Actions', meaning: 'Hide whitespace-only changes', size: 14, glyph: 'pilcrow', paint: outline },
  send: { group: 'Composer', meaning: 'Send the message', size: 14, glyph: 'arrow-up', paint: outline },
  'stop-run': { group: 'Composer', meaning: 'Stop the running turn', size: 14, glyph: 'square', paint: 'filled' },
  'stop-agent': { group: 'Composer', meaning: 'Stop the agent', size: 16, glyph: 'circle-stop', paint: outline },
  attach: { group: 'Composer', meaning: 'Attach a file or context', size: 14, glyph: 'paperclip', paint: outline },
  'attach-image': { group: 'Composer', meaning: 'Attach an image', size: 14, glyph: 'image-plus', paint: outline },
  recipient: { group: 'Composer', meaning: 'Choose who receives the message', size: 14, glyph: 'at-sign', paint: outline },
  model: { group: 'Composer', meaning: 'Model', size: 14, glyph: 'cpu', paint: outline },
  effort: { group: 'Composer', meaning: 'Reasoning effort', size: 12, glyph: 'signal-medium', paint: outline },
  history: { group: 'Composer', meaning: 'Recall an earlier message; past incarnations', size: 14, glyph: 'rotate-ccw-clock', paint: outline },
  read: { group: 'Work kinds', meaning: 'Read files', size: 14, glyph: 'file-text', paint: outline },
  run: { group: 'Work kinds', meaning: 'Run a command; terminal input', size: 14, glyph: 'square-terminal', paint: outline },
  eval: { group: 'Work kinds', meaning: 'Evaluate code', size: 14, glyph: 'code', paint: outline },
  change: { group: 'Work kinds', meaning: 'Change files', size: 14, glyph: 'file-pen', paint: outline },
  subagents: { group: 'Work kinds', meaning: 'Delegate work to subagents', size: 14, glyph: 'network', paint: outline },
  web: { group: 'Work kinds', meaning: 'Browse the web', size: 14, glyph: 'globe', paint: outline },
  question: { group: 'Work kinds', meaning: 'Ask you to decide', size: 14, glyph: 'circle-question-mark', paint: outline },
  plan: { group: 'Work kinds', meaning: 'Update the plan or todos', size: 14, glyph: 'list-checks', paint: outline },
  wait: { group: 'Work kinds', meaning: 'Wait for results', size: 14, glyph: 'hourglass', paint: outline },
  job: { group: 'Work kinds', meaning: 'Background job or service', size: 14, glyph: 'server-cog', paint: outline },
  'message-out': { group: 'Work kinds', meaning: 'Message another seat', size: 14, glyph: 'send', paint: outline },
  delivery: { group: 'Work kinds', meaning: 'Results delivered to the seat', size: 14, glyph: 'inbox', paint: outline },
  schedule: { group: 'Work kinds', meaning: 'Schedule work', size: 14, glyph: 'calendar-clock', paint: outline },
  skill: { group: 'Work kinds', meaning: 'Use a skill', size: 14, glyph: 'book-open', paint: outline },
  system: { group: 'Work kinds', meaning: 'System event', size: 14, glyph: 'zap', paint: outline },
  thinking: { group: 'Work kinds', meaning: 'Thinking', size: 14, glyph: 'brain', paint: outline },
  compaction: { group: 'Work kinds', meaning: 'Context compaction or summary', size: 14, glyph: 'fold-vertical', paint: outline },
  interjection: { group: 'Work kinds', meaning: 'Message added during a turn', size: 14, glyph: 'message-square-plus', paint: outline },
  irc: { group: 'Work kinds', meaning: 'Peer message', size: 14, glyph: 'message-square', paint: outline },
  hidden: { group: 'Work kinds', meaning: 'Hidden events marker', size: 14, glyph: 'eye-off', paint: outline },
  image: { group: 'Work kinds', meaning: 'Inline image or media', size: 14, glyph: 'image', paint: outline },
  'inline-app': { group: 'Work kinds', meaning: 'Inline app or embedded view', size: 14, glyph: 'app-window', paint: outline },
  running: { group: 'Status', meaning: 'Running', size: 14, glyph: 'kit-ring', paint: 'spin' },
  success: { group: 'Status', meaning: 'Succeeded or resolved', size: 14, glyph: 'circle-check', paint: outline },
  failed: { group: 'Status', meaning: 'Failed', size: 14, glyph: 'circle-x', paint: outline },
  warning: { group: 'Status', meaning: 'Warning or error notice', size: 14, glyph: 'triangle-alert', paint: outline },
  interrupted: { group: 'Status', meaning: 'Run interrupted', size: 14, glyph: 'circle-slash', paint: outline },
  skipped: { group: 'Status', meaning: 'Skipped, not failed', size: 14, glyph: 'circle-minus', paint: outline },
  pending: { group: 'Status', meaning: 'Not started yet', size: 14, glyph: 'circle', paint: outline },
  blocked: { group: 'Status', meaning: 'Blocked', size: 14, glyph: 'circle-alert', paint: outline },
  attention: { group: 'Status', meaning: 'Needs you', size: 14, glyph: 'bell', paint: outline },
  'open-request': { group: 'Status', meaning: 'Open request in the inbox', size: 12, glyph: 'circle-question-mark', paint: outline },
  placeholder: { group: 'Status', meaning: 'Empty-state placeholder', size: 16, glyph: 'circle-dashed', paint: outline },
  folder: { group: 'Objects', meaning: 'Folder, worktree or repository', size: 14, glyph: 'folder', paint: outline },
  'folder-open': { group: 'Objects', meaning: 'Expanded folder', size: 14, glyph: 'folder-open', paint: outline },
  'folder-add': { group: 'Objects', meaning: 'New folder', size: 14, glyph: 'folder-plus', paint: outline },
  branch: { group: 'Objects', meaning: 'Git branch', size: 14, glyph: 'git-branch', paint: outline },
  'pull-request': { group: 'Objects', meaning: 'Pull request', size: 14, glyph: 'git-pull-request', paint: outline },
  diff: { group: 'Objects', meaning: 'Diff or changed files', size: 14, glyph: 'file-diff', paint: outline },
  review: { group: 'Objects', meaning: 'Review', size: 14, glyph: 'eye', paint: outline },
  terminal: { group: 'Objects', meaning: 'Terminal session', size: 14, glyph: 'square-terminal', paint: outline },
  agent: { group: 'Objects', meaning: 'Agent or seat; the agents view', size: 14, glyph: 'bot', paint: outline },
  'standing-seat': { group: 'Objects', meaning: 'Standing seat marker', size: 14, glyph: 'anchor', paint: outline },
  mission: { group: 'Objects', meaning: 'Mission', size: 14, glyph: 'flag', paint: outline },
  thread: { group: 'Objects', meaning: 'Conversation, thread or session', size: 14, glyph: 'message-square', paint: outline },
  link: { group: 'Objects', meaning: 'Link resource', size: 14, glyph: 'link', paint: outline },
  file: { group: 'Objects', meaning: 'Document or file card', size: 14, glyph: 'file', paint: outline },
  resources: { group: 'Objects', meaning: 'Resources view', size: 16, glyph: 'layers', paint: outline },
  inbox: { group: 'Objects', meaning: 'Inbox view', size: 16, glyph: 'inbox', paint: outline },
  monitor: { group: 'Objects', meaning: 'Monitor view', size: 16, glyph: 'activity', paint: outline },
  events: { group: 'Objects', meaning: 'Events view', size: 16, glyph: 'list', paint: outline },
  vista: { group: 'Objects', meaning: 'Vista pages', size: 16, glyph: 'presentation', paint: outline },
  usage: { group: 'Objects', meaning: 'Usage and quota', size: 16, glyph: 'gauge', paint: outline },
  host: { group: 'Meta', meaning: 'Host machine', size: 12, glyph: 'server', paint: outline },
  spend: { group: 'Meta', meaning: 'Spend in dollars', size: 12, glyph: 'coins', paint: outline },
  duration: { group: 'Meta', meaning: 'Run duration', size: 12, glyph: 'timer', paint: outline },
  'last-activity': { group: 'Meta', meaning: 'Last turn or activity', size: 12, glyph: 'clock', paint: outline },
  work: { group: 'Meta', meaning: 'Current work description', size: 12, glyph: 'message-square-text', paint: outline },
  details: { group: 'Meta', meaning: 'Reported details', size: 14, glyph: 'info', paint: outline },
} as const satisfies Record<string, IconEntry>

/** Semantic icon names: call sites name the meaning, never the shape. */
export type IconName = keyof typeof entries
/** One meaning, one glyph (spec §4). */
export const iconRegistry: Readonly<Record<IconName, IconEntry>> = entries
export const iconNames = Object.keys(entries) as readonly IconName[]
export const iconGroups: readonly IconGroup[] = ['Navigation', 'Actions', 'Composer', 'Work kinds', 'Status', 'Objects', 'Meta']
export const iconSizes: readonly IconSize[] = [12, 14, 16]

/** The only meanings allowed to share a glyph (spec §3.5). Everything else is one glyph per meaning. */
export const sharedGlyphAllowance: readonly (readonly [IconName, IconName])[] = [
  ['send', 'sort-asc'], ['scroll-end', 'sort-desc'], ['run', 'terminal'],
  ['question', 'open-request'], ['delivery', 'inbox'], ['irc', 'thread'],
]

/** Groups of meanings that share a glyph without an allowance; empty when the map is injective apart from the allowance. */
export function glyphCollisions(registry: Readonly<Record<string, { readonly glyph: string }>>, allowance: readonly (readonly [string, string])[] = sharedGlyphAllowance): readonly (readonly string[])[] {
  const byGlyph = new Map<string, string[]>()
  for (const [name, { glyph }] of Object.entries(registry)) byGlyph.set(glyph, [...byGlyph.get(glyph) ?? [], name])
  const allowed = new Set(allowance.map(pair => [...pair].sort().join(' ')))
  return [...byGlyph.values()].filter(names => names.length > 1 && !(names.length === 2 && allowed.has([...names].sort().join(' '))))
}
