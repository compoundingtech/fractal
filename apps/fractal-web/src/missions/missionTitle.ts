/** Ad hoc missions are titled by a content hash, as in `person-ask/<hash>`, which says nothing to a reader. */
const machineMissionTitle = /(^|\/)[0-9a-f]{16,}$/

/** A mission's reader-facing title: its own unless machine-generated, then its work's `title ?? goal`. */
export const missionDisplayTitle = ({ title, work }: {
  readonly title: string | undefined
  readonly work: { readonly title?: string | undefined; readonly goal?: string | undefined } | undefined
}): string => {
  if (title !== undefined && !machineMissionTitle.test(title)) return title
  return work?.title || work?.goal || 'Untitled mission'
}

/** A URL-addressed mission: focuses it in the Missions screen and never accumulates in the workspace's tabs. */
const MISSION_PREFIX = 'mission/'
export const isMissionRef = (ref: string): boolean => ref.startsWith(MISSION_PREFIX)
export const missionPaneId = (ref: string): string => `${ref}:detail`
