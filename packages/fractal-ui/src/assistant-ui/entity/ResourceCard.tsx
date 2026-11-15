/**
 * Resource and folder content in the shared card grammar. A resource is anything an agent touches or produces: a file,
 * a diff, a terminal, a document, or a plugin's item (a GitHub pull request, issue, comment or run). The adapter is
 * data-only, so a plugin maps its read into these fields and the kit never fetches or imports plugin code; a plugin's
 * own rich body (labels, a person disc) goes in `children`.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Icon, type IconName } from '../composition/Icons'
import { DiffView } from '../composition/DiffView'
import { radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { ResourceData, ResourceFile } from '../resources/resource-model.ts'
import { EntityCard, EntityCardHeader, EntityCardList, EntityCardListItem, EntityCardMeta, EntityCardMetaLine, EntityCardNote, EntityCardSection, EntityCardText, EntityStatusMark } from './EntityCard'
import type { EntityDescriptor, EntityStatus } from './entity.ts'

export interface EntityMetaLine {
  readonly icon: IconName
  readonly label: string
  readonly value: React.ReactNode
}

export interface ResourceEntityCardProps {
  /** The resource uri, its identity everywhere. */
  readonly id: string
  readonly title: string
  /** "File", "Diff", "Terminal", "Pull request", "Issue", "Run", "Document". */
  readonly kind: string
  readonly icon?: IconName
  readonly status?: EntityStatus
  /** Where it lives, then its handle: e.g. ["compoundingtech/fractal", "#5741"]. */
  readonly summary?: readonly React.ReactNode[]
  /** A short excerpt or the resource's detail line. */
  readonly text?: React.ReactNode
  readonly meta?: readonly EntityMetaLine[]
  /** Observed lines (a terminal tail, the first lines of a diff), shown monospaced and clipped to six. */
  readonly preview?: readonly string[]
  /** `diff` tints added and removed lines. */
  readonly previewKind?: 'text' | 'diff'
  readonly note?: React.ReactNode
  readonly children?: React.ReactNode
}

export const resourceDescriptor = ({ id, title, status }: Pick<ResourceEntityCardProps, 'id' | 'title' | 'status'>): EntityDescriptor =>
  ({ kind: 'resource', id, title, ...(status === undefined ? {} : { status }) })

export function ResourceEntityCard({ id, title, kind, icon, status, summary = [], text, meta = [], preview, previewKind = 'text', note, children }: ResourceEntityCardProps) {
  return <EntityCard entity={resourceDescriptor({ id, title, status })} testId="resource-hover-card">
    <EntityCardHeader title={title} kind={kind}
      glyph={status !== undefined ? <EntityStatusMark status={status} /> : icon !== undefined ? <Icon name={icon} size={12} /> : undefined}
      summary={[status?.label, ...summary]} />
    {text !== undefined && text !== '' && <EntityCardText icon={icon ?? 'message'} data-resource-detail>{text}</EntityCardText>}
    {preview !== undefined && preview.length > 0 && previewKind === 'diff' && <div data-entity-card-preview="diff" {...stylex.props(styles.diffPreview)}><DiffView patch={preview.join('\n')} path={title} density="compact" maxLines={6} label={`Diff preview, ${title}`} /></div>}
    {preview !== undefined && preview.length > 0 && previewKind !== 'diff' && <pre data-entity-card-preview={previewKind} {...stylex.props(styles.preview)}>{preview.slice(0, 6).map((line, index) => <div key={index}>{line}</div>)}</pre>}
    {children}
    {meta.length > 0 && <EntityCardMeta>{meta.map(line => <EntityCardMetaLine key={line.label} icon={line.icon} label={line.label}>{line.value}</EntityCardMetaLine>)}</EntityCardMeta>}
    {note !== undefined && <EntityCardNote>{note}</EntityCardNote>}
  </EntityCard>
}

const tasteKind: Record<ResourceData['kind'], string> = { file: 'File', diff: 'Diff', pr: 'Pull request', worktree: 'Worktree' }
const tasteIcon: Record<ResourceData['kind'], IconName> = { file: 'attach', diff: 'swap', pr: 'pull-request', worktree: 'folder' }
/** The resource card model the details pane and resource cards hold, as card data. */
export const resourceCardData = ({ id, resource, file = resource.files[0] }: { readonly id: string; readonly resource: ResourceData; readonly file?: ResourceFile }): ResourceEntityCardProps => ({
  id, title: resource.label, kind: tasteKind[resource.kind], icon: tasteIcon[resource.kind],
  summary: file === undefined ? [] : [file.path],
  text: resource.detail,
  ...(file !== undefined && file.lines._tag === 'Known' ? { preview: file.lines.value, previewKind: resource.kind === 'diff' || resource.kind === 'pr' ? 'diff' as const : 'text' as const } : {}),
})

export interface FolderEntityCardProps {
  readonly id: string
  readonly name: string
  readonly host?: string
  readonly description?: string
  /** Agent titles in the folder, in sidebar order. */
  readonly agents?: readonly string[]
  readonly needsYou?: number
}
const folderAgentLimit = 5
export function FolderEntityCard({ id, name, host, description, agents = [], needsYou = 0 }: FolderEntityCardProps) {
  const status: EntityStatus | undefined = needsYou > 0 ? { tone: 'needs-you', label: `${needsYou} need${needsYou === 1 ? 's' : ''} you` } : undefined
  return <EntityCard entity={{ kind: 'folder', id }} testId="folder-hover-card">
    <EntityCardHeader title={name} kind="Folder" glyph={status === undefined ? undefined : <EntityStatusMark status={status} />}
      summary={[status?.label, `${agents.length} agent${agents.length === 1 ? '' : 's'}`, host === undefined ? undefined : <><Icon name="gear" size={12} />{host}</>]} />
    {description !== undefined && <EntityCardText>{description}</EntityCardText>}
    {agents.length > 0 && <EntityCardSection label="Agents" icon="message">
      <EntityCardList>{agents.slice(0, folderAgentLimit).map((agent, index) => <EntityCardListItem key={`${index}:${agent}`} title={agent} />)}</EntityCardList>
      {agents.length > folderAgentLimit && <EntityCardNote>+{agents.length - folderAgentLimit} more</EntityCardNote>}
    </EntityCardSection>}
  </EntityCard>
}

const styles = stylex.create({
  diffPreview: { borderRadius: r.sm, overflow: 'hidden' },
  preview: { margin: s.zero, padding: s.sm, borderRadius: r.sm, backgroundColor: surface.canvas, color: ink.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'pre', overflow: 'hidden' },
})
