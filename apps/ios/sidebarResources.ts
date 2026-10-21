import type { ResourceObservation, ResourcesFilter, ResourcesPage } from '../../clients/typescript/fractal-client';
import type { SidebarResource } from './sidebarView';

export const resourceFacts = (row: ResourceObservation): Record<string, unknown> => row.facts as Record<string, unknown>;
export function resourceRow(row: ResourceObservation): SidebarResource {
  const facts = resourceFacts(row);
  const label = row.kind === 'vcs.repository' ? row.id.replace(/^resource\/github\//, '') : row.id;
  const title = [facts.title, facts.name, facts.repository, label].find((v): v is string => typeof v === 'string' && !!v.trim())!;
  const kind = row.kind === 'vcs.pull-request' ? 'Pull requests' : row.kind === 'vcs.repository' ? 'Repositories' : 'Resources';
  return { id: row.id, kind, target: row.id, title: kind === 'Pull requests' && typeof facts.number === 'number' ? `#${facts.number} ${title}` : title };
}
/** Three bounded windows preserve PRs and repos even when comments dominate the catalog. */
export async function readSidebarResources(list: (filters: ResourcesFilter, options: { limit: number }) => Promise<{ value: ResourcesPage }>, focused?: string): Promise<{ rows: ResourceObservation[]; truncated: boolean }> {
  const rows = new Map<string, ResourceObservation>(); let truncated = false;
  for (const kind of [undefined, 'vcs.pull-request', 'vcs.repository']) {
    const { value } = await list(kind ? { kind } : {}, { limit: 200 });
    truncated ||= value.page.has_more;
    for (const row of value.items) rows.set(row.id, row);
  }
  if (focused) {
    const { value } = await list({ subject_prefix: focused }, { limit: 1 });
    for (const row of value.items) if (row.id === focused) rows.set(row.id, row);
  }
  return { rows: [...rows.values()], truncated };
}
/** Explicit filing can name a graph subject before its observation is in this window. */
export function subjectToFile(query: string, rows: Map<string, SidebarResource>): SidebarResource | undefined {
  const id = query.trim();
  if (!/^[^\s\u0000-\u001f\u007f-\u009f]+\/[^\s\u0000-\u001f\u007f-\u009f]+$/u.test(id) || /^(session|pty)\//.test(id) || rows.has(id)) return;
  return { id, title: id, kind: 'Unavailable', target: id, missing: true };
}
